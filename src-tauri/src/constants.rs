/// Default text chunk size for document processing
pub const DEFAULT_CHUNK_SIZE: usize = 1000;

/// Default text chunk overlap for document processing
pub const DEFAULT_CHUNK_OVERLAP: usize = 200;

/// Default search result limit
#[allow(dead_code)]
pub const DEFAULT_SEARCH_LIMIT: usize = 10;

/// Default model search limit
pub const DEFAULT_MODEL_SEARCH_LIMIT: u32 = 20;

/// Maximum model search limit
pub const MAX_MODEL_SEARCH_LIMIT: u32 = 100;

/// OVMS download URL
pub const OVMS_DOWNLOAD_URL: &str =
    "https://github.com/openvinotoolkit/model_server/releases/download/v2025.4/ovms_windows_python_off.zip";

/// OVMS zip file name
pub const OVMS_ZIP_FILE: &str = "ovms_windows_python_off.zip";

/// OVMS executable name
pub const OVMS_EXE_NAME: &str = "ovms.exe";

/// OVMS config file name
pub const OVMS_CONFIG_FILE: &str = "models_config.json";

/// Default OVMS port
#[allow(dead_code)]
pub const OVMS_DEFAULT_PORT: u16 = 1114;

/// OVMS API base URL
pub const OVMS_API_BASE: &str = "http://localhost:1114";

/// OVMS OpenAI-compatible API path
pub const OVMS_OPENAI_PATH: &str = "/v3";

/// Default embedding model name
pub const DEFAULT_EMBEDDING_MODEL: &str = "bge-base-en-v1.5-int8-ov";

/// OpenVINO organization name on HuggingFace
pub const OPENVINO_ORG: &str = "OpenVINO";

/// Default chat session title
pub const DEFAULT_CHAT_TITLE: &str = "New Chat";

/// Maximum chat title length
pub const MAX_CHAT_TITLE_LENGTH: usize = 60;

/// Minimum chat title length before truncation
pub const MIN_CHAT_TITLE_LENGTH: usize = 40;

/// Download progress emit interval (milliseconds)
pub const DOWNLOAD_PROGRESS_INTERVAL_MS: u128 = 100;

/// Download timeout (seconds)
pub const DOWNLOAD_TIMEOUT_SECS: u64 = 600;

/// Maximum download retries
pub const MAX_DOWNLOAD_RETRIES: u8 = 3;

/// Log retention days
pub const LOG_RETENTION_DAYS: i64 = 30;

/// Archive retention days
#[allow(dead_code)]
pub const ARCHIVE_RETENTION_DAYS: i64 = 90;

/// Model file extensions to check
#[allow(dead_code)]
pub const MODEL_FILE_EXTENSIONS: &[&str] = &[".json", ".bin", ".safetensors", ".model"];

/// Special model files to check
#[allow(dead_code)]
pub const SPECIAL_MODEL_FILES: &[&str] = &["README.md"];

/// User agent for HTTP requests
pub const USER_AGENT: &str = "SparrowAI/1.0";

/// HuggingFace API base URL
pub const HUGGINGFACE_API_BASE: &str = "https://huggingface.co/api";

/// Default log level filter
pub const DEFAULT_LOG_FILTER: &str = "info,sparrow=debug";

/// Environment variable names
#[allow(dead_code)]
pub mod env_vars {
    pub const USERPROFILE: &str = "USERPROFILE";
    pub const HOME: &str = "HOME";
}

/// Directory names
#[allow(dead_code)]
pub mod dirs {
    pub const SPARROW: &str = ".sparrow";
    pub const MODELS: &str = "models";
    pub const OVMS: &str = "ovms";
    pub const LOGS: &str = "logs";
    pub const ARCHIVE: &str = "archive";
    pub const VECTOR_STORE: &str = "vector_store";
    pub const MCP: &str = "mcp";
}

/// File names
#[allow(dead_code)]
pub mod files {
    pub const CHAT_SESSIONS: &str = "chat_sessions.json";
    pub const MCP_CONFIG: &str = "config.json";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_defined() {
        assert!(DEFAULT_CHUNK_SIZE > 0);
        assert!(DEFAULT_CHUNK_OVERLAP < DEFAULT_CHUNK_SIZE);
        assert!(!OVMS_DOWNLOAD_URL.is_empty());
        assert!(!DEFAULT_EMBEDDING_MODEL.is_empty());
    }
}
