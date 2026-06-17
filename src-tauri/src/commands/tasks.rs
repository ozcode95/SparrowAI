use crate::error::IpcResult;
use crate::state::AppStateExt;
use crate::tasks::{self, NewTask, Task};
use tauri::AppHandle;

#[tauri::command]
pub async fn tasks_list(app: AppHandle) -> IpcResult<Vec<Task>> {
    tasks::list(&app.zero().db)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn tasks_create(app: AppHandle, task: NewTask) -> IpcResult<String> {
    tasks::create(&app.zero().db, task)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn tasks_update(app: AppHandle, task: Task) -> IpcResult<()> {
    tasks::update(&app.zero().db, task)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn tasks_delete(app: AppHandle, id: String) -> IpcResult<()> {
    tasks::delete(&app.zero().db, &id)
        .await
        .map_err(|e| e.to_string().into())
}

/// Run a task on demand from the UI.
///
/// We load the task, execute its action via [`tasks::run_action`], and
/// stamp `last_run_at`/`last_status` so the row reflects the outcome in
/// the next `tasks_list`. Errors from the action are propagated back to
/// the caller so the UI can show them; we still record an `error` row in
/// that case so the user can see *something* happened on the timeline.
#[tauri::command]
pub async fn tasks_run_now(app: AppHandle, id: String) -> IpcResult<String> {
    let state = app.zero();
    let task = tasks::get(&state.db, &id)
        .await
        .map_err(|e| e.to_string())?;

    // Stamp "running" so the UI can show in-flight state if the action
    // takes a while (e.g. a long script).
    let _ = tasks::record_run(&state.db, &id, "running").await;

    match tasks::run_action(&app, &task.action).await {
        Ok(summary) => {
            if let Err(e) = tasks::record_run(&state.db, &id, "ok").await {
                tracing::warn!("tasks: record_run(ok) failed for {id}: {e:#}");
            }
            Ok(summary)
        }
        Err(e) => {
            let msg = e.to_string();
            if let Err(rec) = tasks::record_run(&state.db, &id, "error").await {
                tracing::warn!("tasks: record_run(error) failed for {id}: {rec:#}");
            }
            Err(msg.into())
        }
    }
}

#[tauri::command]
pub async fn tasks_set_enabled(app: AppHandle, id: String, enabled: bool) -> IpcResult<()> {
    tasks::set_enabled(&app.zero().db, &id, enabled)
        .await
        .map_err(|e| e.to_string().into())
}
