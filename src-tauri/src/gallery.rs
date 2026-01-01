use serde::{Deserialize, Serialize};
use tracing::{error, info, debug};
use std::path::PathBuf;
use std::fs;
use uuid::Uuid;
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::images::{
        CreateImageRequestArgs,
        ImageSize,
        ImageResponseFormat,
        ImageModel
    },
};

use crate::paths;
use crate::constants;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImage {
    pub id: String,
    pub prompt: String,
    #[serde(alias = "image_path")]
    pub image_path: String,
    pub timestamp: i64,
    #[serde(alias = "model_id")]
    pub model_id: String,
    pub size: String,
    #[serde(skip_serializing_if = "Option::is_none", alias = "num_inference_steps")]
    pub num_inference_steps: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GeneratedImagesStorage {
    pub images: Vec<GeneratedImage>,
}

impl Default for GeneratedImagesStorage {
    fn default() -> Self {
        Self {
            images: Vec::new(),
        }
    }
}

fn get_images_dir() -> Result<PathBuf, String> {
    let dir = paths::get_sparrow_dir()
        .map_err(|e| e.to_string())?
        .join("images");
    
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create images directory: {}", e))?;
    }
    
    Ok(dir)
}

fn get_images_metadata_path() -> Result<PathBuf, String> {
    let dir = get_images_dir()?;
    Ok(dir.join("metadata.json"))
}

fn load_images_metadata() -> Result<GeneratedImagesStorage, String> {
    let path = get_images_metadata_path()?;
    
    if !path.exists() {
        return Ok(GeneratedImagesStorage::default());
    }
    
    let contents = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read images metadata: {}", e))?;
    
    serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse images metadata: {}", e))
}

fn save_images_metadata(storage: &GeneratedImagesStorage) -> Result<(), String> {
    let path = get_images_metadata_path()?;
    
    let json = serde_json::to_string_pretty(storage)
        .map_err(|e| format!("Failed to serialize images metadata: {}", e))?;
    
    fs::write(&path, json)
        .map_err(|e| format!("Failed to write images metadata: {}", e))?;
    
    Ok(())
}

#[tauri::command]
pub async fn generate_image(
    model_id: String,
    prompt: String,
    size: String,
    num_inference_steps: i32,
    reference_images: Vec<String>,
) -> Result<serde_json::Value, String> {
    info!("Generating image with model: {}, size: {}, steps: {}", model_id, size, num_inference_steps);
    debug!("Prompt: {}", prompt);
    debug!("Reference images: {:?}", reference_images);

    let mut model_id = model_id;
    if model_id.starts_with("OpenVINO/") {
        // remove OpenVINO/ prefix
        model_id = model_id.trim_start_matches("OpenVINO/").to_string();
        info!("Using OVMS model ID: {}", model_id);
    }
    
    // Handle reference images if provided
    if !reference_images.is_empty() {
        info!("Reference images provided but not yet supported in current implementation");
    }

    // Parse the size string (e.g., "512x512") into ImageSize
    let image_size = match size.as_str() {
        "256x256" => ImageSize::S256x256,
        "512x512" => ImageSize::S512x512,
        "1024x1024" => ImageSize::S1024x1024,
        _ => ImageSize::S512x512, // default
    };

    // Configure async_openai client to use OVMS endpoint
    let config = OpenAIConfig::new()
        .with_api_base(&format!("{}/v3", constants::OVMS_API_BASE))
        .with_api_key(""); // OVMS doesn't require an API key

    let client = Client::with_config(config);
    
    // Build the image generation request
    let request = CreateImageRequestArgs::default()
        .prompt(prompt.clone())
        .model(ImageModel::Other(model_id.clone()))
        .n(1)
        .size(image_size)
        .response_format(ImageResponseFormat::B64Json)
        .build().map_err(|e| {
            error!("Failed to generate image request: {}", e);
            format!("Failed to generate image request: {}", e)
        })?;

    // Add num_inference_steps as extra parameter if needed
    // Note: This might need to be handled differently depending on OVMS implementation
    info!("Making image generation request to OVMS");

    // Call the async_openai API
    let response = client
        .images()
        .generate(request)
        .await
        .map_err(|e| {
            error!("Failed to generate image: {}", e);
            format!("Failed to generate image: {}", e)
        })?;

    // Get the base64 image data from the response
    if response.data.is_empty() {
        return Err("No image data received from server".to_string());
    }

    // Generate metadata before saving
    let image_id = Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().timestamp();
    
    let images_dir = get_images_dir()?;
    
    // Save images using the response's save method
    let paths = response.save(images_dir.to_str().ok_or("Invalid images directory path")?)
        .await
        .map_err(|e| format!("Failed to save image: {}", e))?;

    let image_path = paths.get(0)
        .ok_or("No image path returned from save operation")?;

    info!("Image saved to: {}", image_path.display());

    // Create metadata entry
    let generated_image = GeneratedImage {
        id: image_id.clone(),
        prompt: prompt.clone(),
        image_path: image_path.to_string_lossy().to_string(),
        timestamp,
        model_id: model_id.clone(),
        size: size.clone(),
        num_inference_steps: Some(num_inference_steps),
    };

    // Save to metadata
    let mut storage = load_images_metadata()?;
    storage.images.insert(0, generated_image);
    save_images_metadata(&storage)?;

    Ok(serde_json::json!({
        "id": image_id,
        "image_path": image_path.to_string_lossy().to_string(),
    }))
}

#[tauri::command]
pub async fn get_generated_images() -> Result<Vec<GeneratedImage>, String> {
    let storage = load_images_metadata()?;
    Ok(storage.images)
}

#[tauri::command]
pub async fn delete_generated_image(image_id: String, image_path: String) -> Result<(), String> {
    info!("Deleting image: {} at path: {}", image_id, image_path);

    // Delete the file
    let path = PathBuf::from(&image_path);
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| format!("Failed to delete image file: {}", e))?;
    }

    // Update metadata
    let mut storage = load_images_metadata()?;
    storage.images.retain(|img| img.id != image_id);
    save_images_metadata(&storage)?;

    Ok(())
}

#[tauri::command]
pub async fn copy_file(source_path: String, dest_path: String) -> Result<(), String> {
    fs::copy(&source_path, &dest_path)
        .map_err(|e| format!("Failed to copy file: {}", e))?;
    Ok(())
}
