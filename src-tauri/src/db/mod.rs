//! SQLite pool + schema.
//!
//! We avoid sqlx's compile-time macros (`query!`, `migrate!`) so the schema
//! can evolve without forcing a `DATABASE_URL` at build time. The schema is
//! applied as a single idempotent baseline on every boot — we don't track
//! per-bump migrations yet because zero is still pre-release and any change
//! lands directly in [`BASELINE`]. Once we ship, additive migrations should
//! be appended to a new `MIGRATIONS` ledger here.

pub mod runtimes;
pub mod served_models;

use anyhow::{Context, Result};
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

/// Apply the baseline schema.
///
/// The baseline is itself idempotent (everything is `CREATE TABLE IF NOT
/// EXISTS`) so it can be re-applied freely on every boot. There are no
/// per-version migrations yet — the app is pre-release and every schema
/// change lands in [`BASELINE`] directly.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::query(BASELINE)
        .execute(pool)
        .await
        .context("apply baseline schema")?;
    Ok(())
}

/// Baseline schema. The single source of truth for the on-disk shape.
/// `CREATE TABLE IF NOT EXISTS` keeps existing dbs intact across pre-release
/// schema bumps; once we ship, future additive changes should land as
/// per-version `Migration` entries above instead of being edited in place.
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
    action_json     TEXT NOT NULL,              -- tagged TaskAction payload
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
    name          TEXT PRIMARY KEY,   -- 'ovms' | future: 'ollama' | 'llama.cpp'
    version       TEXT NOT NULL,
    install_dir   TEXT NOT NULL,
    executable    TEXT NOT NULL,
    installed_at  TEXT NOT NULL,
    source_url    TEXT,
    metadata      TEXT                 -- json blob
);
"#;
