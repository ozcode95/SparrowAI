//! CRUD for the `runtime_versions` table.
//!
//! Generic over runtime name (`ovms`, future `ollama` / `llama.cpp`), so all
//! runtime lifecycle bookkeeping lives in one place.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeVersion {
    pub name: String,
    pub version: String,
    pub install_dir: String,
    pub executable: String,
    pub installed_at: String,
    pub source_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

pub async fn get(pool: &SqlitePool, name: &str) -> Result<Option<RuntimeVersion>> {
    let row = sqlx::query(
        "SELECT name, version, install_dir, executable, installed_at, source_url, metadata
         FROM runtime_versions WHERE name = ?",
    )
    .bind(name)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let meta_json: Option<String> = r.try_get("metadata").ok();
        let metadata = meta_json.and_then(|s| serde_json::from_str(&s).ok());
        RuntimeVersion {
            name: r.get("name"),
            version: r.get("version"),
            install_dir: r.get("install_dir"),
            executable: r.get("executable"),
            installed_at: r.get("installed_at"),
            source_url: r.try_get("source_url").ok(),
            metadata,
        }
    }))
}

pub async fn upsert(pool: &SqlitePool, rv: &RuntimeVersion) -> Result<()> {
    let meta_json = rv
        .metadata
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let installed_at = if rv.installed_at.is_empty() {
        Utc::now().to_rfc3339()
    } else {
        rv.installed_at.clone()
    };
    sqlx::query(
        "INSERT INTO runtime_versions
            (name, version, install_dir, executable, installed_at, source_url, metadata)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(name) DO UPDATE SET
            version      = excluded.version,
            install_dir  = excluded.install_dir,
            executable   = excluded.executable,
            installed_at = excluded.installed_at,
            source_url   = excluded.source_url,
            metadata     = excluded.metadata",
    )
    .bind(&rv.name)
    .bind(&rv.version)
    .bind(&rv.install_dir)
    .bind(&rv.executable)
    .bind(&installed_at)
    .bind(rv.source_url.as_deref())
    .bind(meta_json.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, name: &str) -> Result<()> {
    sqlx::query("DELETE FROM runtime_versions WHERE name = ?")
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}
