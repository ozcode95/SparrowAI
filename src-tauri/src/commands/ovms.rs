use crate::error::IpcResult;
use crate::ovms::{
    tfs::{ConfigStatus, ModelStatus},
    OvmsInfo,
};
use crate::state::AppStateExt;
use tauri::AppHandle;

#[tauri::command]
pub async fn ovms_info(app: AppHandle) -> IpcResult<OvmsInfo> {
    Ok(app.zero().ovms.info().await)
}

#[tauri::command]
pub async fn ovms_install(app: AppHandle) -> IpcResult<()> {
    app.zero()
        .ovms
        .install()
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn ovms_update(app: AppHandle) -> IpcResult<()> {
    app.zero()
        .ovms
        .update()
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn ovms_start(app: AppHandle, model_id: String) -> IpcResult<()> {
    app.zero()
        .ovms
        .start(&model_id)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn ovms_stop(app: AppHandle) -> IpcResult<()> {
    app.zero()
        .ovms
        .stop()
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn ovms_load_model(app: AppHandle, model_id: String) -> IpcResult<()> {
    app.zero()
        .ovms
        .load_model(&model_id)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn ovms_unload_model(app: AppHandle) -> IpcResult<()> {
    app.zero()
        .ovms
        .unload_model()
        .await
        .map_err(|e| e.to_string().into())
}

/// Remove a single model from the live serving set. Unlike
/// [`ovms_unload_model`] (which stops the whole server), this leaves the
/// other loaded models serving and applies the change via OVMS's TFS
/// `POST /v1/config/reload` endpoint — no restart, no dropped
/// connections on the surviving models.
///
/// Also removes the model from the persisted `served_models` set, so it
/// won't come back on the next app start.
#[tauri::command]
pub async fn ovms_remove_model(app: AppHandle, model_id: String) -> IpcResult<()> {
    app.zero()
        .ovms
        .remove_model(&model_id)
        .await
        .map_err(|e| e.to_string().into())
}

/// `GET /v1/config` passthrough — returns OVMS's aggregated TFS-style
/// status map for every loaded model. Powers the model inspector in the
/// Settings page so the UI can show OVMS's own view of readiness
/// (`AVAILABLE` / `LOADING` / `UNAVAILABLE`) alongside our cached
/// `loaded_models` list.
#[tauri::command]
pub async fn ovms_config_status(app: AppHandle) -> IpcResult<ConfigStatus> {
    app.zero()
        .ovms
        .config_status()
        .await
        .map_err(|e| e.to_string().into())
}

/// `GET /v1/models/<id>` passthrough — status of a single model on
/// OVMS (per-version state + error code). Used by the bottom-bar
/// status popover when the user clicks the active model name.
#[tauri::command]
pub async fn ovms_model_status(app: AppHandle, model_id: String) -> IpcResult<ModelStatus> {
    app.zero()
        .ovms
        .model_status(&model_id)
        .await
        // Use the alternate `{:#}` format so the JS side sees the full
        // anyhow cause chain (HTTP status, body excerpt, fallback note)
        // instead of just the top-level context wrapper.
        .map_err(|e| format!("{e:#}").into())
}

/// `GET /v1/models/<id>/metadata` passthrough — returns the raw TFS
/// metadata JSON for a single loaded model (signature_def with input /
/// output tensor names, dtypes, and shapes). The shape varies per
/// served task, so we keep the value untyped here and let the frontend
/// renderer pick what to surface.
#[tauri::command]
pub async fn ovms_model_metadata(app: AppHandle, model_id: String) -> IpcResult<serde_json::Value> {
    app.zero()
        .ovms
        .model_metadata(&model_id)
        .await
        .map_err(|e| e.to_string().into())
}
