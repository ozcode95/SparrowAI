use serde::{Deserialize, Serialize};
use tracing::{error, info, debug};
use std::path::PathBuf;
use std::fs;
use uuid::Uuid;
use base64::Engine;
use reqwest;

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

    // Build request body
    let mut request_body = serde_json::json!({
        "model": "stable-diffusion-v1-5-int8-ov",
        "prompt": prompt,
        "size": size,
        "n": 1,
        "response_format": "b64_json"
    });

    // Add num_inference_steps to the request
    if let Some(obj) = request_body.as_object_mut() {
        obj.insert(
            "num_inference_steps".to_string(),
            serde_json::Value::Number(serde_json::Number::from(num_inference_steps))
        );
    }

    // Handle reference images if provided
    if !reference_images.is_empty() {
        info!("Reference images provided but not yet supported in current implementation");
    }

    // Make direct HTTP request to OVMS
    let url = format!("{}/v3/images/generations", constants::OVMS_API_BASE);
    info!("Calling OVMS image generation API at: {}", url);

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            error!("Failed to send image generation request: {}", e);
            format!("Failed to send image generation request: {}", e)
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        error!("Image generation failed with status {}: {}", status, error_text);
        return Err(format!("Image generation failed: {} - {}", status, error_text));
    }

    let response_json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| {
            error!("Failed to parse response: {}", e);
            format!("Failed to parse response: {}", e)
        })?;

    // Get the base64 image data
    let data_array = response_json["data"].as_array()
        .ok_or("No data array in response")?;
    
    if data_array.is_empty() {
        return Err("No image data received from server".to_string());
    }

    let image_b64 = data_array[0]["b64_json"].as_str()
        .ok_or("No base64 image data in response")?;

    // Decode base64 and save to file
    let image_bytes = base64::engine::general_purpose::STANDARD
        .decode(image_b64)
        .map_err(|e| format!("Failed to decode base64 image: {}", e))?;

    // Generate unique filename
    let image_id = Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().timestamp();
    let filename = format!("{}_{}.png", timestamp, &image_id[..8]);
    
    let images_dir = get_images_dir()?;
    let image_path = images_dir.join(&filename);

    // Save the image
    fs::write(&image_path, &image_bytes)
        .map_err(|e| format!("Failed to save image: {}", e))?;

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
