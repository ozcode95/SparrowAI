//! SQLite pool + schema.
//!
//! We avoid sqlx's compile-time macros (`query!`, `migrate!`) so the schema
//! can evolve without forcing a `DATABASE_URL` at build time. Migrations are
//! tracked in-process via a `schema_migrations` ledger, applied at startup,
//! and replayed only the first time we see each one — so dbs created by
//! earlier ad-hoc migrations are picked up cleanly on upgrade.

pub mod runtimes;
pub mod served_models;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

pub async fn open_pool(file: &Path) -> Result<SqlitePool> {
    let url = format!("sqlite://{}", file.display());
    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    Ok(pool)
}

/// Apply the baseline schema, then any pending entries in [`MIGRATIONS`].
///
/// The baseline is itself idempotent (everything is `CREATE TABLE IF NOT
/// EXISTS`) so it can be re-applied freely on every boot. Migrations beyond
/// the baseline are recorded in `schema_migrations` so they only run once.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            name        TEXT PRIMARY KEY,
            applied_at  TEXT NOT NULL
         )",
    )
    .execute(pool)
    .await?;

    sqlx::query(BASELINE)
        .execute(pool)
        .await
        .context("apply baseline schema")?;

    for m in MIGRATIONS {
        if migration_applied(pool, m.name).await? {
            continue;
        }
        tracing::info!("applying db migration: {}", m.name);
        m.run(pool)
            .await
            .with_context(|| format!("migration {} failed", m.name))?;
        sqlx::query("INSERT INTO schema_migrations (name, applied_at) VALUES (?, ?)")
            .bind(m.name)
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn migration_applied(pool: &SqlitePool, name: &str) -> Result<bool> {
    let row = sqlx::query("SELECT 1 AS _present FROM schema_migrations WHERE name = ?")
        .bind(name)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

/// A single additive migration. Each statement is run in order; statements
/// that fail with an "idempotent" error (e.g. ADD COLUMN where the column
/// already exists from the old ad-hoc migration path) are silently treated
/// as success so existing user dbs keep working.
struct Migration {
    name: &'static str,
    statements: &'static [&'static str],
}

impl Migration {
    async fn run(&self, pool: &SqlitePool) -> Result<()> {
        for stmt in self.statements {
            match sqlx::query(stmt).execute(pool).await {
                Ok(_) => {}
                Err(e) => {
                    let msg = e.to_string();
                    if is_idempotent_error(&msg) {
                        tracing::debug!(
                            "migration {}: ignoring idempotent error: {msg}",
                            self.name
                        );
                    } else {
                        return Err(anyhow!(e));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Detect SQLite errors we want to treat as no-ops — typically "column already
/// exists" from a partial earlier migration. Add to this list with care; it's
/// here to make migrations safe to re-run, not to swallow real bugs.
fn is_idempotent_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("duplicate column") || lower.contains("already exists")
}

/// Ordered list of migrations the runtime knows about. Append-only: never
/// renumber or rewrite history once a release has shipped. Old entries can be
/// dropped only when we're sure no live db is older than the next entry.
const MIGRATIONS: &[Migration] = &[
    // The baseline schema used to lack these columns; they were added later
    // by an ad-hoc ALTER block. Keep the migration entry so fresh dbs (which
    // already get the columns from BASELINE) record the same history as
    // upgraded ones.
    Migration {
        name: "0001_local_models_add_revision_and_files",
        statements: &[
            "ALTER TABLE local_models ADD COLUMN revision TEXT",
            "ALTER TABLE local_models ADD COLUMN files INTEGER",
        ],
    },
    // `verified_files` counts how many manifest entries have a recorded
    // sha256 (i.e. were verified against an HF-published LFS digest at
    // download time). NULL means we haven't computed it yet — the next
    // install/update pass will backfill it.
    Migration {
        name: "0002_local_models_add_verified_files",
        statements: &["ALTER TABLE local_models ADD COLUMN verified_files INTEGER"],
    },
    // `served_models` tracks the user's desired serving set so the
    // OVMS controller can restore the same models on every app start.
    // Fresh installs already get the table from BASELINE; this row just
    // records the same history in upgraded dbs.
    Migration {
        name: "0003_create_served_models",
        statements: &["CREATE TABLE IF NOT EXISTS served_models (\
                model_id   TEXT PRIMARY KEY,\
                added_at   TEXT NOT NULL\
             )"],
    },
    // Per-conversation override of the enabled-tool catalog. `NULL` =
    // "inherit from Settings" (every tool currently enabled on the Tools
    // page is offered to the model). Non-NULL is a JSON array of
    // catalog keys (`<server_id>::<tool_name>`) the user has explicitly
    // disabled for this conversation. The runner subtracts these from
    // `mcp_catalog::fetch_enabled` before building the OpenAI `tools`
    // request.
    Migration {
        name: "0004_conversations_add_disabled_tools",
        statements: &["ALTER TABLE conversations ADD COLUMN disabled_tools TEXT"],
    },
    // Per-conversation sampling override. NULL = "inherit from the
    // active provider's `sampling` block (which in turn falls back to
    // the per-model `ModelProfile` default)". Non-NULL is a JSON
    // encoding of [`crate::settings::SamplingConfig`] — each field is
    // independently optional so a chat can override just `temperature`
    // without disturbing the provider-wide `top_p` / `top_k`.
    Migration {
        name: "0005_conversations_add_sampling",
        statements: &["ALTER TABLE conversations ADD COLUMN sampling TEXT"],
    },
    // Task actions used to be a plain `prompt` string aimed at the agent
    // runner. They now carry a tagged JSON payload (`command` / `script`
    // / `notify` / `prompt`) so a scheduled task can do something other
    // than talk to the LLM. Existing rows are backfilled by wrapping the
    // old prompt as `{"kind":"prompt","prompt":...,"notify":false}`; the
    // legacy `prompt` column is kept for read-side fallback in case any
    // upgrade path missed the backfill.
    Migration {
        name: "0006_tasks_add_action_json",
        statements: &[
            "ALTER TABLE tasks ADD COLUMN action_json TEXT",
            "UPDATE tasks SET action_json = json_object('kind','prompt','prompt',prompt,'notify',json('false')) \
             WHERE action_json IS NULL OR action_json = ''",
        ],
    },
    // Per-turn capability flags (web search, deep research, thinking
    // opt-in, autonomous loop) the composer attached to a user
    // message. NULL = "no overrides set" (the default, all flags off);
    // otherwise a JSON-encoded [`crate::chat::TurnOverrides`]. The
    // runner reads this from the latest user row instead of scanning
    // the message content for slash prefixes (`/web`, `/loop`, ...).
    // Old rows persisted before this column existed deserialise as
    // None and the runner falls back to the legacy slash parser so
    // existing history keeps replaying correctly.
    Migration {
        name: "0007_messages_add_turn_overrides",
        statements: &["ALTER TABLE messages ADD COLUMN turn_overrides TEXT"],
    },
    // `pipeline_tag` mirrors the upstream HuggingFace `pipeline_tag`
    // (e.g. `text-generation`, `image-text-to-text`, `feature-extraction`)
    // captured at download time so the Models page can categorise each
    // local model without re-hitting the network. NULL on rows that
    // pre-date this column — the Models read path derives a display
    // value from the per-model sidecar in that case (and ultimately
    // falls back to `text-generation`).
    Migration {
        name: "0008_local_models_add_pipeline_tag",
        statements: &["ALTER TABLE local_models ADD COLUMN pipeline_tag TEXT"],
    },
];

/// Baseline schema. New tables and new columns on existing tables both go
/// here too — the `CREATE TABLE IF NOT EXISTS` keeps fresh installs in sync
/// with whatever the latest migration produces. Older dbs are reconciled by
/// the per-migration entries above.
const BASELINE: &str = r#"
CREATE TABLE IF NOT EXISTS conversations (
    id              TEXT PRIMARY KEY,
    title           TEXT NOT NULL,
    model           TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    disabled_tools  TEXT,            -- json array of `<server>::<tool>` keys
    sampling        TEXT             -- json SamplingConfig override
);

CREATE TABLE IF NOT EXISTS messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role            TEXT NOT NULL,
    content         TEXT NOT NULL,
    thinking        TEXT,
    attachments     TEXT,            -- json array
    turn_overrides  TEXT,            -- json TurnOverrides (user rows only)
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conversation_id, created_at);

CREATE TABLE IF NOT EXISTS local_models (
    id              TEXT PRIMARY KEY,
    hf_id           TEXT,
    path            TEXT NOT NULL,
    bytes           INTEGER NOT NULL,
    added_at        TEXT NOT NULL,
    revision        TEXT,
    files           INTEGER,
    verified_files  INTEGER,
    pipeline_tag    TEXT
);

CREATE TABLE IF NOT EXISTS tasks (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    prompt          TEXT NOT NULL DEFAULT '',  -- legacy; new rows use action_json
    action_json     TEXT,                       -- tagged TaskAction payload
    trigger_json    TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    last_run_at     TEXT,
    last_status     TEXT,
    created_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_facts (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,   -- 'fact' | 'preference' | 'pinned'
    content     TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

-- Persistent record of which models the user wants OVMS to keep loaded.
-- The OVMS controller mirrors this set into `config.json` on every start
-- (and on every `POST /v1/config/reload`) so the same models come back
-- after restart without manual reloading from the UI.
CREATE TABLE IF NOT EXISTS served_models (
    model_id   TEXT PRIMARY KEY,
    added_at   TEXT NOT NULL
);

-- One row per installed runtime. Lets us survive restarts without re-probing
-- the filesystem, and gives a single audit trail for downloads/updates.
CREATE TABLE IF NOT EXISTS runtime_versions (
    name          TEXT PRIMARY KEY,   -- 'ovms' | future: 'ollama' | 'vllm'
    version       TEXT NOT NULL,
    install_dir   TEXT NOT NULL,
    executable    TEXT NOT NULL,
    installed_at  TEXT NOT NULL,
    source_url    TEXT,
    metadata      TEXT                 -- json blob
);
"#;
