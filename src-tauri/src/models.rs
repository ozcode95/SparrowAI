use crate::paths;
use std::fs;
use std::path::PathBuf;

/// Check if model files exist in a directory
#[allow(dead_code)]
pub fn has_model_files(dir: &PathBuf) -> bool {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    // Check for common model files
                    if file_name.ends_with(".json")
                        || file_name.ends_with(".bin")
                        || file_name.ends_with(".safetensors")
                        || file_name.ends_with(".model")
                        || file_name == "README.md"
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Get the models directory path from option or default
pub fn get_models_dir_from_option(download_path: Option<String>) -> Result<PathBuf, String> {
    if let Some(path) = download_path {
        Ok(PathBuf::from(path))
    } else {
        paths::get_models_dir().map_err(|e| e.to_string())
    }
}

/// Normalize model ID to include OpenVINO organization prefix
pub fn normalize_model_id(model_id: &str) -> String {
    if model_id.starts_with("OpenVINO/") {
        model_id.to_string()
    } else {
        format!("OpenVINO/{}", model_id)
    }
}

#[tauri::command]
pub async fn check_downloaded_models(_download_path: Option<String>) -> Result<Vec<String>, String> {
    use crate::huggingface::get_all_model_metadata;
    
    // Read from metadata file instead of scanning filesystem
    let metadata = get_all_model_metadata().await?;
    
    // Return all model IDs from metadata
    let downloaded_models: Vec<String> = metadata.keys()
        .map(|id| id.to_string())
        .collect();
    
    Ok(downloaded_models)
}

#[tauri::command]
pub async fn delete_downloaded_model(
    model_id: String,
    download_path: Option<String>,
) -> Result<String, String> {
    use crate::huggingface::remove_model_metadata;
    
    let normalized_model_id = normalize_model_id(&model_id);
    let base_dir = get_models_dir_from_option(download_path)?;
    let model_dir = base_dir.join(&normalized_model_id);

    if !model_dir.exists() {
        return Err(format!("Model directory does not exist: {}", model_dir.display()));
    }

    fs::remove_dir_all(&model_dir)
        .map_err(|e| format!("Failed to delete model {}: {}", normalized_model_id, e))?;

    // If this was an org/model structure, check if the org directory is now empty
    if normalized_model_id.contains('/') {
        let org_name = normalized_model_id.split('/').next().unwrap();
        let org_dir = base_dir.join(org_name);

        if org_dir.exists() {
            if let Ok(entries) = fs::read_dir(&org_dir) {
                if entries.count() == 0 {
                    // Remove empty org directory
                    let _ = fs::remove_dir(&org_dir);
                }
            }
        }
    }

    // Remove from metadata
    if let Err(e) = remove_model_metadata(&normalized_model_id).await {
        tracing::warn!(
            error = %e,
            model_id = %normalized_model_id,
            "Failed to remove model from metadata, but model files were deleted"
        );
    }

    Ok(format!("Successfully deleted model: {}", normalized_model_id))
}

#[tauri::command]
pub async fn open_model_folder(
    model_id: String,
    download_path: Option<String>,
) -> Result<String, String> {
    let normalized_model_id = normalize_model_id(&model_id);
    let base_dir = get_models_dir_from_option(download_path)?;
    let model_dir = base_dir.join(&normalized_model_id);

    if !model_dir.exists() {
        return Err(format!("Model directory does not exist: {}", model_dir.display()));
    }

    // Use different commands based on the OS
    #[cfg(target_os = "windows")]
    {
        let windows_path = model_dir.to_string_lossy().replace('/', "\\");
        std::process::Command::new("explorer")
            .arg(&windows_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
        Ok(format!("Opened folder: {}", model_dir.display()))
    }

    #[cfg(not(target_os = "windows"))]
    {
        Err("Unsupported operating system".to_string())
    }
}

#[tauri::command]
pub async fn list_directory_names(path: String) -> Result<Vec<String>, String> {
    let dir_path = PathBuf::from(&path);

    if !dir_path.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();

    for entry in fs::read_dir(&dir_path).map_err(|e| format!("Failed to read directory: {}", e))?.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            // Check for nested OpenVINO organization structure
            if let Some(dir_name) = entry.file_name().to_str() {
                if dir_name == "OpenVINO" {
                    // List models inside OpenVINO directory
                    if let Ok(org_entries) = fs::read_dir(&entry_path) {
                        for org_entry in org_entries.flatten() {
                            if org_entry.path().is_dir() {
                                if let Some(model_name) = org_entry.file_name().to_str() {
                                    names.push(format!("OpenVINO/{}", model_name));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(names)
}

#[tauri::command]
pub async fn delete_directory(path: String) -> Result<String, String> {
    let dir_path = PathBuf::from(&path);

    if !dir_path.exists() {
        return Err(format!("Directory does not exist: {}", path));
    }

    fs::remove_dir_all(&dir_path)
        .map_err(|e| format!("Failed to delete directory: {}", e))?;

    Ok(format!("Deleted directory: {}", path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_model_id() {
        assert_eq!(normalize_model_id("model-name"), "OpenVINO/model-name");
        assert_eq!(normalize_model_id("OpenVINO/model-name"), "OpenVINO/model-name");
    }
}
