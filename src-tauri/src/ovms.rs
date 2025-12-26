use std::fs;
use std::io::{ Write, Read };
use std::path::PathBuf;
use std::process::{ Command, Stdio, Child };
use std::sync::{ Arc, Mutex };
use zip::ZipArchive;
use serde_json::{ json, Value };
use serde::{ Deserialize, Serialize };
use tauri::AppHandle;
use tracing::{ info, warn, error, debug };

use crate::{ paths, constants };

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OvmsStatus {
    pub status: String,
    pub loaded_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ModelVersionStatus {
    version: String,
    state: String,
    status: ModelStatus,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ModelStatus {
    error_code: String,
    error_message: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ModelInfo {
    model_version_status: Vec<ModelVersionStatus>,
}

// Global OVMS process management
static OVMS_PROCESS: std::sync::OnceLock<Arc<Mutex<Option<Child>>>> = std::sync::OnceLock::new();

// Get loaded models from models_config.json
#[tauri::command]
pub async fn get_loaded_models(app_handle: AppHandle) -> Result<Vec<String>, String> {
    let config_path = paths::get_ovms_config_path(Some(&app_handle))
        .map_err(|e| e.to_string())?;
    
    if !config_path.exists() {
        return Ok(Vec::new());
    }
    
    let config_str = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;
    
    let config: Value = serde_json::from_str(&config_str)
        .map_err(|e| format!("Failed to parse config file: {}", e))?;
    
    let mut loaded_models = Vec::new();
    
    if let Some(model_list) = config["mediapipe_config_list"].as_array() {
        for model in model_list {
            if let Some(name) = model["name"].as_str() {
                loaded_models.push(name.to_string());
            }
        }
    }
    
    Ok(loaded_models)
}

#[allow(dead_code)]
pub fn create_minimal_test_config(config_path: &PathBuf) -> Result<(), String> {
    // Create parent directories if they don't exist
    if let Some(parent) = config_path.parent() {
        fs
            ::create_dir_all(parent)
            .map_err(|e| format!("Failed to create parent directories: {}", e))?;
    }

    // Create a minimal empty configuration that OVMS can parse
    let config = json!({
        "mediapipe_config_list": [],
        "model_config_list": []
    });

    let config_str = serde_json
        ::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    fs::write(config_path, config_str).map_err(|e| format!("Failed to write config file: {}", e))?;

    info!(config_path = %config_path.display(), "Created minimal OVMS config");
    Ok(())
}

pub fn validate_ovms_config(config_path: &PathBuf) -> Result<(), String> {
    tracing::debug!(config_path = %config_path.display(), "Validating OVMS configuration");
    
    if !config_path.exists() {
        return Err(format!("Config file does not exist: {}", config_path.display()));
    }

    if !config_path.is_file() {
        return Err(format!("Config path is not a file: {}", config_path.display()));
    }

    // Read and validate JSON structure
    let config_str = fs
        ::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    let config: Value = serde_json
        ::from_str(&config_str)
        .map_err(|e| format!("Invalid JSON in config file: {}", e))?;

    // Check for required fields
    if !config.is_object() {
        return Err("Config must be a JSON object".to_string());
    }

    if config.get("model_config_list").is_none() {
        return Err("Config must contain 'model_config_list' field".to_string());
    }

    if !config["model_config_list"].is_array() {
        return Err("'model_config_list' must be an array".to_string());
    }

    if config.get("mediapipe_config_list").is_none() {
        return Err("Config must contain 'mediapipe_config_list' field".to_string());
    }

    if !config["mediapipe_config_list"].is_array() {
        return Err("'mediapipe_config_list' must be an array".to_string());
    }

    tracing::debug!(config_path = %config_path.display(), "OVMS config validation passed");
    Ok(())
}

#[tauri::command]
pub async fn download_ovms(app_handle: AppHandle) -> Result<String, String> {
    log_operation_start!("Downloading OVMS");
    
    let sparrow_dir = paths::get_sparrow_dir()
        .map_err(|e| e.to_string())?;
    let ovms_dir = paths::get_ovms_dir(Some(&app_handle))
        .map_err(|e| e.to_string())?;

    // Create both directories if they don't exist
    if !sparrow_dir.exists() {
        fs
            ::create_dir_all(&sparrow_dir)
            .map_err(|e| format!("Failed to create .sparrow directory: {}", e))?;
    }
    if !ovms_dir.exists() {
        fs
            ::create_dir_all(&ovms_dir)
            .map_err(|e| format!("Failed to create ovms directory: {}", e))?;
    }

    // Download zip to .sparrow root directory
    let zip_path = sparrow_dir.join(constants::OVMS_ZIP_FILE);

    // Check if OVMS executable already exists
    let ovms_exe = paths::get_ovms_exe_path(Some(&app_handle))
        .map_err(|e| e.to_string())?;
    if ovms_exe.exists() {
        log_operation_success!("OVMS already present", 
            path = %ovms_exe.display()
        );
        return Ok("OVMS already downloaded and extracted".to_string());
    }

    // Remove any existing corrupted zip file
    if zip_path.exists() {
        if let Err(e) = fs::remove_file(&zip_path) {
            log_warning!("Failed to remove existing zip file", error = %e);
        } else {
            tracing::debug!("Removed existing zip file for fresh download");
        }
    }

    // Download the file with retry logic and better error handling
    let client = reqwest::Client
        ::builder()
        .user_agent(constants::USER_AGENT)
        .timeout(std::time::Duration::from_secs(constants::DOWNLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|e| {
            log_operation_error!("OVMS download setup", &e);
            format!("Failed to create HTTP client: {}", e)
        })?;

    log_progress!("Starting OVMS download", url = %constants::OVMS_DOWNLOAD_URL);

    let mut retries = constants::MAX_DOWNLOAD_RETRIES;

    while retries > 0 {
        match download_and_validate(&client, &zip_path).await {
            Ok(_bytes) => {
                break;
            }
            Err(e) => {
                retries -= 1;
                log_warning!(
                    "OVMS download attempt failed", 
                    error = %e,
                    attempts_remaining = retries
                );

                // Remove corrupted file if it exists
                if zip_path.exists() {
                    let _ = fs::remove_file(&zip_path);
                }

                if retries == 0 {
                    log_operation_error!("OVMS download", &e);
                    return Err(format!("Failed to download OVMS after 3 attempts: {}", e));
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    }

    log_progress!("OVMS download completed, extracting...");

    // Extract the zip file to ovms directory
    extract_ovms(&zip_path, &ovms_dir)?;

    // Clean up the zip file after successful extraction
    if zip_path.exists() {
        if let Err(e) = fs::remove_file(&zip_path) {
            log_warning!("Failed to cleanup zip file", 
                zip_path = %zip_path.display(),
                error = %e
            );
        } else {
            tracing::debug!(zip_path = %zip_path.display(), "Cleaned up zip file");
        }
    }

    log_operation_success!("OVMS downloaded and extracted");
    Ok("OVMS downloaded and extracted successfully".to_string())
}

async fn download_and_validate(
    client: &reqwest::Client,
    zip_path: &PathBuf
) -> Result<Vec<u8>, String> {
    let response = client
        .get(constants::OVMS_DOWNLOAD_URL)
        .send().await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    // Get content length for validation
    let expected_length = response.content_length();
    if let Some(length) = expected_length {
        let size_mb = length / 1024 / 1024;
        log_progress!("Downloading OVMS", size_mb = size_mb);
        tracing::debug!(size_bytes = length, size_mb = size_mb, "Download size");
    }

    let bytes = response
        .bytes().await
        .map_err(|e| format!("Failed to read response bytes: {}", e))?;

    // Validate content length if provided
    if let Some(expected) = expected_length {
        if (bytes.len() as u64) != expected {
            return Err(
                format!(
                    "Downloaded size mismatch: expected {} bytes, got {} bytes",
                    expected,
                    bytes.len()
                )
            );
        }
    }

    // Validate that it's a valid ZIP file before writing
    validate_zip_bytes(&bytes)?;

    info!("Download validation passed, writing to file...");

    // Write to file
    let mut file = fs::File
        ::create(zip_path)
        .map_err(|e| format!("Failed to create zip file: {}", e))?;

    file.write_all(&bytes).map_err(|e| format!("Failed to write zip file: {}", e))?;

    Ok(bytes.into())
}

fn validate_zip_bytes(bytes: &[u8]) -> Result<(), String> {
    use std::io::Cursor;

    // Check if it starts with ZIP magic number
    if bytes.len() < 4 {
        return Err("File too small to be a valid ZIP".to_string());
    }

    // ZIP files start with "PK" (0x504B)
    if &bytes[0..2] != b"PK" {
        return Err("Invalid ZIP file signature".to_string());
    }

    // Try to open as ZIP archive to validate structure
    let cursor = Cursor::new(bytes);
    match zip::ZipArchive::new(cursor) {
        Ok(archive) => {
            if archive.len() == 0 {
                return Err("ZIP file is empty".to_string());
            }
            info!(file_count = archive.len(), "ZIP validation passed");
            Ok(())
        }
        Err(e) => Err(format!("Invalid ZIP file structure: {}", e)),
    }
}

pub fn extract_ovms(zip_path: &PathBuf, extract_to: &PathBuf) -> Result<(), String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("Failed to open zip file: {}", e))?;

    let mut archive = ZipArchive::new(file).map_err(|e|
        format!("Failed to read zip archive: {}", e)
    )?;

    info!(file_count = archive.len(), "Extracting files from archive");

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read file {} from archive: {}", i, e))?;

        let file_name = file.name();
        debug!(file_name = %file_name, "Extracting file");

        // Skip directories (they end with '/')
        if file_name.ends_with('/') {
            continue;
        }

        // Strip the root directory from the path
        let relative_path = if let Some(slash_pos) = file_name.find('/') {
            &file_name[slash_pos + 1..]
        } else {
            file_name
        };

        // Skip if the relative path is empty
        if relative_path.is_empty() {
            continue;
        }

        let outpath = extract_to.join(relative_path);

        // Create parent directories if needed
        if let Some(p) = outpath.parent() {
            if !p.exists() {
                fs
                    ::create_dir_all(p)
                    .map_err(|e|
                        format!("Failed to create parent directory {}: {}", p.display(), e)
                    )?;
            }
        }

        // Extract the file
        let mut outfile = fs::File
            ::create(&outpath)
            .map_err(|e| format!("Failed to create output file {}: {}", outpath.display(), e))?;

        std::io
            ::copy(&mut file, &mut outfile)
            .map_err(|e| format!("Failed to extract file {}: {}", outpath.display(), e))?;

        debug!(output_path = %outpath.display(), "File extracted");
    }

    info!("Extraction completed successfully");
    Ok(())
}

#[tauri::command]
pub async fn create_ovms_config(
    app_handle: AppHandle,
    _model_name: String,
    _model_path: String
) -> Result<String, String> {
    // Create an empty configuration
    let config = json!({
        "mediapipe_config_list": [],
        "model_config_list": []
    });

    let config_str = serde_json
        ::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    let config_path = paths::get_ovms_config_path(Some(&app_handle))
        .map_err(|e| e.to_string())?;
    fs::write(&config_path, config_str).map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok("OVMS configuration file created successfully".to_string())
}

#[tauri::command]
pub async fn update_ovms_config(
    app_handle: AppHandle,
    model_name: String,
    model_path: String
) -> Result<String, String> {
    use crate::huggingface::{ get_model_type, ModelType };
    
    let config_path = paths::get_ovms_config_path(Some(&app_handle))
        .map_err(|e| e.to_string())?;

    // Read existing config or create new one
    let mut config: Value = if config_path.exists() {
        let config_str = fs
            ::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        serde_json
            ::from_str(&config_str)
            .map_err(|e| format!("Failed to parse config file: {}", e))?
    } else {
        json!({
            "mediapipe_config_list": [],
            "model_config_list": []
        })
    };

    // Normalize the model_path to use forward slashes for OVMS
    let normalized_model_path = model_path.replace('\\', "/");
    
    // Extract model ID from model_name (e.g., "Qwen2.5-VL-7B-Instruct-int4-ov" from full path)
    let model_id = if model_name.starts_with("OpenVINO/") {
        model_name.clone()
    } else {
        format!("OpenVINO/{}", model_name)
    };
    
    // Get the model type from metadata
    let model_type = get_model_type(&model_id).await.ok().flatten();
    
    tracing::info!(
        model_id = %model_id,
        model_type = ?model_type,
        "Updating OVMS config with model"
    );

    if let Some(model_list) = config["mediapipe_config_list"].as_array_mut() {
        // Build a map of existing models by type
        let mut models_by_type: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut rag_models: Vec<usize> = Vec::new();
        
        for (index, model) in model_list.iter().enumerate() {
            if let Some(name) = model["name"].as_str() {
                let full_model_id = if name.starts_with("OpenVINO/") {
                    name.to_string()
                } else {
                    format!("OpenVINO/{}", name)
                };
                
                // Get model type for this model
                if let Ok(Some(mtype)) = get_model_type(&full_model_id).await {
                    let type_str = mtype.as_str().to_string();
                    
                    // Track RAG models separately (embedding and reranker)
                    if matches!(mtype, ModelType::Embedding | ModelType::Reranker) {
                        rag_models.push(index);
                    } else {
                        // For non-RAG models, track by type
                        models_by_type.insert(type_str, index);
                    }
                }
            }
        }
        
        // Determine what to do with the new model
        if let Some(new_model_type) = model_type {
            let type_str = new_model_type.as_str().to_string();
            
            // Check if this is a RAG model
            if matches!(new_model_type, ModelType::Embedding | ModelType::Reranker) {
                // For RAG models, just update or add them without replacing
                let mut found = false;
                for i in 0..model_list.len() {
                    if let Some(name) = model_list[i]["name"].as_str() {
                        if name == model_name {
                            // Update existing model
                            model_list[i]["base_path"] = json!(normalized_model_path);
                            found = true;
                            break;
                        }
                    }
                }
                
                if !found {
                    // Add new RAG model
                    model_list.push(json!({
                        "name": model_name,
                        "base_path": normalized_model_path
                    }));
                }
            } else {
                // For non-RAG models, enforce one-per-type
                if let Some(&existing_index) = models_by_type.get(&type_str) {
                    // Replace the existing model of this type
                    model_list[existing_index] = json!({
                        "name": model_name,
                        "base_path": normalized_model_path
                    });
                    
                    tracing::info!(
                        model_type = %type_str,
                        replaced_index = existing_index,
                        "Replaced existing model of same type"
                    );
                } else {
                    // No model of this type exists, add it
                    model_list.push(json!({
                        "name": model_name,
                        "base_path": normalized_model_path
                    }));
                    
                    tracing::info!(
                        model_type = %type_str,
                        "Added new model of type"
                    );
                }
            }
        } else {
            // No type metadata available, just update or add the model
            let mut found = false;
            for i in 0..model_list.len() {
                if let Some(name) = model_list[i]["name"].as_str() {
                    if name == model_name {
                        // Update existing model
                        model_list[i]["base_path"] = json!(normalized_model_path);
                        found = true;
                        break;
                    }
                }
            }
            
            if !found {
                // Add new model
                model_list.push(json!({
                    "name": model_name,
                    "base_path": normalized_model_path
                }));
            }
            
            tracing::warn!(
                model_id = %model_id,
                "No type metadata found for model, added without type enforcement"
            );
        }
    }

    let config_str = serde_json
        ::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    fs::write(&config_path, config_str).map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok("OVMS configuration updated successfully".to_string())
}

#[tauri::command]
pub async fn reload_ovms_config() -> Result<String, String> {
    let client = reqwest::Client::new();

    let response = client
        .post("http://localhost:1114/v1/config/reload")
        .send().await
        .map_err(|e| format!("Failed to send reload request: {}", e))?;

    if response.status().is_success() {
        let body = response.text().await.map_err(|e| format!("Failed to read response: {}", e))?;
        Ok(format!("Config reloaded successfully: {}", body))
    } else {
        Err(format!("Config reload failed with status: {}", response.status()))
    }
}

// Check if OVMS is present on the system (Tauri command)
#[tauri::command]
pub async fn check_ovms_present(app_handle: AppHandle) -> Result<bool, String> {
    Ok(is_ovms_present(Some(&app_handle)))
}

// Check if OVMS is present on the system (internal function)
pub fn is_ovms_present(app_handle: Option<&AppHandle>) -> bool {
    let ovms_exe = match paths::get_ovms_exe_path(app_handle) {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to get OVMS exe path: {}", e);
            return false;
        }
    };
    info!(ovms_path = %ovms_exe.display(), "Checking for OVMS");

    if !ovms_exe.exists() || !ovms_exe.is_file() {
        return false;
    }

    // Check OVMS version
    match check_ovms_version(&ovms_exe) {
        Ok(is_valid) => {
            if !is_valid {
                // Version is too low, delete the OVMS folder
                if let Ok(ovms_dir) = crate::paths::get_ovms_dir(app_handle) {
                    if let Err(e) = fs::remove_dir_all(&ovms_dir) {
                        warn!(error = %e, ovms_dir = %ovms_dir.display(), "Failed to remove outdated OVMS directory");
                    } else {
                        info!(ovms_dir = %ovms_dir.display(), "Removed outdated OVMS directory");
                    }
                }
                return false;
            }
            true
        }
        Err(e) => {
            warn!(error = %e, "Failed to check OVMS version");
            false
        }
    }
}

fn check_ovms_version(ovms_exe: &PathBuf) -> Result<bool, String> {
    let mut cmd = Command::new(ovms_exe);
    cmd.arg("--version");

    // Hide console window on Windows
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = cmd.output().map_err(|e| format!("Failed to execute ovms --version: {}", e))?;

    if !output.status.success() {
        return Err(format!("OVMS --version command failed with status: {}", output.status));
    }

    let version_output = String::from_utf8_lossy(&output.stdout);
    info!(version_output = %version_output.trim(), "OVMS version output");

    // Parse version from output like "OpenVINO Model Server 2025.4"
    let required_version = (2025, 4, 0);

    for line in version_output.lines() {
        if line.starts_with("OpenVINO Model Server") {
            // Extract version string after "OpenVINO Model Server "
            if let Some(version_str) = line.strip_prefix("OpenVINO Model Server ") {
                // Parse version like "2025.4" - take only the numeric part before any suffix
                let version_parts: Vec<&str> = version_str.split('.').collect();
                if version_parts.len() >= 3 {
                    // Parse major.minor.patch
                    let major: u32 = version_parts[0].parse().map_err(|_| "Invalid major version")?;
                    let minor: u32 = version_parts[1].parse().map_err(|_| "Invalid minor version")?;

                    // For patch, only take numeric part before any non-numeric characters
                    let patch_str = version_parts[2];
                    let patch_numeric: String = patch_str
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    let patch: u32 = patch_numeric.parse().map_err(|_| "Invalid patch version")?;

                    let current_version = (major, minor, patch);
                    info!(current = ?current_version, required = ?required_version, "Comparing OVMS versions");

                    // Return true if current version >= required version
                    return Ok(current_version >= required_version);
                }
            }
        }
    }

    Err("Could not parse OVMS version from output".to_string())
}

#[tauri::command]
pub async fn start_ovms_server(app_handle: AppHandle) -> Result<String, String> {
    log_operation_start!("Starting OVMS server");
    
    // Check if OVMS is already running
    match check_ovms_status().await {
        Ok(ovms_status) => {
            log_operation_success!(
                "OVMS server already running",
                loaded_models = ?ovms_status.loaded_models
            );
            return Ok("OVMS server is already running".to_string());
        }
        Err(_) => {
            tracing::debug!("OVMS not running, proceeding with startup");
        }
    }

    let ovms_exe = paths::get_ovms_exe_path(Some(&app_handle))
        .map_err(|e| e.to_string())?;
    let config_path = paths::get_ovms_config_path(Some(&app_handle))
        .map_err(|e| e.to_string())?;

    // Validate config
    validate_ovms_config(&config_path)?;

    log_progress!("Launching OVMS process", 
        exe = %ovms_exe.display(),
        config = %config_path.display()
    );

    // Start OVMS process
    let mut cmd = Command::new(&ovms_exe);
    cmd.args([
        "--config_path",
        &config_path.to_string_lossy(),
        "--rest_port",
        "1114",
        "--log_level",
        "INFO",
    ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Hide console window on Windows
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd.spawn().map_err(|e| {
        log_operation_error!("OVMS process spawn", &e);
        format!("Failed to start OVMS: {}", e)
    })?;

    // Wait a moment for server to start
    tracing::debug!("Waiting for OVMS to initialize...");
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    // Check if process is still running before storing it
    match child.try_wait() {
        Ok(Some(status)) => {
            // Process exited
            let mut stderr_output = String::new();
            let mut stdout_output = String::new();

            if let Some(mut stderr) = child.stderr.take() {
                stderr.read_to_string(&mut stderr_output).unwrap_or_default();
            }

            if let Some(mut stdout) = child.stdout.take() {
                stdout.read_to_string(&mut stdout_output).unwrap_or_default();
            }

            let error_msg = format!(
                "OVMS exited with status: {}\nSTDOUT: {}\nSTDERR: {}\nConfig: {}\nExecutable: {}",
                status,
                stdout_output.trim(),
                stderr_output.trim(),
                config_path.display(),
                ovms_exe.display()
            );

            log_operation_error!("OVMS startup", &error_msg,
                exit_status = %status,
                config = %config_path.display(),
                executable = %ovms_exe.display()
            );
            Err(error_msg)
        }
        Ok(None) => {
            // Process is still running, store it globally
            // Scope the mutex guard properly to avoid Send issues
            {
                let process_mutex = OVMS_PROCESS.get_or_init(|| Arc::new(Mutex::new(None)));
                let mut process_guard = process_mutex.lock().unwrap();
                *process_guard = Some(child);
            } // Guard is dropped here

            log_operation_success!("OVMS server started on port 1114");

            Ok("OVMS server started successfully.".to_string())
        }
        Err(e) => { 
            log_operation_error!("OVMS status check", &e);
            Err(format!("Failed to check OVMS status: {}", e)) 
        }
    }
}

// Stop OVMS server
pub fn stop_ovms_server() -> Result<(), String> {
    log_operation_start!("Stopping OVMS server");
    
    let process_mutex = OVMS_PROCESS.get_or_init(|| Arc::new(Mutex::new(None)));
    let mut process_guard = process_mutex.lock().unwrap();

    if let Some(mut child) = process_guard.take() {
        tracing::debug!("Terminating OVMS process...");

        // Try to terminate gracefully first
        if let Err(e) = child.kill() {
            error!(error = %e, "Failed to kill OVMS process");
        }

        // Wait for the process to exit
        match child.wait() {
            Ok(status) => {
                info!(exit_status = ?status, "OVMS server stopped");
            }
            Err(e) => {
                error!(error = %e, "Error waiting for OVMS process to exit");
            }
        }
    } else {
        info!("No OVMS process was running");
    }

    // Also try the system-wide kill as fallback
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/IM", "ovms.exe", "/F"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("pkill").args(["-f", "ovms"]).output();
    }

    Ok(())
}

// Load a model into OVMS
#[tauri::command]
pub async fn load_model(app_handle: AppHandle, model_id: String) -> Result<String, String> {
    log_operation_start!("Loading model", model_id = %model_id);
    
    // Ensure we're working with an OpenVINO model
    let normalized_model_id = if model_id.starts_with("OpenVINO/") {
        model_id.clone()
    } else {
        format!("OpenVINO/{}", model_id)
    };

    // Get the model path
    let models_dir = paths::get_models_dir()
        .map_err(|e| e.to_string())?;

    // Build the path using the original model_id structure
    let original_model_id = if model_id.starts_with("OpenVINO") {
        model_id.clone()
    } else {
        format!("OpenVINO/{}", model_id)
    };

    let model_path = models_dir.join(&original_model_id);

    if !model_path.exists() {
        return Err(
            format!(
                "Model not found at: {}. Please download the model first.",
                model_path.display()
            )
        );
    }

    // Extract model name from the full ID
    let model_name = normalized_model_id.split('/').next_back().unwrap_or(&normalized_model_id);

    log_progress!("Updating OVMS configuration", model_name = %model_name);
    
    // Update OVMS config with the model
    update_ovms_config(
        app_handle.clone(),
        model_name.to_string(),
        model_path.to_string_lossy().to_string()
    ).await?;

    log_progress!("Reloading OVMS configuration");
    
    // Reload OVMS config to apply changes
    reload_ovms_config().await?;

    log_operation_success!("Model loaded", model_id = %normalized_model_id);
    Ok(format!("Model '{}' loaded successfully", normalized_model_id))
}



// Get the currently loaded model from config file
#[tauri::command]
pub async fn get_loaded_model(app_handle: AppHandle) -> Result<Option<String>, String> {
    let loaded_models = get_loaded_models(app_handle).await?;
    // Return the first loaded model, or None if no models are loaded
    Ok(loaded_models.into_iter().next())
}

#[tauri::command]
pub async fn check_ovms_status() -> Result<OvmsStatus, String> {
    let client = reqwest::Client::new();

    let response = client
        .get("http://localhost:1114/v1/config")
        .send().await
        .map_err(|e| format!("Failed to connect to OVMS server: {}", e))?;

    if response.status().is_success() {
        let body = response.text().await.map_err(|e| format!("Failed to read response: {}", e))?;

        // Parse the JSON response to extract loaded models
        let json_value: Value = serde_json
            ::from_str(&body)
            .map_err(|e| format!("Failed to parse OVMS response JSON: {}", e))?;

        let mut loaded_models = Vec::new();

        // Extract model names from the JSON structure
        if let Some(config_obj) = json_value.as_object() {
            for (key, value) in config_obj {
                // Skip metadata keys
                if key.starts_with("_") {
                    continue;
                }

                // Skip RAG models (Qwen3 embedding and reranker models)
                if key.starts_with("Qwen3") {
                    continue;
                }

                // Check if this is a model entry with model_version_status
                if let Some(model_info) = value.as_object() {
                    if let Some(version_status) = model_info.get("model_version_status") {
                        if let Some(status_array) = version_status.as_array() {
                            // Check if any version is AVAILABLE
                            let has_available = status_array.iter().any(|status| {
                                if let Some(status_obj) = status.as_object() {
                                    if let Some(state) = status_obj.get("state") {
                                        return state.as_str() == Some("AVAILABLE");
                                    }
                                }
                                false
                            });

                            if has_available {
                                loaded_models.push(key.clone());
                            }
                        }
                    }
                }
            }
        }

        Ok(OvmsStatus {
            status: "healthy".to_string(),
            loaded_models,
        })
    } else {
        Err(format!("OVMS status check failed with status: {}", response.status()))
    }
}

#[tauri::command]
pub async fn get_ovms_model_metadata(model_name: String) -> Result<String, String> {
    let client = reqwest::Client::new();

    // Try to get model metadata for more detailed error information
    let metadata_url = format!("http://localhost:1114/v1/models/{}/metadata", model_name);
    let response = client
        .get(&metadata_url)
        .send().await
        .map_err(|e| format!("Failed to get model metadata: {}", e))?;

    if response.status().is_success() {
        let body = response
            .text().await
            .map_err(|e| format!("Failed to read metadata response: {}", e))?;
        Ok(body)
    } else {
        // If metadata fails, try the model status endpoint
        let status_url = format!("http://localhost:1114/v1/models/{}", model_name);
        let status_response = client
            .get(&status_url)
            .send().await
            .map_err(|e| format!("Failed to get model status: {}", e))?;

        let status_code = status_response.status();
        let status_body = status_response
            .text().await
            .map_err(|e| format!("Failed to read status response: {}", e))?;

        if status_code.is_success() {
            Ok(status_body)
        } else {
            Err(format!("Model {} status check failed: {}", model_name, status_body))
        }
    }
}

#[allow(dead_code)]
pub fn generate_ovms_graph(model_dir: &PathBuf, model_id: &str) -> Result<(), String> {
    // Extract model name from ID (e.g., "OpenVINO/Phi-3.5-mini-instruct-int4-ov" -> "Phi-3.5-mini-instruct-int4-ov")
    let model_name = model_id.split('/').last().unwrap_or(model_id);

    // Check if we have OpenVINO IR files (.xml and .bin)
    let xml_files: Vec<_> = std::fs
        ::read_dir(model_dir)
        .map_err(|e| format!("Failed to read model directory: {}", e))?
        .filter_map(|entry| {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("xml") {
                    Some(path.file_stem().unwrap().to_string_lossy().to_string())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    if xml_files.is_empty() {
        return Err("No OpenVINO IR files (.xml) found in model directory".to_string());
    }

    // Check for tokenizer and detokenizer
    let tokenizer_name = xml_files
        .iter()
        .find(|name| name.contains("tokenizer") && !name.contains("detokenizer"));
    let detokenizer_name = xml_files.iter().find(|name| name.contains("detokenizer"));

    // Generate graph.pbtxt content based on model type
    let cache_dir = format!("{}/.ovms_cache", model_dir.to_string_lossy().replace('\\', "/"));
    let graph_content = if tokenizer_name.is_some() && detokenizer_name.is_some() {
        if model_name == "Qwen3-Reranker-0.6B-fp16-ov" {
            format!(
                r#"input_stream: "REQUEST_PAYLOAD:input"
output_stream: "RESPONSE_PAYLOAD:output"
node {{
  calculator: "OpenVINOModelServerSessionCalculator"
  output_side_packet: "SESSION:tokenizer"
  node_options: {{
    [type.googleapis.com / mediapipe.OpenVINOModelServerSessionCalculatorOptions]: {{
      servable_name: "tokenizer"
      servable_version: "1"
    }}
  }}
}}
node {{
  calculator: "OpenVINOModelServerSessionCalculator"
  output_side_packet: "SESSION:rerank"
  node_options: {{
    [type.googleapis.com / mediapipe.OpenVINOModelServerSessionCalculatorOptions]: {{
      servable_name: "rerank_model"
      servable_version: "1"
            }}
            }}
            }}
node {{
    input_side_packet: "TOKENIZER_SESSION:tokenizer"
    input_side_packet: "RERANK_SESSION:rerank"
    calculator: "RerankCalculator"
    input_stream: "REQUEST_PAYLOAD:input"
    output_stream: "RESPONSE_PAYLOAD:output"
            }}"#
            )
        } else if model_name == "Qwen3-Embedding-0.6B-int8-ov" {
            format!(
                r#"input_stream: "REQUEST_PAYLOAD:input"
output_stream: "RESPONSE_PAYLOAD:output"
node {{
  name: "EmbeddingsExecutor"
  input_side_packet: "EMBEDDINGS_NODE_RESOURCES:embeddings_servable"
  calculator: "EmbeddingsCalculatorOV"
  input_stream: "REQUEST_PAYLOAD:input"
  output_stream: "RESPONSE_PAYLOAD:output"
  node_options: {{
    [type.googleapis.com / mediapipe.EmbeddingsCalculatorOVOptions]: {{
      models_path: "./",
      normalize_embeddings: true,
      target_device: "GPU"
    }}
  }}
            }}"#
            )
        } else if model_name.ends_with("cw-ov") {
            format!(r#"input_stream: "HTTP_REQUEST_PAYLOAD:input"
                output_stream: "HTTP_RESPONSE_PAYLOAD:output"

                node: {{
                name: "LLMExecutor"
                calculator: "HttpLLMCalculator"
                input_stream: "LOOPBACK:loopback"
                input_stream: "HTTP_REQUEST_PAYLOAD:input"
                input_side_packet: "LLM_NODE_RESOURCES:llm"
                output_stream: "LOOPBACK:loopback"
                output_stream: "HTTP_RESPONSE_PAYLOAD:output"
                input_stream_info: {{
                    tag_index: 'LOOPBACK:0',
                    back_edge: true
                }}
                node_options: {{
                    [type.googleapis.com / mediapipe.LLMCalculatorOptions]: {{
                        models_path: "./",
                        plugin_config: '{{"CACHE_DIR": "{}"}}',
                        enable_prefix_caching: false,
                        cache_size: 2,
                        max_num_seqs: 256,
                        device: "NPU",
                    }}
                }}
                input_stream_handler {{
                    input_stream_handler: "SyncSetInputStreamHandler",
                    options {{
                    [mediapipe.SyncSetInputStreamHandlerOptions.ext] {{
                        sync_set {{
                        tag_index: "LOOPBACK:0"
                        }}
                    }}
                    }}
                }}
                }}
            "#, cache_dir)
        } else {
            format!(r#"input_stream: "HTTP_REQUEST_PAYLOAD:input"
                output_stream: "HTTP_RESPONSE_PAYLOAD:output"

                node: {{
                name: "LLMExecutor"
                calculator: "HttpLLMCalculator"
                input_stream: "LOOPBACK:loopback"
                input_stream: "HTTP_REQUEST_PAYLOAD:input"
                input_side_packet: "LLM_NODE_RESOURCES:llm"
                output_stream: "LOOPBACK:loopback"
                output_stream: "HTTP_RESPONSE_PAYLOAD:output"
                input_stream_info: {{
                    tag_index: 'LOOPBACK:0',
                    back_edge: true
                }}
                node_options: {{
                    [type.googleapis.com / mediapipe.LLMCalculatorOptions]: {{
                        models_path: "./",
                        plugin_config: '{{"CACHE_DIR": "{}"}}',
                        enable_prefix_caching: false,
                        cache_size: 2,
                        max_num_seqs: 256,
                        max_num_batched_tokens: 8192,
                        device: "GPU",
                    }}
                }}
                input_stream_handler {{
                    input_stream_handler: "SyncSetInputStreamHandler",
                    options {{
                    [mediapipe.SyncSetInputStreamHandlerOptions.ext] {{
                        sync_set {{
                        tag_index: "LOOPBACK:0"
                        }}
                    }}
                    }}
                }}
                }}
            "#, cache_dir)
        }
    } else {
        format!(
            r#"input_stream: "REQUEST_PAYLOAD:input"
output_stream: "RESPONSE_PAYLOAD:output"
node {{
    name: "LLMExecutor"
    calculator: "LLMCalculator"
    input_stream: "REQUEST_PAYLOAD:input"
    output_stream: "RESPONSE_PAYLOAD:output"
    input_side_packet: "LLM_NODE_RESOURCES:llm"
    node_options: {{
        [type.googleapis.com / mediapipe.LLMCalculatorOptions]: {{
            models_path: "./",
            target_device: "GPU"
        }}
    }}
}}"#
        )
    };

    let graph_path = model_dir.join("graph.pbtxt");
    std::fs
        ::write(&graph_path, graph_content)
        .map_err(|e| format!("Failed to write graph.pbtxt: {}", e))?;

    // Only print if model graph generation is successful
    println!("graph.pbtxt generated for model: {} at {}", model_name, graph_path.display());

    Ok(())
}
