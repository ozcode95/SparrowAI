use crate::errors::{Result, SparrowError};
use crate::paths::get_sparrow_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    // Chat settings
    pub include_conversation_history: bool,
    pub system_prompt: String,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: Option<i32>,
    pub max_tokens: Option<i32>,
    pub max_completion_tokens: Option<i32>,
    pub use_rag: bool,
    
    // General settings
    pub enable_autostart: bool,
    pub start_minimized: bool,
    
    // Feature flags
    pub enable_image_captioning: bool,
    pub enable_image_generation: bool,
    pub enable_rag: bool,
    pub enable_speech_to_text: bool,
    pub enable_text_to_speech: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            include_conversation_history: true,
            system_prompt: "You're an AI assistant that provides helpful responses.".to_string(),
            temperature: 0.7,
            top_p: 1.0,
            seed: None,
            max_tokens: Some(2048),
            max_completion_tokens: None,
            use_rag: false,
            enable_autostart: false,
            start_minimized: true,
            // Feature flags - all enabled by default
            enable_image_captioning: true,
            enable_image_generation: true,
            enable_rag: true,
            enable_speech_to_text: true,
            enable_text_to_speech: true,
        }
    }
}

impl AppConfig {
    /// Get the path to the app_config.json file
    pub fn config_path() -> Result<PathBuf> {
        Ok(get_sparrow_dir()?.join("app_config.json"))
    }

    /// Load the config from disk, or create default if it doesn't exist
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        
        if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|e| SparrowError::Io(e))?;
            let config: AppConfig = serde_json::from_str(&content)
                .map_err(|e| SparrowError::OperationFailed { message: e.to_string() })?;
            tracing::info!("Loaded app config from {:?}", path);
            Ok(config)
        } else {
            tracing::info!("No app config found, creating default");
            let config = Self::default();
            config.save()?;
            Ok(config)
        }
    }

    /// Save the config to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| SparrowError::Io(e))?;
        }

        let content = serde_json::to_string_pretty(self)
            .map_err(|e| SparrowError::OperationFailed { message: e.to_string() })?;
        fs::write(&path, content)
            .map_err(|e| SparrowError::Io(e))?;
        
        tracing::info!("Saved app config to {:?}", path);
        Ok(())
    }

    /// Update specific fields and save
    pub fn update(&mut self, updates: AppConfigUpdate) -> Result<()> {
        if let Some(v) = updates.include_conversation_history {
            self.include_conversation_history = v;
        }
        if let Some(v) = updates.system_prompt {
            self.system_prompt = v;
        }
        if let Some(v) = updates.temperature {
            self.temperature = v;
        }
        if let Some(v) = updates.top_p {
            self.top_p = v;
        }
        if let Some(seed) = updates.seed {
            self.seed = seed;
        }
        if let Some(max_tokens) = updates.max_tokens {
            self.max_tokens = max_tokens;
        }
        if let Some(max_completion_tokens) = updates.max_completion_tokens {
            self.max_completion_tokens = max_completion_tokens;
        }
        if let Some(v) = updates.use_rag {
            self.use_rag = v;
        }
        if let Some(v) = updates.enable_autostart {
            self.enable_autostart = v;
        }
        if let Some(v) = updates.start_minimized {
            self.start_minimized = v;
        }
        if let Some(v) = updates.enable_image_captioning {
            self.enable_image_captioning = v;
        }
        if let Some(v) = updates.enable_image_generation {
            self.enable_image_generation = v;
        }
        if let Some(v) = updates.enable_rag {
            self.enable_rag = v;
        }
        if let Some(v) = updates.enable_speech_to_text {
            self.enable_speech_to_text = v;
        }
        if let Some(v) = updates.enable_text_to_speech {
            self.enable_text_to_speech = v;
        }
        
        self.save()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfigUpdate {
    pub include_conversation_history: Option<bool>,
    pub system_prompt: Option<String>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub seed: Option<Option<i32>>,
    pub max_tokens: Option<Option<i32>>,
    pub max_completion_tokens: Option<Option<i32>>,
    pub use_rag: Option<bool>,
    pub enable_autostart: Option<bool>,
    pub start_minimized: Option<bool>,
    // Feature flags
    pub enable_image_captioning: Option<bool>,
    pub enable_image_generation: Option<bool>,
    pub enable_rag: Option<bool>,
    pub enable_speech_to_text: Option<bool>,
    pub enable_text_to_speech: Option<bool>,
}

// Tauri commands
#[tauri::command]
pub fn get_app_config() -> std::result::Result<AppConfig, String> {
    AppConfig::load().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_app_config(config: AppConfig) -> std::result::Result<(), String> {
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_app_config(updates: AppConfigUpdate) -> std::result::Result<AppConfig, String> {
    let mut config = AppConfig::load().map_err(|e| e.to_string())?;
    config.update(updates).map_err(|e| e.to_string())?;
    Ok(config)
}
