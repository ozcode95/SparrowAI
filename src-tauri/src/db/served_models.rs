//! CRUD for the `served_models` table.
//!
//! Tracks the set of model ids the user wants OVMS to keep loaded. The
//! controller mirrors this table into a live `mediapipe_config_list` in
//! `config.json` and asks OVMS to reload it via the TFS
//! [`/v1/config/reload`](crate::ovms::tfs) endpoint.
//!
//! Persisting the set means restarting zero always brings the same models
//! back online — the user never has to re-load text, embedding, rerank,
//! s2t, t2s, and image-gen models one at a time after every app start.
//!
//! On startup, the controller's `reconcile_config_to_db` function syncs
//! any models already present in `config.json` back into this table,
//! ensuring manually edited configs are properly recognized.

use anyhow::Result;
use chrono::Utc;
use sqlx::{Row, SqlitePool};

/// Insertion-ordered list (oldest first) of every model the controller
/// should re-stage on the next OVMS start.
pub async fn list(pool: &SqlitePool) -> Result<Vec<String>> {
    let rows =
        sqlx::query("SELECT model_id FROM served_models ORDER BY added_at ASC, model_id ASC")
            .fetch_all(pool)
            .await?;
    Ok(rows
        .iter()
        .map(|r| r.get::<String, _>("model_id"))
        .collect())
}

/// Add `model_id` to the persistent serving set. No-op when the id is
/// already present (the `INSERT OR IGNORE` keeps the original `added_at`
/// so we don't reshuffle insertion order on every re-load).
pub async fn add(pool: &SqlitePool, model_id: &str) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO served_models (model_id, added_at) VALUES (?, ?)")
        .bind(model_id)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove `model_id` from the persistent serving set. Safe to call when
/// the id isn't present.
pub async fn remove(pool: &SqlitePool, model_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM served_models WHERE model_id = ?")
        .bind(model_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Wipe the entire serving set. Used by `stop()` only when the user
/// explicitly asks to forget what was loaded (currently exposed as a
/// dedicated `ovms_clear_served` command).
#[allow(dead_code)]
pub async fn clear(pool: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM served_models")
        .execute(pool)
        .await?;
    Ok(())
}
