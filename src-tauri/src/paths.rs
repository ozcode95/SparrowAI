use crate::errors::{Result, SparrowError};
use std::path::PathBuf;
use tauri::AppHandle;

/// Get the user's home directory
pub fn get_home_dir() -> Result<PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .map_err(|_| SparrowError::HomeDirectoryNotFound)
}

/// Get the main .sparrow directory in the user's home
pub fn get_sparrow_dir() -> Result<PathBuf> {
    Ok(get_home_dir()?.join(".sparrow"))
}

/// Ensure a directory exists, creating it if necessary
pub fn ensure_dir_exists(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Get the .sparrow/models directory
pub fn get_models_dir() -> Result<PathBuf> {
    let dir = get_sparrow_dir()?.join("models");
    ensure_dir_exists(&dir)?;
    Ok(dir)
}

/// Get the .sparrow/ovms directory
pub fn get_ovms_dir(_app_handle: Option<&AppHandle>) -> Result<PathBuf> {
    Ok(get_sparrow_dir()?.join("ovms"))
}

/// Get the OVMS config file path
pub fn get_ovms_config_path(_app_handle: Option<&AppHandle>) -> Result<PathBuf> {
    Ok(get_ovms_dir(None)?.join("models_config.json"))
}

/// Get the OVMS executable path
pub fn get_ovms_exe_path(_app_handle: Option<&AppHandle>) -> Result<PathBuf> {
    Ok(get_ovms_dir(None)?.join("ovms.exe"))
}

/// Get the .sparrow/logs directory
pub fn get_logs_dir() -> Result<PathBuf> {
    Ok(get_sparrow_dir()?.join("logs"))
}

/// Get the .sparrow/logs/archive directory
pub fn get_logs_archive_dir() -> Result<PathBuf> {
    Ok(get_logs_dir()?.join("archive"))
}

/// Get the chat sessions file path
pub fn get_chat_sessions_path() -> Result<PathBuf> {
    let sparrow_dir = get_sparrow_dir()?;
    ensure_dir_exists(&sparrow_dir)?;
    Ok(sparrow_dir.join("chat_sessions.json"))
}

/// Get the vector store database path
pub fn get_vector_store_path() -> Result<PathBuf> {
    let db_dir = get_sparrow_dir()?.join("vector_store");
    ensure_dir_exists(&db_dir)?;
    Ok(db_dir)
}

/// Get the MCP config file path
pub fn get_mcp_config_path(_app_handle: &AppHandle) -> Result<PathBuf> {
    let config_dir = get_sparrow_dir()?.join("mcp");
    ensure_dir_exists(&config_dir)?;
    Ok(config_dir.join("config.json"))
}

/// Get the model metadata file path
pub fn get_model_metadata_path() -> Result<PathBuf> {
    let models_dir = get_models_dir()?;
    Ok(models_dir.join("model_metadata.json"))
}

/// Get the tasks file path
pub fn get_tasks_path() -> Result<PathBuf> {
    Ok(get_sparrow_dir()?.join("tasks.json"))
}

/// Get the images directory path
pub fn get_images_dir() -> Result<PathBuf> {
    let dir = get_sparrow_dir()?.join("images");
    ensure_dir_exists(&dir)?;
    Ok(dir)
}

/// Get the OpenVINO model path for a specific model
#[allow(dead_code)]
pub fn get_openvino_model_path(model_name: &str) -> Result<PathBuf> {
    Ok(get_models_dir()?.join("OpenVINO").join(model_name))
}

/// Validate that a path exists and is a file
#[allow(dead_code)]
pub fn validate_file_exists(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        return Err(SparrowError::FileNotFound { path: path.clone() });
    }
    if !path.is_file() {
        return Err(SparrowError::PathError { path: path.clone() });
    }
    Ok(())
}

/// Validate that a path exists and is a directory
#[allow(dead_code)]
pub fn validate_dir_exists(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        return Err(SparrowError::DirectoryNotFound { path: path.clone() });
    }
    if !path.is_dir() {
        return Err(SparrowError::PathError { path: path.clone() });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_home_dir() {
        let result = get_home_dir();
        assert!(result.is_ok());
        assert!(result.unwrap().exists());
    }

    #[test]
    fn test_get_sparrow_dir() {
        let result = get_sparrow_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.ends_with(".sparrow"));
    }
}
