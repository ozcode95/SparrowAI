//! Tauri commands for chat attachments (image/doc upload).

use crate::attachments::{self, Attachment};
use crate::error::IpcResult;
use crate::state::AppStateExt;
use std::path::PathBuf;
use tauri::AppHandle;

/// Copy a single file from `source_path` into the conversation's
/// attachment store and return the persisted metadata. The frontend
/// then includes the returned `Attachment` in `chat_send_message`.
#[tauri::command]
pub async fn attachments_save(
    conversation_id: String,
    source_path: String,
) -> IpcResult<Attachment> {
    let p = PathBuf::from(&source_path);
    attachments::save(&conversation_id, &p)
        .await
        .map_err(|e| e.to_string().into())
}

/// Drop a single attachment file from disk. Idempotent — missing files
/// are silently ignored so the UI can call this without checking first.
#[tauri::command]
pub async fn attachments_delete(path: String) -> IpcResult<()> {
    let p = PathBuf::from(&path);
    if p.exists() {
        tokio::fs::remove_file(&p)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Strip every attachment directory belonging to a conversation. Called by
/// `chat_delete_conversation` directly; exposed as an IPC command too so a
/// future "clear files only" affordance can call it without removing the
/// conversation itself.
#[tauri::command]
pub async fn attachments_purge_conversation(
    app: AppHandle,
    conversation_id: String,
) -> IpcResult<()> {
    let _ = app.zero(); // ensure state is up so we fail fast on cold start
    let id = conversation_id.clone();
    tokio::task::spawn_blocking(move || attachments::purge_conversation(&id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(())
}
