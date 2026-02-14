use tracing::info;
use async_openai::{Client, config::OpenAIConfig};
use async_openai::types::audio::{
    CreateSpeechRequestArgs, 
    SpeechModel,
    CreateTranscriptionRequestArgs,
    CreateTranslationRequestArgs,
    AudioResponseFormat,
};

use crate::paths;

/// Generate text-to-speech audio for a message
#[tauri::command]
pub async fn generate_tts_for_message(
    session_id: String,
    message_id: String,
    text: String,
    model_id: String,
) -> Result<String, String> {
    // Strip "OpenVINO/" prefix if present
    let model_id = model_id.strip_prefix("OpenVINO/").unwrap_or(&model_id).to_string();
    
    info!("Generating TTS for message: {} with model: {}", message_id, model_id);
    
    // Get audio directory
    let audio_dir = paths::get_audio_dir().map_err(|e| e.to_string())?;
    let audio_path = audio_dir.join(format!("{}.wav", message_id));
    
    // Check if audio file already exists
    if audio_path.exists() {
        info!("Audio file already exists, returning cached path");
        return Ok(audio_path.to_string_lossy().to_string());
    }
    
    // Configure OpenAI client to use OVMS endpoint
    let config = OpenAIConfig::new()
        .with_api_base("http://localhost:1114/v3")
        .with_api_key("unused"); // OVMS doesn't require real API key
    
    let client = Client::with_config(config);
    
    // Create audio speech request (without voice parameter as OVMS doesn't support it)
    let request = CreateSpeechRequestArgs::default()
        .input(text.clone())
        .model(SpeechModel::Other(model_id))
        .build()
        .map_err(|e| format!("Failed to build TTS request: {}", e))?;
    
    // Call the audio API
    let response = client
        .audio()
        .speech()
        .create(request)
        .await
        .map_err(|e| format!("Failed to generate TTS: {}", e))?;
    
    // Save the audio bytes to file
    response
        .save(&audio_path)
        .await
        .map_err(|e| format!("Failed to save audio file: {}", e))?;
    
    info!("TTS audio saved to: {:?}", audio_path);
    
    // Update the message in the session with the audio path
    update_message_audio_path(session_id, message_id, audio_path.to_string_lossy().to_string()).await?;
    
    Ok(audio_path.to_string_lossy().to_string())
}

/// Convert speech to text using OVMS Whisper model
#[tauri::command]
pub async fn transcribe_audio(
    audio_file_path: String,
    model_id: String,
    language: Option<String>,
) -> Result<String, String> {
    // Strip "OpenVINO/" prefix if present
    let model_id = model_id.strip_prefix("OpenVINO/").unwrap_or(&model_id).to_string();
    
    info!("Transcribing audio from: {} with model: {}", audio_file_path, model_id);
    
    // Verify the file exists
    if !std::path::Path::new(&audio_file_path).exists() {
        return Err(format!("Audio file not found: {}", audio_file_path));
    }
    
    // Configure OpenAI client to use OVMS endpoint
    let config = OpenAIConfig::new()
        .with_api_base("http://localhost:1114/v3")
        .with_api_key("unused"); // OVMS doesn't require real API key
    
    let client = Client::with_config(config);
    
    // Build transcription request
    let mut request_builder = CreateTranscriptionRequestArgs::default();
    request_builder
        .file(audio_file_path.clone())
        .model(model_id)
        .response_format(AudioResponseFormat::Json);
    
    // Add language if provided (helps with accuracy)
    if let Some(lang) = language {
        request_builder.language(&lang);
    }
    
    let request = request_builder
        .build()
        .map_err(|e| format!("Failed to build transcription request: {}", e))?;
    
    // Call the transcription API
    let response = client
        .audio()
        .transcription()
        .create(request)
        .await
        .map_err(|e| format!("Failed to transcribe audio: {}", e))?;
    
    info!("Transcription completed successfully");
    
    Ok(response.text)
}

/// Translate speech to English text using OVMS Whisper model
#[tauri::command]
pub async fn translate_audio(
    audio_file_path: String,
    model_id: String,
) -> Result<String, String> {
    // Strip "OpenVINO/" prefix if present
    let model_id = model_id.strip_prefix("OpenVINO/").unwrap_or(&model_id).to_string();
    
    info!("Translating audio from: {} with model: {}", audio_file_path, model_id);
    
    // Verify the file exists
    if !std::path::Path::new(&audio_file_path).exists() {
        return Err(format!("Audio file not found: {}", audio_file_path));
    }
    
    // Configure OpenAI client to use OVMS endpoint
    let config = OpenAIConfig::new()
        .with_api_base("http://localhost:1114/v3")
        .with_api_key("unused"); // OVMS doesn't require real API key
    
    let client = Client::with_config(config);
    
    // Build translation request
    let request = CreateTranslationRequestArgs::default()
        .file(audio_file_path.clone())
        .model(model_id)
        .build()
        .map_err(|e| format!("Failed to build translation request: {}", e))?;
    
    // Call the translation API
    let response = client
        .audio()
        .translation()
        .create(request)
        .await
        .map_err(|e| format!("Failed to translate audio: {}", e))?;
    
    info!("Translation completed successfully");
    
    Ok(response.text)
}

/// Helper function to update message with audio path
async fn update_message_audio_path(
    session_id: String,
    message_id: String,
    audio_path: String,
) -> Result<(), String> {
    let mut storage = crate::chat::load_chat_sessions()?;
    
    if let Some(session) = storage.sessions.get_mut(&session_id) {
        if let Some(message) = session.messages.iter_mut().find(|m| m.id == message_id) {
            message.audio_path = Some(audio_path);
            crate::chat::save_chat_sessions(&storage)?;
            info!("Updated message with audio path");
        }
    }
    
    Ok(())
}
