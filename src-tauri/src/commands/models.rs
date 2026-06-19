use crate::error::IpcResult;
use crate::hf::{
    self,
    params::{self, ModelDeployParams},
    Cancelled, DownloadJobError, HfModelSummary, LocalModel,
};
use crate::state::AppStateExt;
use sqlx::Row;
use std::path::PathBuf;
use tauri::AppHandle;

#[tauri::command]
pub async fn models_search(app: AppHandle, query: String) -> IpcResult<Vec<HfModelSummary>> {
    let state = app.zero();
    hf::search(&state.http, &query)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn models_list_local(app: AppHandle) -> IpcResult<Vec<LocalModel>> {
    let state = app.zero();
    let rows = sqlx::query(
        "SELECT id, hf_id, path, bytes, added_at, revision, files, verified_files, pipeline_tag
         FROM local_models
         ORDER BY added_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let path: String = r.get("path");
            // Same nullable-column pattern as `hf_id` above: `.ok()`
            // collapses both SQL NULL and "column not present in the
            // row" to `None`, which keeps the read path resilient if the
            // baseline schema ever lags a deployed db.
            let stored: Option<String> = r.try_get("pipeline_tag").ok();
            // Backfill: pre-`pipeline_tag` rows store NULL, and the HF
            // upstream sometimes leaves the field unset too. Derive a
            // sensible category from the per-model sidecar so the UI
            // can still render a badge; the final fallback is
            // `text-generation`, matching the implicit default for
            // legacy text-LM installs.
            let pipeline_tag = stored.or_else(|| {
                let dir = PathBuf::from(&path);
                let params = params::read_or_default(&dir);
                Some(params::pipeline_tag_for(&params).to_string())
            });
            LocalModel {
                id: r.get("id"),
                hf_id: r.try_get("hf_id").ok(),
                path,
                bytes: r.get::<i64, _>("bytes") as u64,
                added_at: r.get("added_at"),
                revision: r.try_get("revision").ok(),
                files: r.try_get::<i64, _>("files").ok().map(|n| n.max(0) as u64),
                verified_files: r
                    .try_get::<i64, _>("verified_files")
                    .ok()
                    .map(|n| n.max(0) as u64),
                pipeline_tag,
            }
        })
        .collect())
}

/// Initial install of a HF model. Idempotent — calling it on an already
/// up-to-date model is a fast no-op (manifest read + DB upsert).
///
/// `params` lets the UI hand a fresh `.zero_model.json` to persist at
/// install time (typically used when the user picked a non-default task
/// or tweaked deployment knobs before clicking download). When `None`,
/// any existing sidecar is preserved; fresh installs use defaults.
#[tauri::command]
pub async fn models_download(
    app: AppHandle,
    model_id: String,
    params: Option<ModelDeployParams>,
) -> IpcResult<LocalModel> {
    install_or_update(app, model_id, params).await
}

/// Same code path as `models_download` — `install_or_update` already diffs
/// the upstream revision against the local manifest and re-pulls only the
/// changed files. Update never overwrites params; users edit those via
/// `models_set_params`.
#[tauri::command]
pub async fn models_update(app: AppHandle, model_id: String) -> IpcResult<LocalModel> {
    install_or_update(app, model_id, None).await
}

/// Request cancellation of an in-flight `models_download` / `models_update`.
///
/// Returns `true` when a job was found and signalled, `false` when there was
/// no active download for `model_id`. The frontend can treat both the same
/// way (the runner will emit a terminal `cancelled` progress event when the
/// abort actually lands).
#[tauri::command]
pub async fn models_cancel(app: AppHandle, model_id: String) -> IpcResult<bool> {
    Ok(app.zero().downloads.cancel(&model_id).await)
}

/// Read the per-model deployment params sidecar. Returns defaults when no
/// sidecar exists (fresh install, missing file, malformed JSON).
#[tauri::command]
pub async fn models_get_params(app: AppHandle, model_id: String) -> IpcResult<ModelDeployParams> {
    let dir = lookup_model_dir(&app, &model_id).await?;
    let dir = dir.ok_or_else(|| format!("model `{model_id}` is not installed"))?;
    let p = tokio::task::spawn_blocking(move || params::read_or_default(&dir))
        .await
        .map_err(|e| e.to_string())?;
    Ok(p)
}

/// Suggest deployment params for `model_id` by inspecting upstream HF
/// metadata + `config.json`. Used by the "configure…" modal to open
/// pre-filled with task / parser values that match the chosen model
/// instead of plain text-generation defaults.
///
/// Never fails — falls back to [`ModelDeployParams::default`] when the
/// repo can't be reached or doesn't match any of our patterns. See
/// `crate::hf::detect` for the detection rules.
#[tauri::command]
pub async fn models_suggest_params(
    app: AppHandle,
    model_id: String,
) -> IpcResult<ModelDeployParams> {
    let state = app.zero();
    Ok(crate::hf::suggest_params(&state.http, &model_id).await)
}

/// Persist a fresh sidecar **and** regenerate `<model_dir>/graph.pbtxt`
/// in lockstep so changes take effect on the next OVMS start without the
/// user having to redownload.
#[tauri::command]
pub async fn models_set_params(
    app: AppHandle,
    model_id: String,
    params: ModelDeployParams,
) -> IpcResult<()> {
    let dir = lookup_model_dir(&app, &model_id).await?;
    let dir = dir.ok_or_else(|| format!("model `{model_id}` is not installed"))?;
    let device = crate::settings::Settings::load()
        .await
        .map(|s| s.ovms.device)
        .unwrap_or_else(|_| "GPU".to_string());

    let p = params.clone();
    tokio::task::spawn_blocking(move || {
        params::write(&dir, &p)?;
        crate::ovms::config::ensure_in_model_graph(&dir, &device, &p)?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))?;
    Ok(())
}

async fn lookup_model_dir(app: &AppHandle, model_id: &str) -> IpcResult<Option<PathBuf>> {
    let state = app.zero();
    let row = sqlx::query("SELECT path FROM local_models WHERE id = ? OR hf_id = ?")
        .bind(model_id)
        .bind(model_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.map(|r| PathBuf::from(r.get::<String, _>("path"))))
}

async fn install_or_update(
    app: AppHandle,
    model_id: String,
    params: Option<ModelDeployParams>,
) -> IpcResult<LocalModel> {
    let state = app.zero();

    // Reserve the per-model slot before we do anything else so a double-
    // click in the UI surfaces as a clear error instead of two concurrent
    // writers stomping on the same directory.
    let token = match state.downloads.start(&model_id).await {
        Ok(t) => t,
        Err(DownloadJobError::AlreadyRunning(_)) => {
            return Err(format!("a download for `{model_id}` is already in progress").into());
        }
    };
    let cancel = token.handle();

    let result =
        hf::install_or_update(&app, &state.http, &state.db, &model_id, cancel, params).await;
    // `token` stays alive for the whole call above; dropping it here releases
    // the slot for the next attempt (success, cancel, or error).
    drop(token);

    match result {
        Ok(m) => Ok(m),
        Err(e) => {
            // Distinguish a user-initiated cancel from a real failure so the
            // UI can render them differently.
            let (state_kind, msg) = if e.downcast_ref::<Cancelled>().is_some() {
                (hf::DownloadState::Cancelled, "cancelled".to_string())
            } else {
                (hf::DownloadState::Error, format!("{e:#}"))
            };
            let _ = tauri::Emitter::emit(
                &app,
                crate::events::MODELS_DOWNLOAD_PROGRESS,
                hf::DownloadProgress {
                    model_id: model_id.clone(),
                    bytes_done: 0,
                    bytes_total: None,
                    files_done: 0,
                    files_total: 0,
                    state: state_kind,
                    error: Some(msg.clone()),
                },
            );
            Err(msg.into())
        }
    }
}

#[tauri::command]
pub async fn models_delete(app: AppHandle, model_id: String) -> IpcResult<()> {
    let state = app.zero();
    hf::delete_model(&state.db, &model_id)
        .await
        .map_err(|e| e.to_string().into())
}
