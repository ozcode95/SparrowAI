//! Shared application state.
//!
//! Wraps the SQLite pool, the HTTP client, the OVMS process handle, the
//! per-chat cancellation registry, and any cached system specs. Stored as a
//! Tauri-managed singleton so commands can reach it via
//! `tauri::State<AppState>` (or the `AppStateExt` shortcut below).

use crate::chat::runner::{ChatJobs, ToolConfirms};
use crate::db;
use crate::hf::DownloadJobs;
use crate::mcp::catalog::McpToolsCache;
use crate::ovms::OvmsController;
use crate::paths;
use crate::system::Specs;
use anyhow::Result;
use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::RwLock;

pub struct AppState {
    pub db: SqlitePool,
    pub http: reqwest::Client,
    /// `Arc` so spawned background tasks (chat runner, ovms install) can hold
    /// their own handle without borrowing through `AppState`.
    pub ovms: Arc<OvmsController>,
    pub chat_jobs: Arc<ChatJobs>,
    /// Pending destructive-tool confirm prompts — see
    /// [`ToolConfirms`] for the flow.
    pub tool_confirms: Arc<ToolConfirms>,
    /// Same pattern as `chat_jobs` — a registry of in-flight HF model
    /// downloads, used both as a per-model concurrency guard and as the
    /// cancellation channel for `models_cancel`.
    pub downloads: Arc<DownloadJobs>,
    /// Cached `tools/list` results, keyed by MCP server id. Lets the chat
    /// runner inject the tool catalog into the system prompt without
    /// re-probing every server on every turn.
    pub mcp_cache: Arc<McpToolsCache>,
    pub specs: RwLock<Option<Specs>>,
}

impl AppState {
    /// Initialise once at startup. Stores the result in the Tauri app handle.
    pub async fn init(app: AppHandle) -> Result<()> {
        // Best-effort one-time migration: if the user has data under the
        // pre-`.zero` layout (`%LOCALAPPDATA%\zero\zero\data\` and
        // friends), move it into `~/.zero` before any code touches the
        // new paths. Failures here are non-fatal — we still come up,
        // just with an empty profile rather than the user's old data.
        if let Err(e) = paths::migrate_legacy_root_if_needed() {
            tracing::warn!("legacy storage migration failed: {e:#}");
        }

        let db = db::open_pool(&paths::db_file()?).await?;
        db::migrate(&db).await?;

        // Best-effort: backfill `verified_files` for rows installed before
        // we tracked it. Failures here are non-fatal — we just lose the
        // verified badge in the UI for legacy entries.
        if let Err(e) = crate::hf::backfill_verified(&db).await {
            tracing::warn!("hf backfill_verified failed: {e:#}");
        }

        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("zero/", env!("CARGO_PKG_VERSION")));

        // Load extra CA(s) — corporate MITM root.
        // Path comes from env var so we don't bake site-specific config into the binary.
        if let Ok(path) = std::env::var("ZERO_EXTRA_CA_BUNDLE")
            .or_else(|_| std::env::var("SSL_CERT_FILE"))
        {
            match std::fs::read(&path) {
                Ok(pem) => {
                    // A bundle file may contain multiple PEM blocks; add them all.
                    for cert in reqwest::Certificate::from_pem_bundle(&pem)
                        .unwrap_or_default()
                    {
                        builder = builder.add_root_certificate(cert);
                    }
                    tracing::info!("loaded extra CA bundle from {path}");
                }
                Err(e) => tracing::warn!("failed reading CA bundle {path}: {e}"),
            }
        }

        let http = builder.build()?;

        let ovms = Arc::new(OvmsController::new(app.clone(), db.clone(), http.clone()));
        if let Err(e) = ovms.hydrate_from_db().await {
            tracing::warn!("ovms hydrate from db failed: {e:#}");
        }

        let state = Arc::new(AppState {
            db,
            http,
            ovms,
            chat_jobs: Arc::new(ChatJobs::new()),
            tool_confirms: Arc::new(ToolConfirms::new()),
            downloads: DownloadJobs::new(),
            mcp_cache: McpToolsCache::new(),
            specs: RwLock::new(None),
        });

        app.manage(state);
        tracing::info!("app state initialised");

        // Kick off OVMS auto-provisioning in the background so the UI can
        // render immediately. The controller emits `ovms://status` and
        // `ovms://install-progress` events as it works, so the Server page
        // stays in sync without us having to await here.
        let ovms_provision = Arc::clone(&app.zero().ovms);
        tauri::async_runtime::spawn(async move {
            ovms_provision.auto_provision().await;
        });

        // Start the task ticker. It scans the `tasks` table every
        // ~30 s and fires anything whose trigger is due. Lives for
        // the lifetime of the app handle; nothing here needs to await.
        crate::tasks::scheduler::start(app.clone());

        Ok(())
    }
}

/// Convenience extension: pull the managed `Arc<AppState>` out of a handle.
pub trait AppStateExt {
    fn zero(&self) -> Arc<AppState>;
}

impl<R: tauri::Runtime> AppStateExt for tauri::AppHandle<R> {
    fn zero(&self) -> Arc<AppState> {
        self.state::<Arc<AppState>>().inner().clone()
    }
}
