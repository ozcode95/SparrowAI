//! OpenVINO Model Server lifecycle.
//!
//! The controller owns four runtime concerns:
//!
//! 1. **Install / update** (`install.rs`, `github.rs`) — fetching the
//!    binary distribution from GitHub releases.
//! 2. **Process management** (`process.rs`, `config.rs`, `health.rs`) —
//!    spawning `ovms.exe` against a generated config, draining its log
//!    streams, polling readiness, and shutting it down. A monitor task
//!    watches the child and flips status to `Error` if it dies unexpectedly.
//! 3. **Multi-model serving** — every model the user has loaded lives in
//!    a single `config.json` (one `mediapipe_config_list` entry each), and
//!    add / remove cycles are applied live via the TFS
//!    `POST /v1/config/reload` endpoint (see [`tfs`]) instead of bouncing
//!    the whole OVMS process. The set is persisted in the `served_models`
//!    table so `auto_provision` can re-stage the same lineup on the next
//!    app start without the user having to re-load each model by hand.
//!    On startup, `reconcile_config_to_db` syncs any models present in
//!    `config.json` back into the database, handling manually edited configs
//!    or configs from external sources.
//! 4. **State broadcast** — every status change is mirrored both into the
//!    in-memory `OvmsInfo` and out to the frontend via `ovms://status` so
//!    the UI never has to poll.

pub mod config;
pub mod github;
pub mod health;
pub mod install;
pub mod process;
pub mod tfs;

use crate::db;
use crate::events;
use crate::hf::params::{self as model_params, ModelTask};
use crate::paths;
use crate::settings::Settings;
use anyhow::{anyhow, Context, Result};
use process::{ExitReason, OvmsProcess};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock};

const READY_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OvmsStatus {
    NotInstalled,
    Installing,
    Installed,
    Starting,
    Running,
    Stopping,
    Stopped,
    Error,
}

/// One row of `OvmsInfo.loaded_models`. Carries the per-model bits the
/// UI and chat runner need — full sidecar / runtime state is still read
/// on demand via [`model_params::read_or_default`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedModel {
    pub id: String,
    /// `text_generation` | `embeddings` | `rerank` | `image_generation` |
    /// `text2speech` | `speech2text`. Mirrors
    /// [`crate::hf::ModelTask::kind_str`].
    pub task: String,
    /// `tool_parser` declared in the per-model sidecar, if any. Only
    /// meaningful for text-generation models — `None` everywhere else.
    /// See `OvmsInfo.loaded_model_tool_parser` for the chat-runner
    /// fallback shape.
    pub tool_parser: Option<String>,
    /// `reasoning_parser` declared in the per-model sidecar, if any. Only
    /// meaningful for text-generation models — `None` everywhere else.
    /// Used by the chat runner to determine if `/no_think` should be
    /// automatically appended for models like qwen3/qwen3.5.
    #[serde(default)]
    pub reasoning_parser: Option<String>,
    /// `LM` | `LM_CB` | `VLM` | `VLM_CB` | `AUTO`. Only meaningful for
    /// text-generation models — `None` everywhere else. Surfaced to
    /// the UI for diagnostics; the chat runner no longer gates the
    /// OpenAI `tools` array on this field, since OVMS v2025+ accepts
    /// tool-bearing requests on every LLM graph (including VLM/VLM_CB)
    /// per the chat-completions reference.
    #[serde(default)]
    pub pipeline_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OvmsInfo {
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub status: OvmsStatus,
    pub pid: Option<u32>,
    pub base_url: String,
    /// Every model OVMS is currently serving, insertion-ordered. Empty
    /// when OVMS is stopped or starting with no persisted set.
    #[serde(default)]
    pub loaded_models: Vec<LoadedModel>,
    /// Backward-compat shim: the most recently added text-generation
    /// model id (or the most recently added model of any kind if none
    /// of the loaded models are text-gen). Existing frontend code that
    /// reads `loaded_model` keeps working; new code should prefer
    /// `loaded_models` so it can route by task kind.
    pub loaded_model: Option<String>,
    pub last_error: Option<String>,
    /// `tool_parser` configured on `loaded_model`'s sidecar (e.g.
    /// `"hermes3"`, `"llama3"`). `None` when no model is loaded *or* the
    /// loaded model has no parser configured — the latter is the case for
    /// model families OVMS doesn't ship a parser for (Gemma, Granite,
    /// custom fine-tunes, ...). The chat runner uses this to decide
    /// whether to send the OpenAI-native `tools` array (parser present)
    /// or fall back to prompt-based fenced JSON (parser absent), and the
    /// Server page surfaces it as a warning chip.
    #[serde(default)]
    pub loaded_model_tool_parser: Option<String>,
}

impl Default for OvmsInfo {
    fn default() -> Self {
        // Use the settings-derived default URL here so the field is correct
        // before `hydrate_from_db` has a chance to refresh it.
        Self {
            installed_version: None,
            latest_version: None,
            status: OvmsStatus::NotInstalled,
            pid: None,
            base_url: crate::settings::OvmsSettings::default().base_url(),
            loaded_models: Vec::new(),
            loaded_model: None,
            last_error: None,
            loaded_model_tool_parser: None,
        }
    }
}

pub struct OvmsController {
    app: AppHandle,
    db: SqlitePool,
    http: reqwest::Client,
    info: RwLock<OvmsInfo>,
    /// The live OVMS process, if any. `Mutex` (not `RwLock`) because every
    /// transition needs exclusive access and contention here is negligible.
    proc: Mutex<Option<OvmsProcess>>,
    /// Serialises mutations to the serving set so concurrent
    /// `load_model` / `remove_model` calls can't race on `config.json` or
    /// step on each other's reload responses.
    reload_lock: Mutex<()>,
}

impl OvmsController {
    pub fn new(app: AppHandle, db: SqlitePool, http: reqwest::Client) -> Self {
        Self {
            app,
            db,
            http,
            info: RwLock::new(OvmsInfo::default()),
            proc: Mutex::new(None),
            reload_lock: Mutex::new(()),
        }
    }

    pub async fn info(&self) -> OvmsInfo {
        self.info.read().await.clone()
    }

    /// Called once at startup. Reads the persisted install (if any) and the
    /// saved settings so the UI shows `Installed` + the right `base_url`
    /// immediately without having to re-probe the disk.
    pub async fn hydrate_from_db(&self) -> Result<()> {
        let mut cur = self.info().await;
        // Pull the saved ports so a user-edited REST port is reflected
        // even before they hit `start`.
        if let Ok(s) = Settings::load().await {
            cur.base_url = s.ovms.base_url();
        }
        if let Some(rv) = db::runtimes::get(&self.db, "ovms").await? {
            cur.installed_version = Some(rv.version);
            cur.status = OvmsStatus::Installed;
            cur.last_error = None;
        }
        self.set(cur).await;
        Ok(())
    }

    async fn set(&self, new: OvmsInfo) {
        *self.info.write().await = new.clone();
        let _ = self.app.emit(events::OVMS_STATUS, &new);
    }

    async fn patch<F: FnOnce(&mut OvmsInfo)>(&self, f: F) {
        let mut cur = self.info().await;
        f(&mut cur);
        self.set(cur).await;
    }

    // ─── install / update (unchanged) ───────────────────────────────────

    pub async fn install(&self) -> Result<()> {
        self.patch(|i| {
            i.status = OvmsStatus::Installing;
            i.last_error = None;
        })
        .await;

        match install::install_latest(&self.app, &self.http, &self.db).await {
            Ok(rv) => {
                self.patch(|i| {
                    i.status = OvmsStatus::Installed;
                    i.installed_version = Some(rv.version.clone());
                    i.latest_version = Some(rv.version);
                    i.last_error = None;
                })
                .await;
                Ok(())
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::error!("ovms install failed: {msg}");
                self.patch(|i| {
                    i.status = OvmsStatus::Error;
                    i.last_error = Some(msg.clone());
                })
                .await;
                Err(e)
            }
        }
    }

    pub async fn update(&self) -> Result<()> {
        let release = github::latest_release(&self.http).await?;
        self.patch(|i| i.latest_version = Some(release.tag_name.clone()))
            .await;

        let installed = self.info().await.installed_version.clone();
        if installed.as_deref() == Some(release.tag_name.as_str()) {
            tracing::info!("ovms already at latest version {}", release.tag_name);
            return Ok(());
        }
        self.install().await
    }

    // ─── lifecycle ──────────────────────────────────────────────────────

    /// Add `model_id` to the serving set and make sure OVMS is running.
    ///
    /// - When OVMS is **already running**, rewrites `config.json` with the
    ///   new model added and calls `POST /v1/config/reload` — the existing
    ///   models stay served the entire time.
    /// - When OVMS is **stopped / errored**, spawns `ovms.exe` against the
    ///   full persisted set (this model plus any previously served ones).
    /// - When the model is already in the live set, returns `Ok(())`
    ///   immediately without touching disk.
    ///
    /// The model id is written to `served_models` regardless of which
    /// branch is taken, so the next `auto_provision` restores it.
    ///
    /// Takes `self: &Arc<Self>` so the crash monitor can hold its own clone
    /// without us needing a global weak-reference dance.
    pub async fn start(self: &Arc<Self>, model_id: &str) -> Result<()> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(anyhow!("start: empty model id"));
        }

        let _reload_guard = self.reload_lock.lock().await;

        // Persist the membership change up front so a crash between here
        // and the reload still leaves the model in the desired set —
        // auto_provision on the next start will replay it.
        db::served_models::add(&self.db, model_id).await?;

        // Enforce one-model-per-task-family in the serving set: if
        // there's already a model of the same task kind loaded (e.g.
        // another text-generation / image-text-to-text model when the
        // user just clicked Load on a new chat model), evict the old
        // one before the reload. The chat UI only routes to a single
        // model per family at a time, so keeping two loaded just
        // duplicates GPU memory pressure. `ModelTask::kind_str()`
        // already collapses `text-generation` and `image-text-to-text`
        // into the same `text_generation` discriminator, so the
        // "VLM and text LM are the same family" rule the user asked
        // for falls out of the comparison naturally.
        if let Err(e) = self.evict_same_family(model_id).await {
            tracing::warn!(
                "ovms: same-family eviction for `{model_id}` failed: {e:#} — proceeding with reload anyway"
            );
        }

        let cur = self.info().await;
        let already_loaded = cur.loaded_models.iter().any(|m| m.id == model_id);
        let is_running = matches!(cur.status, OvmsStatus::Running);

        if already_loaded && is_running {
            return Ok(());
        }

        if is_running {
            // Hot path: live reload via TFS — no process restart.
            match self.reload_with_set(self.persisted_set().await?).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Roll the desired set back if reload truly failed
                    // (e.g. model dir missing) — leaving the model in
                    // served_models when OVMS rejected it would have
                    // auto_provision re-fail every launch.
                    let _ = db::served_models::remove(&self.db, model_id).await;
                    let msg = format!("{e:#}");
                    tracing::error!("ovms live reload failed: {msg}");
                    self.patch(|i| i.last_error = Some(msg.clone())).await;
                    Err(e)
                }
            }
        } else {
            // Cold path: spawn OVMS with the full desired set.
            let desired = self.persisted_set().await?;
            match self.spawn_with_set(&desired).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Make sure we don't leak a half-spawned child on failure.
                    if let Some(p) = self.proc.lock().await.take() {
                        let _ = p.shutdown().await;
                    }
                    let msg = format!("{e:#}");
                    tracing::error!("ovms start failed: {msg}");
                    self.patch(|i| {
                        i.status = OvmsStatus::Error;
                        i.pid = None;
                        i.loaded_models = Vec::new();
                        i.loaded_model = None;
                        i.loaded_model_tool_parser = None;
                        i.last_error = Some(msg.clone());
                    })
                    .await;
                    Err(e)
                }
            }
        }
    }

    /// Remove any other model in the persisted serving set whose
    /// sidecar task kind matches the one we're about to load. Driven
    /// from [`OvmsController::start`] so that loading e.g. a new
    /// text-generation model transparently unloads the previous
    /// text-generation model rather than stacking another graph on
    /// top of it.
    ///
    /// Skips the row when the sidecar can't be read (we'd rather
    /// leave it in than evict something we don't understand) and is a
    /// no-op for the very first model in a family.
    async fn evict_same_family(&self, new_model_id: &str) -> Result<()> {
        let Some(new_dir) = lookup_model_dir(&self.db, new_model_id).await? else {
            return Ok(());
        };
        let new_kind = model_params::read_or_default(&new_dir).task.kind_str();

        for id in db::served_models::list(&self.db).await? {
            if id == new_model_id {
                continue;
            }
            let Some(dir) = lookup_model_dir(&self.db, &id).await? else {
                continue;
            };
            let kind = model_params::read_or_default(&dir).task.kind_str();
            if kind == new_kind {
                tracing::info!(
                    "ovms: evicting `{id}` (task={kind}) in favour of new same-family `{new_model_id}`"
                );
                db::served_models::remove(&self.db, &id).await?;
            }
        }
        Ok(())
    }

    /// Resolve every persisted served model into `(id, dir)` pairs,
    /// skipping ids that are no longer downloaded on disk (with a warning
    /// in the log). Used by both the cold-start spawn path and the
    /// in-process reload path.
    async fn persisted_set(&self) -> Result<Vec<(String, PathBuf)>> {
        let ids = db::served_models::list(&self.db).await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            match lookup_model_dir(&self.db, &id).await? {
                Some(dir) if dir.is_dir() => out.push((id, dir)),
                Some(dir) => {
                    tracing::warn!(
                        "served model `{id}` is registered but its directory is gone ({}); skipping",
                        dir.display()
                    );
                }
                None => {
                    tracing::warn!(
                        "served model `{id}` is registered but not in `local_models`; skipping"
                    );
                }
            }
        }
        Ok(out)
    }

    /// Cold-start path. Writes `config.json` with `set`, spawns
    /// `ovms.exe`, waits for the OpenAI-compat `/models` endpoint, then
    /// flips status to `Running`.
    async fn spawn_with_set(self: &Arc<Self>, set: &[(String, PathBuf)]) -> Result<()> {
        if set.is_empty() {
            // Nothing to serve. The controller treats this as a clean
            // "stopped" — callers should add a model first.
            return Err(anyhow!(
                "ovms has no models to serve — load one from the Models page first"
            ));
        }

        let runtime = db::runtimes::get(&self.db, "ovms")
            .await?
            .ok_or_else(|| anyhow!("ovms is not installed yet"))?;
        let executable = PathBuf::from(&runtime.executable);
        if !executable.is_file() {
            return Err(anyhow!(
                "ovms executable missing on disk: {} (re-install required)",
                executable.display()
            ));
        }

        let settings = Settings::load().await.unwrap_or_default();
        let ovms_cfg = settings.ovms;
        let base_url = ovms_cfg.base_url();

        let loaded = render_config(set, &ovms_cfg.device)?;

        self.patch(|i| {
            i.status = OvmsStatus::Starting;
            i.loaded_models = loaded.clone();
            i.loaded_model = primary_text_model(&loaded);
            i.loaded_model_tool_parser = primary_tool_parser(&loaded);
            i.base_url = base_url.clone();
            i.last_error = None;
        })
        .await;

        // `--file_system_poll_wait_seconds 0` disables OVMS's own
        // file-watcher reload loop. We drive reloads via the TFS
        // `/v1/config/reload` endpoint so changes land deterministically;
        // letting OVMS *also* poll would race with our explicit reload
        // and double-load on every config rewrite.
        //
        // `--cache_dir` points at a persistent OpenVINO compiled-model
        // cache (see
        // <https://docs.openvino.ai/2026/model-server/ovms_docs_model_cache.html>).
        // First start of a given (model, device, OpenVINO version) tuple
        // pays the full compile cost; later starts reuse the blob and
        // get to `Running` in a fraction of the time. We only inject the
        // default when the user hasn't already specified one (either via
        // the typed `OvmsSettings.cache_dir` field, which `cli_args()`
        // already pushed, or through `extra_args`).
        let args = {
            let mut a = ovms_cfg.cli_args();
            a.push("--config_path".into());
            a.push(state_config_path()?.to_string_lossy().into_owned());
            if !a.iter().any(|s| s == "--file_system_poll_wait_seconds") {
                a.push("--file_system_poll_wait_seconds".into());
                a.push("0".into());
            }
            if !a.iter().any(|s| s == "--cache_dir") {
                let cache_dir = paths::ovms_cache_dir()?;
                a.push("--cache_dir".into());
                a.push(cache_dir.to_string_lossy().into_owned());
            }
            a
        };
        let working_dir = paths::ovms_state_dir()?;

        // The crash monitor closure runs after the child exits (kill or
        // crash). It owns its own `Arc<Self>` so the controller's status can
        // be updated even if the original caller's future has been dropped.
        let watcher = Arc::clone(self);
        let on_exit = move |reason: ExitReason| {
            tokio::spawn(async move {
                watcher.handle_child_exit(reason).await;
            });
        };

        let proc = OvmsProcess::spawn(&self.app, &executable, &args, &working_dir, on_exit).await?;
        let pid = proc.pid();
        *self.proc.lock().await = Some(proc);

        self.patch(|i| i.pid = Some(pid)).await;

        // Wait for the HTTP endpoint to come up. If readiness fails,
        // try to pull the actual OVMS error out of the log buffer so the
        // user sees the real failure (e.g. PagedAttention init) instead
        // of just "did not become ready".
        if let Err(e) = health::wait_ready(&self.http, &base_url, READY_TIMEOUT).await {
            let excerpt = match self.proc.lock().await.as_ref() {
                Some(p) => p.error_excerpt().await,
                None => None,
            };
            let msg = match excerpt {
                Some(detail) => format!("waiting for ovms readiness: {e:#} — {detail}"),
                None => format!("waiting for ovms readiness: {e:#}"),
            };
            return Err(anyhow!(msg));
        }

        // HTTP is up, but the MediaPipe graphs for each model still need
        // to compile + load weights before chat requests will succeed —
        // until then OVMS replies `404 Mediapipe graph definition with
        // requested name is not found`. Block flipping to `Running`
        // until TFS reports every served model as `AVAILABLE` so the
        // composer doesn't unlock too early.
        let expected_names: Vec<String> = set.iter().map(|(id, _)| id.clone()).collect();
        if let Err(e) =
            health::wait_models_available(&self.http, &base_url, &expected_names, READY_TIMEOUT)
                .await
        {
            let excerpt = match self.proc.lock().await.as_ref() {
                Some(p) => p.error_excerpt().await,
                None => None,
            };
            let msg = match excerpt {
                Some(detail) => format!("waiting for ovms model load: {e:#} — {detail}"),
                None => format!("waiting for ovms model load: {e:#}"),
            };
            return Err(anyhow!(msg));
        }

        self.patch(|i| {
            i.status = OvmsStatus::Running;
            i.last_error = None;
        })
        .await;
        Ok(())
    }

    /// Hot path. Re-renders `config.json` for `set` and asks OVMS to
    /// pick the changes up in-place via `POST /v1/config/reload`. The
    /// `info` snapshot is updated only after OVMS confirms every model
    /// is `AVAILABLE`.
    async fn reload_with_set(&self, set: Vec<(String, PathBuf)>) -> Result<()> {
        if set.is_empty() {
            // Caller asked for an empty serving set on a running OVMS —
            // delegate to `stop` so the process exits cleanly instead of
            // sitting idle.
            self.stop_inner(false).await?;
            return Ok(());
        }

        let settings = Settings::load().await.unwrap_or_default();
        let loaded = render_config(&set, &settings.ovms.device)?;
        let base_url = self.info().await.base_url;

        let status = tfs::reload_config(&self.http, &base_url)
            .await
            .context("POST /v1/config/reload")?;
        // Validate readiness against the desired set only. OVMS echoes
        // back every model name it has ever known in the reload
        // response — including ones we just removed, which arrive in
        // `END` state with `error_code=OK` to confirm the unload
        // succeeded. Treating those as failures (which the unscoped
        // version of this check used to do) made every Unload action
        // look broken even though the reload itself worked exactly as
        // intended. See `tfs::all_available` for the filtering rule.
        let expected: Vec<&str> = set.iter().map(|(id, _)| id.as_str()).collect();
        if !tfs::all_available(&status, expected.iter().copied()) {
            let detail = tfs::unavailable_summary(&status, expected.iter().copied())
                .unwrap_or_else(|| "unknown".into());
            return Err(anyhow!(
                "ovms reload reported non-AVAILABLE models: {detail}"
            ));
        }

        self.patch(|i| {
            i.status = OvmsStatus::Running;
            i.loaded_models = loaded.clone();
            i.loaded_model = primary_text_model(&loaded);
            i.loaded_model_tool_parser = primary_tool_parser(&loaded);
            i.last_error = None;
        })
        .await;
        Ok(())
    }

    /// Remove `model_id` from the persistent serving set and apply the
    /// change live. If the set becomes empty, OVMS is stopped (so we
    /// don't keep an idle process around) but the *previous* set is
    /// already gone — callers wanting a clean "everything off, but
    /// remember it for next launch" semantic should use [`stop`] instead.
    pub async fn remove_model(self: &Arc<Self>, model_id: &str) -> Result<()> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(anyhow!("remove_model: empty model id"));
        }

        let _reload_guard = self.reload_lock.lock().await;
        db::served_models::remove(&self.db, model_id).await?;

        let is_running = matches!(self.info().await.status, OvmsStatus::Running);
        if !is_running {
            // Nothing live to reconcile; the persisted change is enough.
            return Ok(());
        }

        let desired = self.persisted_set().await?;
        self.reload_with_set(desired).await
    }

    /// Tear down the OVMS process without touching the persisted serving
    /// set — restarting zero (or hitting `start` again) brings the same
    /// models back online. Use [`remove_model`] to forget a model.
    pub async fn stop(&self) -> Result<()> {
        self.stop_inner(true).await
    }

    /// Inner shutdown. `from_user` distinguishes the explicit `stop()`
    /// call (we want to land in `Stopped`) from the internal "set went
    /// empty" path inside `reload_with_set` (same outcome, but logged
    /// differently in case anyone greps for it later).
    async fn stop_inner(&self, from_user: bool) -> Result<()> {
        let proc = self.proc.lock().await.take();
        if proc.is_none() {
            // Still normalise the status — the caller may have hit stop on an
            // orphaned `Starting` row, or just want to reset after an error.
            self.patch(|i| {
                if !matches!(i.status, OvmsStatus::NotInstalled) {
                    i.status = OvmsStatus::Stopped;
                }
                i.pid = None;
                i.loaded_models = Vec::new();
                i.loaded_model = None;
                i.loaded_model_tool_parser = None;
            })
            .await;
            return Ok(());
        }

        if !from_user {
            tracing::info!("ovms stopping: serving set became empty");
        }

        self.patch(|i| i.status = OvmsStatus::Stopping).await;
        if let Some(p) = proc {
            let _ = p.shutdown().await;
        }
        self.patch(|i| {
            i.status = OvmsStatus::Stopped;
            i.pid = None;
            i.loaded_models = Vec::new();
            i.loaded_model = None;
            i.loaded_model_tool_parser = None;
            i.last_error = None;
        })
        .await;
        Ok(())
    }

    /// Alias kept for the existing `ovms_load_model` IPC contract.
    pub async fn load_model(self: &Arc<Self>, model_id: &str) -> Result<()> {
        self.start(model_id).await
    }

    /// Alias for [`stop`], kept for the existing `ovms_unload_model` IPC
    /// contract. The frontend "Unload" button has always meant "stop the
    /// server" rather than "drop this specific model"; renaming would
    /// silently change behaviour for upgraders.
    pub async fn unload_model(&self) -> Result<()> {
        self.stop().await
    }

    // ─── TFS inspection passthroughs ────────────────────────────────────

    /// `GET /v1/config` — aggregated TFS status of every loaded model.
    /// Used by the Settings page's model inspector to surface OVMS's own
    /// view of model readiness alongside the cached `loaded_models` list.
    /// Returns an error when OVMS isn't running so the UI can render an
    /// empty-state instead of a stale snapshot.
    pub async fn config_status(&self) -> Result<tfs::ConfigStatus> {
        let info = self.info().await;
        if !matches!(info.status, OvmsStatus::Running) {
            return Err(anyhow!(
                "ovms is not running (status: {:?}); start it before inspecting models",
                info.status
            ));
        }
        tfs::config_status(&self.http, &info.base_url)
            .await
            .context("GET /v1/config")
    }

    /// `GET /v1/models/<id>` — TFS-style status block for a single
    /// model. Returned as the typed [`tfs::ModelStatus`] so the bottom-
    /// bar status popover can render the per-version state and any
    /// error code without re-implementing the wire shape on the JS side.
    ///
    /// Implementation note: OVMS serves text-generation / embeddings /
    /// rerank workloads as MediaPipe pipelines (entries in
    /// `mediapipe_config_list`, not `model_config_list`) and a few OVMS
    /// builds return `404` from `GET /v1/models/<name>` when the name
    /// contains a `/` (HF ids always do). When the per-model endpoint
    /// fails we fall back to filtering the aggregated `GET /v1/config`
    /// response — same wire shape, keyed by name — so the popover
    /// renders real data instead of an opaque error.
    pub async fn model_status(&self, model_id: &str) -> Result<tfs::ModelStatus> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(anyhow!("model_status: empty model id"));
        }
        let info = self.info().await;
        if !matches!(info.status, OvmsStatus::Running) {
            return Err(anyhow!(
                "ovms is not running (status: {:?}); start it before inspecting models",
                info.status
            ));
        }
        match tfs::model_status(&self.http, &info.base_url, model_id).await {
            Ok(s) => Ok(s),
            Err(direct_err) => {
                tracing::debug!(
                    "model_status `{model_id}` direct endpoint failed: {direct_err:#}; \
                     falling back to /v1/config"
                );
                match tfs::config_status(&self.http, &info.base_url).await {
                    Ok(map) => match map.get(model_id) {
                        Some(s) => Ok(s.clone()),
                        None => Err(direct_err.context(format!(
                            "model `{model_id}` not present in /v1/config either"
                        ))),
                    },
                    Err(fallback_err) => Err(direct_err
                        .context(format!("fallback /v1/config also failed: {fallback_err:#}"))),
                }
            }
        }
    }

    /// `GET /v1/models/<id>/metadata` — input/output tensor signature
    /// for one loaded model. Returned as raw JSON because the OVMS
    /// response shape varies by served task and we want the inspector
    /// renderer to be the single source of truth for what to show.
    pub async fn model_metadata(&self, model_id: &str) -> Result<serde_json::Value> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(anyhow!("model_metadata: empty model id"));
        }
        let info = self.info().await;
        if !matches!(info.status, OvmsStatus::Running) {
            return Err(anyhow!(
                "ovms is not running (status: {:?}); start it before inspecting models",
                info.status
            ));
        }
        tfs::model_metadata(&self.http, &info.base_url, model_id)
            .await
            .context("Failed to fetch model metadata")
    }

    /// Best-effort startup provisioning. Triggered from `AppState::init`
    /// so a fresh install of zero becomes usable without the user having to
    /// drive the Server page by hand.
    ///
    /// Sequence:
    /// 1. If `settings.auto_provision_ovms` is false, do nothing.
    /// 2. If the binary is not installed yet, install the latest release.
    /// 3. Load the persisted `served_models` set. If empty, fall back to
    ///    `settings.default_model` (for backwards compatibility with
    ///    installs that pre-date the persisted set).
    /// 4. Start OVMS with whichever set we ended up with.
    ///
    /// All errors are logged and swallowed — auto-provision must never
    /// crash startup, and the UI surfaces the resulting status via the
    /// existing `ovms://status` event.
    pub async fn auto_provision(self: &Arc<Self>) {
        let settings = match Settings::load().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("auto_provision: settings load failed: {e:#}");
                return;
            }
        };
        if !settings.auto_provision_ovms {
            tracing::debug!("auto_provision: disabled in settings, skipping");
            return;
        }

        // ─── 1. Install if needed ──────────────────────────────────────
        let status = self.info().await.status;
        match status {
            OvmsStatus::NotInstalled => {
                tracing::info!("auto_provision: ovms not installed, fetching latest release");
                if let Err(e) = self.install().await {
                    tracing::warn!("auto_provision: install failed: {e:#}");
                    return;
                }
            }
            // Already installing (somehow), already running, or already in
            // an error state we shouldn't overwrite — nothing to do.
            OvmsStatus::Installing
            | OvmsStatus::Starting
            | OvmsStatus::Running
            | OvmsStatus::Stopping => return,
            OvmsStatus::Installed | OvmsStatus::Stopped | OvmsStatus::Error => {}
        }

        // ─── 1.5. Reconcile config.json with database ──────────────────
        // If config.json exists and has models that aren't in the database,
        // add them. This handles manually edited configs or configs from
        // external sources.
        if let Err(e) = self.reconcile_config_to_db().await {
            tracing::warn!("auto_provision: config reconciliation failed: {e:#}");
        }

        // ─── 2. Resolve the desired serving set ────────────────────────
        // Priority is: persisted set → settings.default_model. The
        // legacy single-model field is only consulted when the persisted
        // set is empty, so we don't override an explicit user choice.
        let mut desired_ids = match db::served_models::list(&self.db).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("auto_provision: served_models lookup failed: {e:#}");
                Vec::new()
            }
        };
        if desired_ids.is_empty() {
            if let Some(id) = settings.default_model.as_deref().map(str::trim) {
                if !id.is_empty() {
                    desired_ids.push(id.to_string());
                    // Persist so future launches don't fall through this
                    // legacy branch again — the user is "really" using
                    // the new persisted set, they just haven't loaded
                    // anything through the new path yet.
                    if let Err(e) = db::served_models::add(&self.db, id).await {
                        tracing::warn!(
                            "auto_provision: failed to seed served_models from default_model: {e:#}"
                        );
                    }
                }
            }
        }
        if desired_ids.is_empty() {
            tracing::debug!("auto_provision: no served models configured, leaving ovms stopped");
            return;
        }

        // ─── 3. Resolve which ones are actually downloaded ─────────────
        let mut available: Vec<(String, PathBuf)> = Vec::new();
        for id in &desired_ids {
            match lookup_model_dir(&self.db, id).await {
                Ok(Some(dir)) if dir.is_dir() => available.push((id.clone(), dir)),
                Ok(Some(dir)) => {
                    tracing::warn!(
                        "auto_provision: model `{id}` registered but directory missing ({}); skipping",
                        dir.display()
                    );
                }
                Ok(None) => {
                    tracing::info!("auto_provision: model `{id}` not downloaded yet — skipping");
                }
                Err(e) => {
                    tracing::warn!("auto_provision: model lookup for `{id}` failed: {e:#}");
                }
            }
        }
        if available.is_empty() {
            tracing::info!("auto_provision: no served models are downloaded — skipping start");
            return;
        }

        tracing::info!(
            "auto_provision: starting ovms with {} model(s): {}",
            available.len(),
            available
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        if let Err(e) = self.spawn_with_set(&available).await {
            // `spawn_with_set` already pushed the error into `info` —
            // just log here and keep startup non-fatal.
            tracing::warn!("auto_provision: start failed: {e:#}");
        }
    }

    /// Reconcile `config.json` with the `served_models` database table.
    ///
    /// Reads the existing `config.json` file (if it exists) and ensures any
    /// models listed in `mediapipe_config_list` are also present in the
    /// `served_models` table. This handles manually edited configs or
    /// configs from external sources.
    ///
    /// This is best-effort — errors are returned but should be logged and
    /// ignored by the caller to avoid blocking startup.
    async fn reconcile_config_to_db(&self) -> Result<()> {
        let config_path = paths::ovms_state_dir()?.join("config.json");

        // If config.json doesn't exist yet, nothing to reconcile
        if !config_path.exists() {
            return Ok(());
        }

        // Read and parse config.json
        let config_content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("read {}", config_path.display()))?;

        let config: serde_json::Value = serde_json::from_str(&config_content)
            .with_context(|| format!("parse {}", config_path.display()))?;

        // Extract model names from mediapipe_config_list
        let Some(mediapipe_list) = config["mediapipe_config_list"].as_array() else {
            return Ok(()); // No mediapipe_config_list or not an array
        };

        let mut reconciled_count = 0;
        for entry in mediapipe_list {
            if let Some(model_name) = entry["name"].as_str() {
                let model_name = model_name.trim();
                if model_name.is_empty() {
                    continue;
                }

                // Add to database (INSERT OR IGNORE means it's safe to call
                // even if already present)
                if let Err(e) = db::served_models::add(&self.db, model_name).await {
                    tracing::warn!(
                        "reconcile_config_to_db: failed to add '{}' to database: {e:#}",
                        model_name
                    );
                } else {
                    tracing::debug!(
                        "reconcile_config_to_db: ensured '{}' is in served_models table",
                        model_name
                    );
                    reconciled_count += 1;
                }
            }
        }

        if reconciled_count > 0 {
            tracing::info!(
                "reconcile_config_to_db: reconciled {} model(s) from config.json to database",
                reconciled_count
            );
        }

        Ok(())
    }

    /// Called from the process monitor when the child exits. We only react to
    /// *unexpected* deaths — expected ones (`stop`, `start` restart, drop)
    /// are already reflected in the status flow.
    async fn handle_child_exit(&self, reason: ExitReason) {
        let code = match reason {
            ExitReason::Expected => return,
            ExitReason::Crashed { code } => code,
        };

        // If the controller has already moved on (a deliberate stop landed
        // while we were unwinding) don't clobber its status.
        let cur = self.info().await;
        if !matches!(cur.status, OvmsStatus::Running | OvmsStatus::Starting) {
            return;
        }

        let msg = match code {
            Some(c) => format!("ovms exited unexpectedly (code {c})"),
            None => "ovms exited unexpectedly (terminated by signal)".to_string(),
        };
        tracing::error!("{msg}");

        // Pull whatever the log pump captured before the child exited so
        // we can attach a useful hint to the persisted error — otherwise
        // a crash on model load shows up as just an exit code.
        let proc_guard = self.proc.lock().await.take();
        let excerpt = if let Some(p) = &proc_guard {
            p.error_excerpt().await
        } else {
            None
        };
        drop(proc_guard);
        let combined = match excerpt {
            Some(detail) => format!("{msg} — {detail}"),
            None => msg.clone(),
        };

        self.patch(|i| {
            i.status = OvmsStatus::Error;
            i.pid = None;
            i.loaded_models = Vec::new();
            i.loaded_model = None;
            i.loaded_model_tool_parser = None;
            i.last_error = Some(combined);
        })
        .await;
    }
}

// ─── helpers ───────────────────────────────────────────────────────────────

/// Resolve `<id_or_hf_id>` → on-disk model directory via the `local_models`
/// table. Returns `Ok(None)` when the model hasn't been downloaded yet.
async fn lookup_model_dir(db: &SqlitePool, model_id: &str) -> Result<Option<PathBuf>> {
    let row = sqlx::query("SELECT path FROM local_models WHERE id = ? OR hf_id = ?")
        .bind(model_id)
        .bind(model_id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| PathBuf::from(r.get::<String, _>("path"))))
}

/// Canonical path of the OVMS config we own. Centralised so the spawn
/// and reload paths can't drift.
fn state_config_path() -> Result<PathBuf> {
    Ok(paths::ovms_state_dir()?.join("config.json"))
}

/// Render `config.json` + per-model `graph.pbtxt` for `set` and return
/// the `LoadedModel` rows the controller should publish on its
/// `OvmsInfo` snapshot.
fn render_config(set: &[(String, PathBuf)], runtime_device: &str) -> Result<Vec<LoadedModel>> {
    let inputs: Vec<config::PrepareInput<'_>> = set
        .iter()
        .map(|(id, dir)| config::PrepareInput {
            model_name: id.as_str(),
            model_dir: dir.as_path(),
            device: runtime_device,
        })
        .collect();

    config::prepare(&inputs).context("prepare ovms config")?;

    let mut out = Vec::with_capacity(set.len());
    for (id, dir) in set {
        let params = model_params::read_or_default(dir);
        let (task, tool_parser, reasoning_parser, pipeline_type) = match params.task {
            ModelTask::TextGeneration(tg) => (
                "text_generation",
                tg.tool_parser,
                tg.reasoning_parser,
                tg.pipeline_type,
            ),
            ModelTask::Embeddings(_) => ("embeddings", None, None, None),
            ModelTask::Rerank(_) => ("rerank", None, None, None),
            ModelTask::ImageGeneration(_) => ("image_generation", None, None, None),
            ModelTask::Text2Speech(_) => ("text2speech", None, None, None),
            ModelTask::Speech2Text(_) => ("speech2text", None, None, None),
        };
        out.push(LoadedModel {
            id: id.clone(),
            task: task.to_string(),
            tool_parser,
            reasoning_parser,
            pipeline_type,
        });
    }
    Ok(out)
}

/// Pick the "primary" text-generation model to expose through the
/// backwards-compat `loaded_model` field. The most recently added
/// text-gen model wins; if none of the loaded models are text-gen, the
/// most recently added model of any kind is used so the field is never
/// stale when *something* is serving.
fn primary_text_model(loaded: &[LoadedModel]) -> Option<String> {
    loaded
        .iter()
        .rev()
        .find(|m| m.task == "text_generation")
        .or_else(|| loaded.last())
        .map(|m| m.id.clone())
}

/// Tool parser of the primary text-generation model, if any. Returns
/// `None` when there's no text-gen model loaded, or when the loaded one
/// has no parser configured in its sidecar.
fn primary_tool_parser(loaded: &[LoadedModel]) -> Option<String> {
    loaded
        .iter()
        .rev()
        .find(|m| m.task == "text_generation")
        .and_then(|m| m.tool_parser.clone())
}
