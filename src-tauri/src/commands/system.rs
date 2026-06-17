use crate::error::IpcResult;
use crate::state::AppStateExt;
use crate::system::{self, recommend, Specs};
use std::collections::HashMap;
use tauri::AppHandle;

/// Hardware/OS probe. Cached to `system.json`; `force = true` re-probes.
#[tauri::command]
pub async fn system_probe(app: AppHandle, force: Option<bool>) -> IpcResult<Specs> {
    let force = force.unwrap_or(false);

    // Try the in-memory cache first.
    let state = app.zero();
    if !force {
        if let Some(cached) = state.specs.read().await.clone() {
            return Ok(cached);
        }
        // Then the on-disk cache.
        if let Some(disk) = system::load_cached() {
            *state.specs.write().await = Some(disk.clone());
            return Ok(disk);
        }
    }

    // Fresh probe (sysinfo + WMI on Windows). Run on a blocking pool so we
    // don't block the Tokio runtime with COM/WMI calls.
    let specs = tokio::task::spawn_blocking(system::probe)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    if let Err(e) = system::save_cached(&specs) {
        tracing::warn!("system cache write failed: {e}");
    }
    *state.specs.write().await = Some(specs.clone());
    Ok(specs)
}

/// Recommend models for every task category based on the user's hardware
/// specs. Searches HuggingFace for OpenVINO IR int8 chat models (Qwen3/3.5,
/// Gemma 4, Phi‑3.5) and supplements a static catalog of non‑chat models.
///
/// Returns a map keyed by task kind (e.g. `"text_generation"`), each value
/// being a ranked list of [`recommend::RecommendedModel`].
#[tauri::command]
pub async fn system_recommend_models(
    app: AppHandle,
) -> IpcResult<HashMap<String, Vec<recommend::RecommendedModel>>> {
    let state = app.zero();

    // Ensure we have specs — probe if not yet cached.
    let specs = {
        let guard = state.specs.read().await;
        match guard.clone() {
            Some(s) => s,
            None => {
                drop(guard);
                let s = tokio::task::spawn_blocking(system::probe)
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                if let Err(e) = system::save_cached(&s) {
                    tracing::warn!("system cache write failed: {e}");
                }
                *state.specs.write().await = Some(s.clone());
                s
            }
        }
    };

    Ok(recommend::recommend_all(&state.http, &specs).await)
}
