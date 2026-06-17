//! Unified app error type. Anything that crosses the Tauri boundary becomes
//! a `String` so the frontend gets a stable shape.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("db: {0}")]
    Db(#[from] sqlx::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("not initialised")]
    NotInitialised,

    #[error("{0}")]
    Other(String),

    #[error(transparent)]
    Any(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, AppError>;

/// Tauri commands return `Result<T, IpcError>` so the JS side gets a string.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct IpcError(pub String);

impl<E: std::fmt::Display> From<E> for IpcError {
    fn from(e: E) -> Self {
        IpcError(e.to_string())
    }
}

pub type IpcResult<T> = std::result::Result<T, IpcError>;
