use tauri_plugin_autostart::ManagerExt;
use tauri::Runtime;

#[derive(Debug, thiserror::Error)]
pub enum AutostartError {
    #[error("Failed to enable autostart: {0}")]
    EnableFailed(String),
    #[error("Failed to disable autostart: {0}")]
    DisableFailed(String),
    #[error("Failed to check autostart status: {0}")]
    StatusCheckFailed(String),
}

impl serde::Serialize for AutostartError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Enable autostart for the application
#[tauri::command]
pub async fn enable_autostart<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), AutostartError> {
    app.autolaunch()
        .enable()
        .map_err(|e| AutostartError::EnableFailed(e.to_string()))
}

/// Disable autostart for the application
#[tauri::command]
pub async fn disable_autostart<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), AutostartError> {
    app.autolaunch()
        .disable()
        .map_err(|e| AutostartError::DisableFailed(e.to_string()))
}

/// Check if autostart is enabled
#[tauri::command]
pub async fn is_autostart_enabled<R: Runtime>(app: tauri::AppHandle<R>) -> Result<bool, AutostartError> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| AutostartError::StatusCheckFailed(e.to_string()))
}

/// Toggle autostart on/off
#[tauri::command]
pub async fn toggle_autostart<R: Runtime>(app: tauri::AppHandle<R>) -> Result<bool, AutostartError> {
    let is_enabled = is_autostart_enabled(app.clone()).await?;
    
    if is_enabled {
        disable_autostart(app).await?;
        Ok(false)
    } else {
        enable_autostart(app).await?;
        Ok(true)
    }
}
