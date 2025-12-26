use serde::{ Deserialize, Serialize };
use serde_json;
use tracing::{ info, warn, error };
use std::path::PathBuf;
use tauri::Emitter;
use tokio::io::AsyncWriteExt;
use std::fs;
use std::collections::HashMap;

use crate::{ constants, paths };

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub enum ModelType {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "image-to-text")]
    ImageToText,
    #[serde(rename = "embedding")]
    Embedding,
    #[serde(rename = "reranker")]
    Reranker,
    #[serde(rename = "image-generation")]
    ImageGeneration,
    #[serde(rename = "speech-to-text")]
    SpeechToText,
    #[serde(rename = "text-to-speech")]
    TextToSpeech,
}

impl ModelType {
    pub fn as_str(&self) -> &str {
        match self {
            ModelType::Text => "text",
            ModelType::ImageToText => "image-to-text",
            ModelType::Embedding => "embedding",
            ModelType::Reranker => "reranker",
            ModelType::ImageGeneration => "image-generation",
            ModelType::SpeechToText => "speech-to-text",
            ModelType::TextToSpeech => "text-to-speech",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelMetadata {
    pub model_id: String,
    pub model_type: ModelType,
    pub pipeline_tag: String,
    pub commit_sha: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ModelMetadataStore {
    models: HashMap<String, ModelMetadata>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelSibling {
    pub rfilename: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub author: Option<String>,
    pub sha: Option<String>,
    pub pipeline_tag: Option<String>,
    pub tags: Vec<String>,
    pub downloads: Option<u64>,
    pub likes: Option<u64>,
    pub created_at: Option<String>,
    pub last_modified: Option<String>,
    pub collections: Option<Vec<String>>,
    pub siblings: Vec<ModelSibling>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub model_ids: Vec<String>,
    pub total_count: Option<u64>,
}

// Hugging Face API response structures
#[derive(Debug, Deserialize)]
struct HfCardData {
    pub collections: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct HfModelSibling {
    pub rfilename: String,
}

#[derive(Debug, Deserialize)]
struct HfModelInfo {
    pub id: String,
    pub author: Option<String>,
    pub sha: Option<String>,
    #[serde(rename = "pipeline-tag")]
    pub pipeline_tag: Option<String>,
    #[serde(rename = "pipeline_tag")]
    pub pipeline_tag_alt: Option<String>,  // Handle both formats
    pub tags: Option<Vec<String>>,
    pub downloads: Option<u64>,
    pub likes: Option<u64>,
    // The search API uses "createdAt" instead of "created_at"
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(rename = "lastModified")]
    pub last_modified: Option<String>,
    #[serde(rename = "last_modified")]
    pub last_modified_alt: Option<String>,  // Handle snake_case format
    #[serde(rename = "cardData")]
    pub card_data: Option<HfCardData>,
    pub siblings: Option<Vec<HfModelSibling>>,
}

#[derive(Debug, Deserialize)]
struct HfFileInfo {
    #[serde(rename = "path")]
    pub path: String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub file_type: String,
    #[allow(dead_code)]
    pub size: Option<u64>,
}

// Memory-efficient streaming file download
async fn download_single_file(
    client: &reqwest::Client,
    file_url: &str,
    target_dir: &PathBuf,
    file_info: &HfFileInfo,
    model_id: &str,
    file_index: usize,
    total_files: usize,
    total_downloaded_so_far: u64,
    total_estimated_size: u64,
    app: &tauri::AppHandle
) -> Result<u64, String> {
    use futures::StreamExt;

    // Create subdirectories if needed (async)
    let target_file = target_dir.join(&file_info.path);
    if let Some(parent) = target_file.parent() {
        tokio::fs
            ::create_dir_all(parent).await
            .map_err(|e| format!("Failed to create directory for {}: {}", file_info.path, e))?;
    }

    // Start the request
    let response = client
        .get(file_url)
        .header("User-Agent", constants::USER_AGENT)
        .send().await
        .map_err(|e| {
            log_operation_error!("File download", &e, file = %file_info.path, model_id = %model_id);
            format!("Request failed: {}", e)
        })?;

    if !response.status().is_success() {
        let status = response.status();
        log_operation_error!("File download", &format!("HTTP {}", status), file = %file_info.path, model_id = %model_id);
        return Err(format!("HTTP error {}", status));
    }

    // Get content length for progress tracking
    let content_length = response.content_length().unwrap_or(0);

    // Create the file
    let mut file = tokio::fs::File
        ::create(&target_file).await
        .map_err(|e| {
            log_operation_error!("File creation", &e, file = %file_info.path, model_id = %model_id);
            format!("Failed to create file: {}", e)
        })?;

    // Stream the response body in chunks to avoid loading entire file into memory
    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;
    let mut last_progress_emit = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Failed to read chunk: {}", e))?;

        // Write chunk to file
        file.write_all(&chunk).await.map_err(|e| format!("Failed to write chunk: {}", e))?;

        downloaded += chunk.len() as u64;

        // Emit progress events, but not too frequently to avoid overwhelming the UI
        if last_progress_emit.elapsed().as_millis() > constants::DOWNLOAD_PROGRESS_INTERVAL_MS || downloaded == content_length {
            let file_progress = if content_length > 0 {
                (((downloaded as f64) / (content_length as f64)) * 100.0) as u32
            } else {
                0
            };

            // Calculate overall progress based on total downloaded bytes across all files
            let total_downloaded_bytes = total_downloaded_so_far + downloaded;
            let overall_progress = if total_estimated_size > 0 {
                (((total_downloaded_bytes as f64) / (total_estimated_size as f64)) * 100.0) as u32
            } else {
                // Fallback to file-based progress if no size info
                (((file_index as f64) / (total_files as f64)) * 100.0) as u32
            };

            let _ = app.emit(
                "download-progress",
                serde_json::json!({
                "modelId": model_id,
                "progress": overall_progress.min(100), // Cap at 100%
                "currentFile": file_info.path,
                "fileIndex": file_index,
                "totalFiles": total_files,
                "fileProgress": file_progress,
                "downloadedBytes": total_downloaded_bytes,
                "totalBytes": total_estimated_size,
                "currentFileDownloaded": downloaded,
                "currentFileTotal": content_length
            })
            );

            last_progress_emit = std::time::Instant::now();
        }

        // Add a small yield to prevent blocking the async runtime
        tokio::task::yield_now().await;
    }

    // Ensure all data is written to disk
    file.flush().await.map_err(|e| {
        log_operation_error!("File flush", &e, file = %file_info.path, model_id = %model_id);
        format!("Failed to flush file: {}", e)
    })?;

    tracing::debug!(
        file = %file_info.path,
        bytes = downloaded,
        model_id = %model_id,
        "File downloaded successfully"
    );

    Ok(downloaded)
}

#[tauri::command]
pub async fn search_models(query: String, limit: Option<u32>) -> Result<SearchResult, String> {
    log_operation_start!("Model search");
    
    let client = reqwest::Client::new();
    let search_limit = limit.unwrap_or(constants::DEFAULT_MODEL_SEARCH_LIMIT).min(constants::MAX_MODEL_SEARCH_LIMIT);

    // Search specifically under OpenVINO organization
    let search_query = if query.trim().is_empty() {
        constants::OPENVINO_ORG.to_string()
    } else {
        format!("{}/{}", constants::OPENVINO_ORG, query)
    };

    tracing::debug!(query = %search_query, limit = search_limit, org = constants::OPENVINO_ORG, "Searching HuggingFace models");

    let url = format!(
        "{}/models?search={}&limit={}&author={}",
        constants::HUGGINGFACE_API_BASE,
        urlencoding::encode(&search_query),
        search_limit,
        constants::OPENVINO_ORG
    );

    let response = client
        .get(&url)
        .header("User-Agent", "SparrowAI/1.0")
        .send().await
        .map_err(|e| {
            log_operation_error!("Model search", &e);
            format!("Failed to send request: {}", e)
        })?;

    if !response.status().is_success() {
        let status = response.status();
        log_operation_error!("Model search", &format!("API returned status {}", status));
        return Err(format!("API request failed with status: {}", status));
    }

    let hf_models: Vec<HfModelInfo> = response
        .json().await
        .map_err(|e| {
            log_operation_error!("Model search", &format!("JSON parse failed: {}", e));
            format!("Failed to parse JSON: {}", e)
        })?;

    // Filter to only include OpenVINO models and optionally filter by query
    let model_ids: Vec<String> = hf_models
        .into_iter()
        .filter(|hf_model| {
            // Ensure the model is from OpenVINO organization
            hf_model.id.starts_with("OpenVINO/") &&
                // If there's a specific query, check if the model name contains it
                (query.trim().is_empty() ||
                    hf_model.id.to_lowercase().contains(&query.to_lowercase()))
        })
        .map(|hf_model| hf_model.id)
        .collect();

    let total_count = model_ids.len() as u64;

    log_operation_success!("Model search");
    tracing::debug!(count = total_count, query = %query, "Found models");

    Ok(SearchResult {
        model_ids,
        total_count: Some(total_count),
    })
}

#[tauri::command]
pub async fn get_model_info(model_id: String) -> Result<ModelInfo, String> {
    log_operation_start!("Get model info");
    
    let client = reqwest::Client::new();

    // Ensure we're getting info for an OpenVINO model
    let normalized_model_id = if model_id.starts_with("OpenVINO/") {
        model_id
    } else {
        format!("OpenVINO/{}", model_id)
    };

    tracing::debug!(model_id = %normalized_model_id, "Fetching model info from HuggingFace");

    // Don't encode the model ID - HuggingFace API expects it as-is in the path
    let url = format!(
        "https://huggingface.co/api/models/{}",
        normalized_model_id
    );

    let response = client
        .get(&url)
        .header("User-Agent", "SparrowAI/1.0")
        .send().await
        .map_err(|e| {
            log_operation_error!("Get model info", &e, model_id = %normalized_model_id);
            format!("Failed to send request: {}", e)
        })?;

    if !response.status().is_success() {
        let status = response.status();
        log_operation_error!("Get model info", &format!("API returned status {}", status), model_id = %normalized_model_id);
        return Err(
            format!(
                "API request failed with status: {}. Make sure the model exists under OpenVINO organization.",
                status
            )
        );
    }

    let hf_model: HfModelInfo = response
        .json().await
        .map_err(|e| {
            log_operation_error!("Get model info", &format!("JSON parse failed: {}", e), model_id = %normalized_model_id);
            format!("Failed to parse JSON: {}", e)
        })?;

    // Verify this is actually an OpenVINO model
    if !hf_model.id.starts_with("OpenVINO/") {
        log_operation_error!("Get model info", "Model not from OpenVINO organization", model_id = %hf_model.id);
        return Err(format!("Model {} is not from OpenVINO organization", hf_model.id));
    }

    // Extract collections from cardData
    let collections = hf_model.card_data
        .and_then(|card| card.collections);

    // Extract siblings (files in the repository)
    let siblings: Vec<ModelSibling> = hf_model.siblings
        .unwrap_or_default()
        .into_iter()
        .map(|s| ModelSibling { rfilename: s.rfilename })
        .collect();

    log_operation_success!("Get model info");
    tracing::debug!(model_id = %hf_model.id, files = siblings.len(), "Retrieved model info");

    // Handle both API formats for pipeline_tag
    let pipeline_tag = hf_model.pipeline_tag.or(hf_model.pipeline_tag_alt);
    
    // Handle both API formats for last_modified
    let last_modified = hf_model.last_modified.or(hf_model.last_modified_alt);

    let model_info = ModelInfo {
        id: hf_model.id,
        author: hf_model.author,
        sha: hf_model.sha,
        pipeline_tag,
        tags: hf_model.tags.unwrap_or_default(),
        downloads: hf_model.downloads,
        likes: hf_model.likes,
        created_at: hf_model.created_at,
        last_modified,
        collections,
        siblings,
    };

    info!(
        model_id = %model_info.id,
        pipeline_tag = ?model_info.pipeline_tag,
        collections = ?model_info.collections,
        downloads = ?model_info.downloads,
        siblings_count = %model_info.siblings.len(),
        "Fetched model info"
    );

    Ok(model_info)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelUpdateInfo {
    pub model_id: String,
    pub is_latest: bool,
    pub local_commit: Option<String>,
    pub remote_commit: Option<String>,
    pub needs_update: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphGenerationParams {
    // Task type (text_generation, embeddings_ov, rerank_ov, etc.)
    pub task_type: Option<String>,
    
    // Common parameters
    pub target_device: Option<String>, // CPU, GPU, NPU, AUTO
    
    // Text generation specific
    pub enable_prefix_caching: Option<bool>,
    pub cache_size: Option<u32>,
    pub max_num_seqs: Option<u32>,
    pub kv_cache_precision: Option<String>,
    pub max_num_batched_tokens: Option<u32>,
    pub dynamic_split_fuse: Option<bool>,
    pub pipeline_type: Option<String>,
    
    // Embeddings specific
    pub normalize: Option<bool>,
    pub pooling: Option<String>,
    pub truncate: Option<bool>,
    
    // Common stream parameter
    pub num_streams: Option<u32>,
    
    // Image generation specific
    pub resolution: Option<String>,
    pub guidance_scale: Option<String>,
    pub num_images_per_prompt: Option<String>,
    pub max_resolution: Option<String>,
    pub default_resolution: Option<String>,
    pub max_num_images_per_prompt: Option<u32>,
    pub default_num_inference_steps: Option<u32>,
    pub max_num_inference_steps: Option<u32>,
}

// Load model metadata from disk
async fn load_model_metadata() -> Result<ModelMetadataStore, String> {
    let metadata_path = paths::get_model_metadata_path()
        .map_err(|e| e.to_string())?;
    
    if !metadata_path.exists() {
        return Ok(ModelMetadataStore::default());
    }
    
    let content = tokio::fs::read_to_string(&metadata_path)
        .await
        .map_err(|e| format!("Failed to read metadata file: {}", e))?;
    
    let store: ModelMetadataStore = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse metadata file: {}", e))?;
    
    Ok(store)
}

// Save model metadata to disk
async fn save_model_metadata(store: &ModelMetadataStore) -> Result<(), String> {
    let metadata_path = paths::get_model_metadata_path()
        .map_err(|e| e.to_string())?;
    
    // Ensure parent directory exists
    if let Some(parent) = metadata_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create metadata directory: {}", e))?;
    }
    
    let content = serde_json::to_string_pretty(store)
        .map_err(|e| format!("Failed to serialize metadata: {}", e))?;
    
    tokio::fs::write(&metadata_path, content)
        .await
        .map_err(|e| format!("Failed to write metadata file: {}", e))?;
    
    Ok(())
}

// Add or update model metadata
async fn save_model_type(model_id: String, model_type: ModelType, pipeline_tag: String, commit_sha: Option<String>) -> Result<(), String> {
    let mut store = load_model_metadata().await?;
    
    let metadata = ModelMetadata {
        model_id: model_id.clone(),
        model_type,
        pipeline_tag,
        commit_sha,
    };
    
    store.models.insert(model_id, metadata);
    save_model_metadata(&store).await?;
    
    Ok(())
}

// Get model type from metadata
pub async fn get_model_type(model_id: &str) -> Result<Option<ModelType>, String> {
    let store = load_model_metadata().await?;
    Ok(store.models.get(model_id).map(|m| m.model_type.clone()))
}

// Remove model from metadata
pub async fn remove_model_metadata(model_id: &str) -> Result<(), String> {
    let mut store = load_model_metadata().await?;
    
    if store.models.remove(model_id).is_some() {
        save_model_metadata(&store).await?;
        tracing::info!(model_id = %model_id, "Removed model from metadata");
    } else {
        tracing::debug!(model_id = %model_id, "Model not found in metadata");
    }
    
    Ok(())
}

// Get all models grouped by type
#[tauri::command]
pub async fn get_models_by_type() -> Result<HashMap<String, Vec<String>>, String> {
    let store = load_model_metadata().await?;
    
    let mut models_by_type: HashMap<String, Vec<String>> = HashMap::new();
    
    for (model_id, metadata) in store.models.iter() {
        let type_key = metadata.model_type.as_str().to_string();
        models_by_type
            .entry(type_key)
            .or_insert_with(Vec::new)
            .push(model_id.clone());
    }
    
    Ok(models_by_type)
}

// Get all model metadata
#[tauri::command]
pub async fn get_all_model_metadata() -> Result<HashMap<String, ModelMetadata>, String> {
    let store = load_model_metadata().await?;
    Ok(store.models)
}

// Manually set model type for a model (useful for existing models or manual corrections)
#[tauri::command]
pub async fn set_model_type(model_id: String, model_type_str: String) -> Result<(), String> {
    let model_type = match model_type_str.as_str() {
        "text" => ModelType::Text,
        "image-to-text" => ModelType::ImageToText,
        "embedding" => ModelType::Embedding,
        "reranker" => ModelType::Reranker,
        "image-generation" => ModelType::ImageGeneration,
        "speech-to-text" => ModelType::SpeechToText,
        "text-to-speech" => ModelType::TextToSpeech,
        _ => return Err(format!("Invalid model type: {}", model_type_str)),
    };
    
    save_model_type(model_id, model_type, String::new(), None).await
}

// Initialize metadata for all downloaded models by fetching from HuggingFace
#[tauri::command]
pub async fn initialize_model_metadata(models_dir: Option<String>) -> Result<String, String> {
    log_operation_start!("Initialize model metadata");
    
    let downloads_dir = if let Some(path) = models_dir {
        PathBuf::from(path)
    } else {
        paths::get_models_dir()
            .map_err(|e| e.to_string())?
    };

    if !downloads_dir.exists() {
        return Err("Models directory does not exist".to_string());
    }

    // Look for OpenVINO models
    let openvino_dir = downloads_dir.join("OpenVINO");
    if !openvino_dir.exists() {
        return Ok("No OpenVINO models found".to_string());
    }

    let mut initialized_count = 0;
    let mut failed_count = 0;

    // Iterate through model directories
    let entries = fs::read_dir(&openvino_dir)
        .map_err(|e| format!("Failed to read OpenVINO directory: {}", e))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let model_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        let model_id = format!("OpenVINO/{}", model_name);

        // Check if metadata already exists
        if let Ok(Some(_)) = get_model_type(&model_id).await {
            tracing::debug!(model_id = %model_id, "Metadata already exists, skipping");
            continue;
        }

        // Fetch model info from HuggingFace
        match get_model_info(model_id.clone()).await {
            Ok(model_info) => {
                if let Some(pipeline_tag) = &model_info.pipeline_tag {
                    if let Some(model_type) = map_pipeline_tag_to_model_type(pipeline_tag) {
                        match save_model_type(model_id.clone(), model_type, pipeline_tag.clone(), model_info.sha.clone()).await {
                            Ok(_) => {
                                tracing::info!(model_id = %model_id, pipeline_tag = %pipeline_tag, "Initialized metadata");
                                initialized_count += 1;
                            }
                            Err(e) => {
                                tracing::error!(model_id = %model_id, error = %e, "Failed to save metadata");
                                failed_count += 1;
                            }
                        }
                    } else {
                        tracing::warn!(model_id = %model_id, pipeline_tag = %pipeline_tag, "Unknown pipeline_tag, skipping");
                        failed_count += 1;
                    }
                } else {
                    tracing::warn!(model_id = %model_id, "No pipeline_tag found, skipping");
                    failed_count += 1;
                }
            }
            Err(e) => {
                tracing::error!(model_id = %model_id, error = %e, "Failed to fetch model info");
                failed_count += 1;
            }
        }
    }

    let message = if failed_count > 0 {
        format!("Initialized metadata for {} models ({} failed)", initialized_count, failed_count)
    } else {
        format!("Initialized metadata for {} models", initialized_count)
    };

    log_operation_success!("Initialize model metadata", count = initialized_count);
    Ok(message)
}

// Map HuggingFace pipeline_tag to our ModelType
fn map_pipeline_tag_to_model_type(pipeline_tag: &str) -> Option<ModelType> {
    match pipeline_tag {
        "text-generation" => Some(ModelType::Text),
        "image-text-to-text" => Some(ModelType::ImageToText),
        "feature-extraction" => Some(ModelType::Embedding),
        "reranking" => Some(ModelType::Reranker),
        "text-to-image" => Some(ModelType::ImageGeneration),
        "automatic-speech-recognition" => Some(ModelType::SpeechToText),
        "text-to-speech" => Some(ModelType::TextToSpeech),
        _ => None,
    }
}

// Get commit SHA from metadata
async fn get_commit_sha_from_metadata(model_id: &str) -> Option<String> {
    if let Ok(store) = load_model_metadata().await {
        store.models.get(model_id).and_then(|m| m.commit_sha.clone())
    } else {
        None
    }
}

#[tauri::command]
pub async fn check_model_update_status(
    model_id: String,
    models_dir: Option<String>
) -> Result<ModelUpdateInfo, String> {
    // Ensure we're checking an OpenVINO model
    let normalized_model_id = if model_id.starts_with("OpenVINO/") {
        model_id
    } else {
        format!("OpenVINO/{}", model_id)
    };

    // Determine model directory
    let model_dir = if let Some(dir) = models_dir {
        PathBuf::from(dir).join(&normalized_model_id)
    } else {
        paths::get_models_dir()
            .map_err(|e| e.to_string())?
            .join(&normalized_model_id)
    };

    // Check if model directory exists
    if !model_dir.exists() {
        return Err(format!("Model directory not found: {}", model_dir.to_string_lossy()));
    }

    // Read local commit SHA from metadata
    let local_commit = get_commit_sha_from_metadata(&normalized_model_id).await;

    // Get remote model info to check latest commit
    let remote_model_info = get_model_info(normalized_model_id.clone()).await?;
    let remote_commit = remote_model_info.sha;

    // Determine if update is needed
    let needs_update = match (&local_commit, &remote_commit) {
        (Some(local), Some(remote)) => local != remote,
        (None, Some(_)) => true, // No local commit info, assume update needed
        (Some(_), None) => false, // Remote has no commit info, assume local is fine
        (None, None) => false, // Neither has commit info, assume no update needed
    };

    let is_latest = !needs_update;

    Ok(ModelUpdateInfo {
        model_id: normalized_model_id,
        is_latest,
        local_commit,
        remote_commit,
        needs_update,
    })
}

#[tauri::command]
pub async fn download_entire_model(
    model_id: String,
    download_path: Option<String>,
    graph_params: Option<GraphGenerationParams>,
    app: tauri::AppHandle
) -> Result<String, String> {
    // Ensure we're downloading an OpenVINO model
    let normalized_model_id = if model_id.starts_with("OpenVINO/") {
        model_id
    } else {
        format!("OpenVINO/{}", model_id)
    };

    log_operation_start!("Model download", model_id = %normalized_model_id);

    // Get model info first to retrieve commit SHA
    let model_info = get_model_info(normalized_model_id.clone()).await.map_err(|e| {
        log_operation_error!("Get model info", &e, model_id = %normalized_model_id);
        e
    })?;

    // Create a client with timeout to prevent hanging
    let client = reqwest::Client
        ::builder()
        .timeout(std::time::Duration::from_secs(300)) // 5 minute timeout per request
        .build()
        .map_err(|e| {
            log_operation_error!("HTTP client creation", &e);
            format!("Failed to create HTTP client: {}", e)
        })?;

    let target_dir = if let Some(path) = download_path {
        PathBuf::from(path).join(&normalized_model_id)
    } else {
        // Use .sparrow/models as default
        paths::get_models_dir()
            .map_err(|e| {
                log_operation_error!("Get models directory", &e);
                e.to_string()
            })?
            .join(&normalized_model_id)
    };

    // Create target directory
    std::fs::create_dir_all(&target_dir).map_err(|e| {
        log_operation_error!("Create directory", &e, dir = %target_dir.display());
        format!("Failed to create directory: {}", e)
    })?;

    // Use siblings from model_info instead of making a separate API call
    let downloadable_files: Vec<&ModelSibling> = model_info.siblings
        .iter()
        .filter(|sibling| !sibling.rfilename.is_empty())
        .collect();

    if downloadable_files.is_empty() {
        log_operation_error!("Model download", "No files found in repository", 
            model_id = %normalized_model_id
        );
        return Err("No files found in model repository".to_string());
    }

    let total_files = downloadable_files.len();
    
    log_progress!("Downloading model files", 
        model_id = %normalized_model_id,
        total_files = total_files
    );

    let mut downloaded_files = Vec::new();
    let mut errors = Vec::new();
    let mut total_downloaded_size = 0u64;

    for (index, sibling) in downloadable_files.iter().enumerate() {
        // Don't encode model ID or file path - they're part of the URL path
        let file_url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            normalized_model_id,
            sibling.rfilename
        );

        // Create temporary HfFileInfo for compatibility with download_single_file
        let file_info = HfFileInfo {
            path: sibling.rfilename.clone(),
            file_type: "file".to_string(),
            size: None,  // We don't have size info from siblings
        };

        // Emit progress update
        let current_progress = ((index as f64 / total_files as f64) * 100.0) as u32;
        let _ = app.emit(
            "download-progress",
            serde_json::json!({
                "modelId": normalized_model_id,
                "progress": current_progress,
                "currentFile": sibling.rfilename,
                "fileIndex": index + 1,
                "totalFiles": total_files,
                "fileProgress": 0,
            })
        );

        // Download the file
        let download_result = download_single_file(
            &client,
            &file_url,
            &target_dir,
            &file_info,
            &normalized_model_id,
            index + 1,
            total_files,
            total_downloaded_size,
            0,  // No total size estimate available
            &app
        ).await;

        match download_result {
            Ok(file_size) => {
                downloaded_files.push(sibling.rfilename.clone());
                total_downloaded_size += file_size;
            }
            Err(e) => {
                let error_msg = format!("Failed to download {}: {}", sibling.rfilename, e);
                error!(error = %error_msg, "Model download failed");
                errors.push(error_msg);

                // Continue with other files instead of crashing
                continue;
            }
        }
    }

    if downloaded_files.is_empty() {
        let error_details = if errors.is_empty() {
            "No files could be downloaded from the repository.".to_string()
        } else {
            format!("Download errors occurred:\n{}", errors.join("\n"))
        };
        return Err(format!("Failed to download model files. {}", error_details));
    }

    let total_size_mb = (total_downloaded_size as f64) / (1024.0 * 1024.0);
    let success_msg = format!(
        "Successfully downloaded {} files ({:.2} MB) to:\n{}\n\nDownloaded files:\n• {}",
        downloaded_files.len(),
        total_size_mb,
        target_dir.to_string_lossy(),
        downloaded_files.join("\n• ")
    );

    // Generate graph.pbtxt for OVMS compatibility
    // Use user-selected task type from params if provided, otherwise detect from model info
    let task_type = if let Some(params) = &graph_params {
        params.task_type.clone().or_else(|| detect_task_type(&model_info))
    } else {
        detect_task_type(&model_info)
    };
    
    // Save model type metadata - always save even if we can't determine the type
    let save_result = if let Some(pipeline_tag) = &model_info.pipeline_tag {
        if let Some(model_type) = map_pipeline_tag_to_model_type(pipeline_tag) {
            // We have a recognized pipeline_tag, save with type
            save_model_type(
                normalized_model_id.clone(), 
                model_type, 
                pipeline_tag.clone(),
                model_info.sha.clone()
            ).await
        } else {
            // Unknown pipeline_tag, but still save with Text as default
            warn!(
                model_id = %normalized_model_id,
                pipeline_tag = %pipeline_tag,
                "Unknown pipeline_tag, saving as Text type"
            );
            save_model_type(
                normalized_model_id.clone(), 
                ModelType::Text, 
                pipeline_tag.clone(),
                model_info.sha.clone()
            ).await
        }
    } else {
        // No pipeline_tag, save with Text as default
        warn!(
            model_id = %normalized_model_id,
            "No pipeline_tag found, saving as Text type"
        );
        save_model_type(
            normalized_model_id.clone(), 
            ModelType::Text, 
            String::new(),
            model_info.sha.clone()
        ).await
    };
    
    match save_result {
        Ok(_) => {
            info!(
                model_id = %normalized_model_id,
                pipeline_tag = ?model_info.pipeline_tag,
                commit_sha = ?model_info.sha,
                "Saved model metadata"
            );
        }
        Err(e) => {
            error!(
                error = %e,
                model_id = %normalized_model_id,
                "Failed to save model metadata - this will affect model updates and type tracking"
            );
        }
    }
    
    if let Some(task_type) = task_type {
        info!(
            model_id = %normalized_model_id,
            task_type = %task_type,
            source = if graph_params.as_ref().and_then(|p| p.task_type.as_ref()).is_some() { "user_selected" } else { "auto_detected" },
            "Using task type for graph generation"
        );
        
        if let Err(e) = generate_graph_for_task(&task_type, &target_dir, &normalized_model_id, graph_params.as_ref()) {
            warn!(
                error = %e,
                task_type = %task_type,
                "Failed to generate graph.pbtxt"
            );
        }
    } else {
        warn!(
            model_id = %normalized_model_id,
            pipeline_tag = ?model_info.pipeline_tag,
            "Could not detect model task type, skipping graph.pbtxt generation"
        );
    }

    if !errors.is_empty() {
        Ok(
            format!(
                "{}\n\n⚠️ Some files had issues ({} errors):\n{}",
                success_msg,
                errors.len(),
                errors.join("\n")
            )
        )
    } else {
        Ok(success_msg)
    }
}

/// Check if the required RAG models (embedding and reranker) are downloaded
#[tauri::command]
pub async fn check_rag_models_exist(download_path: Option<String>) -> Result<bool, String> {
    let downloads_dir = if let Some(path) = download_path {
        PathBuf::from(path)
    } else {
        // Use .sparrow/models as default
        paths::get_models_dir()
            .map_err(|e| {
                log_operation_error!("Check RAG models", &e);
                e.to_string()
            })?
    };

    // Check for both RAG models (Qwen3 embedding and reranker)
    let embedding_model_path = downloads_dir.join("OpenVINO").join("Qwen3-Embedding-0.6B-int8-ov");
    let reranker_model_path = downloads_dir.join("OpenVINO").join("Qwen3-Reranker-0.6B-fp16-ov");

    let embedding_exists = embedding_model_path.exists() && embedding_model_path.is_dir();
    let reranker_exists = reranker_model_path.exists() && reranker_model_path.is_dir();

    tracing::debug!(
        embedding_exists = embedding_exists,
        reranker_exists = reranker_exists,
        path = %downloads_dir.display(),
        "Checked RAG models existence"
    );

    Ok(embedding_exists && reranker_exists)
}

// Graph templates for different task types
const TEXT_GENERATION_GRAPH_TEMPLATE: &str = r#"input_stream: "HTTP_REQUEST_PAYLOAD:input"
output_stream: "HTTP_RESPONSE_PAYLOAD:output"

node: {
  name: "LLMExecutor"
  calculator: "HttpLLMCalculator"
  input_stream: "LOOPBACK:loopback"
  input_stream: "HTTP_REQUEST_PAYLOAD:input"
  input_side_packet: "LLM_NODE_RESOURCES:llm"
  output_stream: "LOOPBACK:loopback"
  output_stream: "HTTP_RESPONSE_PAYLOAD:output"

  input_stream_info: {
    tag_index: 'LOOPBACK:0',
    back_edge: true
  }
  node_options: {
      [type.googleapis.com / mediapipe.LLMCalculatorOptions]: {
          {{pipeline_type}}models_path: "./",
          plugin_config: '{{plugin_config}}',
          enable_prefix_caching: {{enable_prefix_caching}},
          cache_size: {{cache_size}},
          {{max_num_batched_tokens}}{{dynamic_split_fuse}}max_num_seqs: {{max_num_seqs}},
          device: "{{target_device}}",
          {{draft_models_path}}{{reasoning_parser}}{{tool_parser}}{{enable_tool_guided_generation}}
      }
  }
  input_stream_handler {
    input_stream_handler: "SyncSetInputStreamHandler",
    options {
      [mediapipe.SyncSetInputStreamHandlerOptions.ext] {
        sync_set {
          tag_index: "LOOPBACK:0"
        }
      }
    }
  }
}"#;

const EMBEDDINGS_OV_GRAPH_TEMPLATE: &str = r#"input_stream: "REQUEST_PAYLOAD:input"
output_stream: "RESPONSE_PAYLOAD:output"
node {
  name: "EmbeddingsExecutor"
  input_side_packet: "EMBEDDINGS_NODE_RESOURCES:embeddings_servable"
  calculator: "EmbeddingsCalculatorOV"
  input_stream: "REQUEST_PAYLOAD:input"
  output_stream: "RESPONSE_PAYLOAD:output"
  node_options: {
    [type.googleapis.com / mediapipe.EmbeddingsCalculatorOVOptions]: {
      models_path: "./",
      plugin_config: '{"NUM_STREAMS": "{{num_streams}}" }',
      normalize_embeddings: {{normalize}},
      {{pooling}}{{truncate}}target_device: "{{target_device}}"
    }
  }
}"#;

const RERANK_OV_GRAPH_TEMPLATE: &str = r#"input_stream: "REQUEST_PAYLOAD:input"
output_stream: "RESPONSE_PAYLOAD:output"
node {
  name: "RerankExecutor"
  input_side_packet: "RERANK_NODE_RESOURCES:rerank_servable"
  calculator: "RerankCalculatorOV"
  input_stream: "REQUEST_PAYLOAD:input"
  output_stream: "RESPONSE_PAYLOAD:output"
  node_options: {
    [type.googleapis.com / mediapipe.RerankCalculatorOVOptions]: {
      models_path: "./",
      plugin_config: '{"NUM_STREAMS": "{{num_streams}}" }',
      target_device: "{{target_device}}"
    }
  }
}"#;

const TEXT2SPEECH_GRAPH_TEMPLATE: &str = r#"input_stream: "HTTP_REQUEST_PAYLOAD:input"
output_stream: "HTTP_RESPONSE_PAYLOAD:output"
node {
  name: "T2sExecutor"
  input_side_packet: "TTS_NODE_RESOURCES:t2s_servable"
  calculator: "T2sCalculator"
  input_stream: "HTTP_REQUEST_PAYLOAD:input"
  output_stream: "HTTP_RESPONSE_PAYLOAD:output"
  node_options: {
    [type.googleapis.com / mediapipe.T2sCalculatorOptions]: {
      models_path: "./",
      plugin_config: '{ "NUM_STREAMS": "{{num_streams}}" }',
      target_device: "{{target_device}}"
    }
  }
}"#;

const SPEECH2TEXT_GRAPH_TEMPLATE: &str = r#"input_stream: "HTTP_REQUEST_PAYLOAD:input"
output_stream: "HTTP_RESPONSE_PAYLOAD:output"
node {
  name: "S2tExecutor"
  input_side_packet: "STT_NODE_RESOURCES:s2t_servable"
  calculator: "S2tCalculator"
  input_stream: "HTTP_REQUEST_PAYLOAD:input"
  output_stream: "HTTP_RESPONSE_PAYLOAD:output"
  node_options: {
    [type.googleapis.com / mediapipe.S2tCalculatorOptions]: {
      models_path: "./",
      plugin_config: '{ "NUM_STREAMS": "{{num_streams}}" }',
      target_device: "{{target_device}}"
    }
  }
}"#;

const IMAGE_GENERATION_GRAPH_TEMPLATE: &str = r#"input_stream: "HTTP_REQUEST_PAYLOAD:input"
output_stream: "HTTP_RESPONSE_PAYLOAD:output"

node: {
  name: "ImageGenExecutor"
  calculator: "ImageGenCalculator"
  input_stream: "HTTP_REQUEST_PAYLOAD:input"
  input_side_packet: "IMAGE_GEN_NODE_RESOURCES:pipes"
  output_stream: "HTTP_RESPONSE_PAYLOAD:output"
  node_options: {
    [type.googleapis.com / mediapipe.ImageGenCalculatorOptions]: {
      models_path: "./",
      {{plugin_config_str}}device: "{{target_device}}",
      {{resolution}}{{num_images_per_prompt}}{{guidance_scale}}{{max_resolution}}{{default_resolution}}{{max_num_images_per_prompt}}{{default_num_inference_steps}}{{max_num_inference_steps}}
    }
  }
}"#;

// Helper function to render template with placeholders
fn render_template(template: &str, params: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in params {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

// Helper function to detect tool and reasoning parsers from model_id
fn detect_parsers(model_id: &str) -> (Option<String>, Option<String>) {
    let model_id_lower = model_id.to_lowercase();
    
    // Check for gpt-oss first (sets both tool_parser and reasoning_parser)
    if model_id_lower.contains("gpt-oss") {
        return (Some("gptoss".to_string()), Some("gptoss".to_string()));
    }
    
    // Check for qwen + coder combination before general qwen check
    if model_id_lower.contains("qwen") && model_id_lower.contains("coder") {
        return (Some("qwen3coder".to_string()), None);
    }
    
    // Check for other model types
    let tool_parser = if model_id_lower.contains("qwen") || model_id_lower.contains("hermes") {
        Some("hermes3".to_string())
    } else if model_id_lower.contains("llama") {
        Some("llama3".to_string())
    } else if model_id_lower.contains("mistral") {
        Some("mistral".to_string())
    } else if model_id_lower.contains("phi") {
        Some("phi4".to_string())
    } else {
        None
    };
    
    (tool_parser, None)
}

// Helper function to detect task type from model info
fn detect_task_type(model_info: &ModelInfo) -> Option<String> {
    // Check pipeline_tag first
    if let Some(pipeline_tag) = &model_info.pipeline_tag {
        match pipeline_tag.as_str() {
            "text-generation" => return Some("text_generation".to_string()),
            "feature-extraction" | "sentence-similarity" => return Some("embeddings_ov".to_string()),
            "text-classification" => {
                // Check if it's a reranker by looking at tags or model name
                if model_info.tags.iter().any(|t| t.contains("rerank")) ||
                   model_info.id.to_lowercase().contains("rerank") {
                    return Some("rerank_ov".to_string());
                }
            },
            "text-to-speech" => return Some("text2speech".to_string()),
            "automatic-speech-recognition" => return Some("speech2text".to_string()),
            "text-to-image" => return Some("image_generation".to_string()),
            _ => {}
        }
    }
    
    // Fallback to checking tags
    for tag in &model_info.tags {
        let tag_lower = tag.to_lowercase();
        if tag_lower.contains("text-generation") || tag_lower.contains("causal-lm") {
            return Some("text_generation".to_string());
        } else if tag_lower.contains("embedding") || tag_lower.contains("feature-extraction") {
            return Some("embeddings_ov".to_string());
        } else if tag_lower.contains("rerank") {
            return Some("rerank_ov".to_string());
        } else if tag_lower.contains("text-to-speech") || tag_lower.contains("tts") {
            return Some("text2speech".to_string());
        } else if tag_lower.contains("speech-to-text") || tag_lower.contains("stt") || tag_lower.contains("asr") {
            return Some("speech2text".to_string());
        } else if tag_lower.contains("text-to-image") || tag_lower.contains("stable-diffusion") {
            return Some("image_generation".to_string());
        }
    }
    
    None
}

// Helper function to generate graph.pbtxt for a given task type
fn generate_graph_for_task(
    task_type: &str,
    model_path: &PathBuf,
    model_id: &str,
    params: Option<&GraphGenerationParams>,
) -> Result<(), String> {
    let mut template_params = HashMap::new();
    
    // Get target device from params or use CPU as default
    let target_device = params
        .and_then(|p| p.target_device.as_deref())
        .unwrap_or("CPU");
    template_params.insert("target_device".to_string(), target_device.to_string());
    
    let graph_content = match task_type {
        "text_generation" | "image_text" => {
            // Build plugin config for text generation (and vision models which use same graph)
            let mut plugin_config = HashMap::new();
            
            // Add cache_dir to plugin_config
            let cache_dir = format!("{}/.ovms_cache", model_path.to_string_lossy().replace('\\', "/"));
            plugin_config.insert("CACHE_DIR".to_string(), cache_dir);
            
            if let Some(params) = params {
                if let Some(kv_precision) = &params.kv_cache_precision {
                    plugin_config.insert("KV_CACHE_PRECISION".to_string(), kv_precision.clone());
                }
            }
            
            let plugin_config_str = serde_json::to_string(&plugin_config)
                .unwrap_or_else(|_| "{}".to_string());
            
            template_params.insert("plugin_config".to_string(), plugin_config_str);
            template_params.insert(
                "enable_prefix_caching".to_string(),
                params.and_then(|p| p.enable_prefix_caching).unwrap_or(false).to_string()
            );
            template_params.insert(
                "cache_size".to_string(),
                params.and_then(|p| p.cache_size).unwrap_or(2).to_string()
            );
            template_params.insert(
                "max_num_seqs".to_string(),
                params.and_then(|p| p.max_num_seqs).unwrap_or(256).to_string()
            );
            
            if let Some(params) = params {
                if let Some(pipeline_type) = &params.pipeline_type {
                    template_params.insert("pipeline_type".to_string(), format!("pipeline_type: {},\n          ", pipeline_type));
                } else {
                    template_params.insert("pipeline_type".to_string(), "".to_string());
                }
                
                let max_batched_tokens = params.max_num_batched_tokens.unwrap_or(8192);
                template_params.insert("max_num_batched_tokens".to_string(), 
                    format!("max_num_batched_tokens: {},\n          ", max_batched_tokens));
                
                if !params.dynamic_split_fuse.unwrap_or(true) {
                    template_params.insert("dynamic_split_fuse".to_string(), "dynamic_split_fuse: false,\n          ".to_string());
                } else {
                    template_params.insert("dynamic_split_fuse".to_string(), "".to_string());
                }
            } else {
                template_params.insert("pipeline_type".to_string(), "".to_string());
                template_params.insert("max_num_batched_tokens".to_string(), 
                    "max_num_batched_tokens: 8192,\n          ".to_string());
                template_params.insert("dynamic_split_fuse".to_string(), "".to_string());
            }
            
            // Detect parsers from model_id
            let (tool_parser, reasoning_parser) = detect_parsers(model_id);
            
            if let Some(tool_parser_value) = tool_parser {
                template_params.insert("tool_parser".to_string(), 
                    format!("tool_parser: \"{}\",\n          ", tool_parser_value));
                template_params.insert("enable_tool_guided_generation".to_string(), 
                    "enable_tool_guided_generation: false,\n          ".to_string());
            } else {
                template_params.insert("tool_parser".to_string(), "".to_string());
                template_params.insert("enable_tool_guided_generation".to_string(), "".to_string());
            }
            
            if let Some(reasoning_parser_value) = reasoning_parser {
                template_params.insert("reasoning_parser".to_string(), 
                    format!("reasoning_parser: \"{}\",\n          ", reasoning_parser_value));
            } else {
                template_params.insert("reasoning_parser".to_string(), "".to_string());
            }
            
            template_params.insert("draft_models_path".to_string(), "".to_string());
            render_template(TEXT_GENERATION_GRAPH_TEMPLATE, &template_params)
        },
        "embeddings_ov" => {
            template_params.insert(
                "num_streams".to_string(),
                params.and_then(|p| p.num_streams).unwrap_or(1).to_string()
            );
            template_params.insert(
                "normalize".to_string(),
                params.and_then(|p| p.normalize).unwrap_or(true).to_string()
            );
            
            if let Some(params) = params {
                if let Some(pooling) = &params.pooling {
                    template_params.insert("pooling".to_string(), format!("pooling: {},\n      ", pooling));
                } else {
                    template_params.insert("pooling".to_string(), "".to_string());
                }
                
                if params.truncate.unwrap_or(false) {
                    template_params.insert("truncate".to_string(), "truncate: true,\n      ".to_string());
                } else {
                    template_params.insert("truncate".to_string(), "".to_string());
                }
            } else {
                template_params.insert("pooling".to_string(), "".to_string());
                template_params.insert("truncate".to_string(), "".to_string());
            }
            
            render_template(EMBEDDINGS_OV_GRAPH_TEMPLATE, &template_params)
        },
        "rerank_ov" => {
            template_params.insert(
                "num_streams".to_string(),
                params.and_then(|p| p.num_streams).unwrap_or(1).to_string()
            );
            render_template(RERANK_OV_GRAPH_TEMPLATE, &template_params)
        },
        "text2speech" => {
            template_params.insert(
                "num_streams".to_string(),
                params.and_then(|p| p.num_streams).unwrap_or(1).to_string()
            );
            render_template(TEXT2SPEECH_GRAPH_TEMPLATE, &template_params)
        },
        "speech2text" => {
            template_params.insert(
                "num_streams".to_string(),
                params.and_then(|p| p.num_streams).unwrap_or(1).to_string()
            );
            render_template(SPEECH2TEXT_GRAPH_TEMPLATE, &template_params)
        },
        "image_generation" => {
            // Build plugin config for image generation with cache_dir
            let cache_dir = format!("{}/.ovms_cache", model_path.to_string_lossy().replace('\\', "/"));
            let mut plugin_config = HashMap::new();
            plugin_config.insert("CACHE_DIR".to_string(), cache_dir);
            let plugin_config_json = serde_json::to_string(&plugin_config)
                .unwrap_or_else(|_| "{}".to_string());
            let plugin_config_str = format!("plugin_config: '{}',\n      ", plugin_config_json);
            template_params.insert("plugin_config_str".to_string(), plugin_config_str);
            
            if let Some(params) = params {
                if let Some(resolution) = &params.resolution {
                    template_params.insert("resolution".to_string(), 
                        format!("resolution: \"{}\",\n      ", resolution));
                } else {
                    template_params.insert("resolution".to_string(), "".to_string());
                }
                
                if let Some(max_resolution) = &params.max_resolution {
                    if !max_resolution.contains('x') {
                        return Err("max_resolution should be in WxH format, e.g. 1024x768".to_string());
                    }
                    template_params.insert("max_resolution".to_string(), 
                        format!("max_resolution: '{}',\n      ", max_resolution));
                } else {
                    template_params.insert("max_resolution".to_string(), "".to_string());
                }
                
                if let Some(default_resolution) = &params.default_resolution {
                    if !default_resolution.contains('x') {
                        return Err("default_resolution should be in WxH format, e.g. 512x512".to_string());
                    }
                    template_params.insert("default_resolution".to_string(), 
                        format!("default_resolution: '{}',\n      ", default_resolution));
                } else {
                    template_params.insert("default_resolution".to_string(), "".to_string());
                }
                
                if let Some(num_images) = &params.num_images_per_prompt {
                    template_params.insert("num_images_per_prompt".to_string(), 
                        format!("num_images_per_prompt: {},\n      ", num_images));
                } else {
                    template_params.insert("num_images_per_prompt".to_string(), "".to_string());
                }
                
                if let Some(guidance_scale) = &params.guidance_scale {
                    template_params.insert("guidance_scale".to_string(), 
                        format!("guidance_scale: {},\n      ", guidance_scale));
                } else {
                    template_params.insert("guidance_scale".to_string(), "".to_string());
                }
                
                if let Some(max_num_images) = params.max_num_images_per_prompt {
                    if max_num_images > 0 {
                        template_params.insert("max_num_images_per_prompt".to_string(), 
                            format!("max_num_images_per_prompt: {},\n      ", max_num_images));
                    } else {
                        template_params.insert("max_num_images_per_prompt".to_string(), "".to_string());
                    }
                } else {
                    template_params.insert("max_num_images_per_prompt".to_string(), "".to_string());
                }
                
                if let Some(default_steps) = params.default_num_inference_steps {
                    if default_steps > 0 {
                        template_params.insert("default_num_inference_steps".to_string(), 
                            format!("default_num_inference_steps: {},\n      ", default_steps));
                    } else {
                        template_params.insert("default_num_inference_steps".to_string(), "".to_string());
                    }
                } else {
                    template_params.insert("default_num_inference_steps".to_string(), "".to_string());
                }
                
                if let Some(max_steps) = params.max_num_inference_steps {
                    if max_steps > 0 {
                        template_params.insert("max_num_inference_steps".to_string(), 
                            format!("max_num_inference_steps: {},\n      ", max_steps));
                    } else {
                        template_params.insert("max_num_inference_steps".to_string(), "".to_string());
                    }
                } else {
                    template_params.insert("max_num_inference_steps".to_string(), "".to_string());
                }
            } else {
                template_params.insert("resolution".to_string(), "".to_string());
                template_params.insert("num_images_per_prompt".to_string(), "".to_string());
                template_params.insert("guidance_scale".to_string(), "".to_string());
                template_params.insert("max_resolution".to_string(), "".to_string());
                template_params.insert("default_resolution".to_string(), "".to_string());
                template_params.insert("max_num_images_per_prompt".to_string(), "".to_string());
                template_params.insert("default_num_inference_steps".to_string(), "".to_string());
                template_params.insert("max_num_inference_steps".to_string(), "".to_string());
            }
            
            render_template(IMAGE_GENERATION_GRAPH_TEMPLATE, &template_params)
        },
        _ => {
            return Err(format!("Unknown task type: {}", task_type));
        }
    };
    
    let graph_path = model_path.join("graph.pbtxt");
    fs::write(&graph_path, graph_content)
        .map_err(|e| format!("Failed to write graph.pbtxt: {}", e))?;
    
    info!(task_type = %task_type, "Generated graph.pbtxt for model");
    Ok(())
}
