//! Tauri IPC wrappers for the `Settings` model. The real type lives in
//! `crate::settings` so non-IPC code (the agent runner, provider factory) can
//! consume it without going through the command bus.
//!
//! Re-exporting keeps the legacy `commands::settings::Settings` import path
//! valid for any frontend bindings that were generated against it.

use crate::error::IpcResult;
use crate::secrets;

pub use crate::settings::{OvmsSettings, ProviderConfig, Settings};

/// Key under which the HF token lives in the OS keychain (or the plaintext
/// fallback file). Kept in one place so [`secrets`] callers don't drift.
const HF_TOKEN_KEY: &str = "hf_token";

#[tauri::command]
pub async fn settings_load() -> IpcResult<Settings> {
    // Synchronise `hf_token_set` with reality so the UI never claims a token
    // exists when the keychain entry has been revoked out-of-band.
    let mut s = Settings::load().await.map_err(|e| e.to_string())?;
    let token_present = secrets::get(HF_TOKEN_KEY)
        .map_err(|e| e.to_string())?
        .is_some();
    if s.hf_token_set != token_present {
        s.hf_token_set = token_present;
        // Best-effort persist; don't fail the load if we can't write it.
        if let Err(e) = s.save().await {
            tracing::warn!("settings_load: could not persist token-state fix: {e:#}");
        }
    }
    Ok(s)
}

#[tauri::command]
pub async fn settings_save(settings: Settings) -> IpcResult<()> {
    settings.save().await.map_err(|e| e.to_string().into())
}

/// Persist the Hugging Face token. Tries the OS keychain first and falls back
/// to a plaintext file under the app's local dir if no credential service is
/// available (headless Linux, some sandboxed environments, etc.). The
/// `Settings.hf_token_set` flag is flipped on so the UI can render `[set]`.
#[tauri::command]
pub async fn settings_set_hf_token(token: String) -> IpcResult<()> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err("token is empty".into());
    }
    // Run the keychain call on the blocking pool — `keyring` is sync and
    // some backends (Linux Secret Service over DBus) can block briefly.
    let val = trimmed.to_string();
    tokio::task::spawn_blocking(move || secrets::set(HF_TOKEN_KEY, &val))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    let mut s = Settings::load().await.map_err(|e| e.to_string())?;
    s.hf_token_set = true;
    s.save().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Forget the stored Hugging Face token. Idempotent: clearing an already
/// empty entry is a successful no-op.
#[tauri::command]
pub async fn settings_clear_hf_token() -> IpcResult<()> {
    tokio::task::spawn_blocking(|| secrets::delete(HF_TOKEN_KEY))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    let mut s = Settings::load().await.map_err(|e| e.to_string())?;
    s.hf_token_set = false;
    s.save().await.map_err(|e| e.to_string())?;
    Ok(())
}
