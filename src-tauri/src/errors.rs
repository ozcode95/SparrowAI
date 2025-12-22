use std::io;
use std::path::PathBuf;

/// Central error type for the application
#[derive(Debug, thiserror::Error)]
pub enum SparrowError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON serialization error: {0}")]
    JsonSerialization(#[from] serde_json::Error),

    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Home directory not found")]
    HomeDirectoryNotFound,

    #[error("Path error: {path}")]
    PathError { path: PathBuf },

    #[error("File not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("Directory not found: {path}")]
    DirectoryNotFound { path: PathBuf },

    #[error("Invalid configuration: {message}")]
    InvalidConfig { message: String },

    #[error("Model not found: {model_id}")]
    ModelNotFound { model_id: String },

    #[error("Chat session not found: {session_id}")]
    SessionNotFound { session_id: String },

    #[error("OVMS error: {message}")]
    OvmsError { message: String },

    #[error("OVMS not found")]
    OvmsNotFound,

    #[error("MCP server error: {server_name} - {message}")]
    McpServerError { server_name: String, message: String },

    #[error("MCP server not found: {server_name}")]
    McpServerNotFound { server_name: String },

    #[error("Embedding generation failed: {message}")]
    EmbeddingError { message: String },

    #[error("Vector store error: {message}")]
    VectorStoreError { message: String },

    #[error("Document processing error: {file_path} - {message}")]
    DocumentProcessingError { file_path: String, message: String },

    #[error("Lock poisoned: {resource}")]
    LockPoisoned { resource: String },

    #[error("Invalid input: {message}")]
    InvalidInput { message: String },

    #[error("Operation failed: {message}")]
    OperationFailed { message: String },

    #[error("Not supported: {operation}")]
    NotSupported { operation: String },
}

/// Result type alias for the application
pub type Result<T> = std::result::Result<T, SparrowError>;

/// Convert SparrowError to String for Tauri command compatibility
impl From<SparrowError> for String {
    fn from(error: SparrowError) -> Self {
        error.to_string()
    }
}

/// Helper to convert std::sync::PoisonError to SparrowError
impl<T> From<std::sync::PoisonError<T>> for SparrowError {
    fn from(_: std::sync::PoisonError<T>) -> Self {
        SparrowError::LockPoisoned {
            resource: "mutex".to_string(),
        }
    }
}
