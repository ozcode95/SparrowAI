//! IPC surface for the persistent-memory subsystem.
//!
//! Backs the Memory page in the UI plus any future hotkeys / command
//! palette actions. Every command resolves [`crate::memory::MemoryTarget`]
//! from a wire-side string so the frontend can stay schema-light and just
//! pass `"memory"` / `"user"` around.
//!
//! Errors are surfaced as `String`s through [`IpcResult`] to keep the
//! React side free of typed error handling — the UI just renders whatever
//! comes back. The Tool surface (used by the model) gets the structured
//! [`crate::memory::MemoryError`] shape instead; the two paths share the
//! same underlying [`crate::memory`] API.

use crate::error::IpcResult;
use crate::memory::{self, MemorySnapshot, MemoryState, MemoryTarget};

#[tauri::command]
pub async fn memory_load() -> IpcResult<MemoryState> {
    memory::load_state().await.map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn memory_add(target: String, content: String) -> IpcResult<MemorySnapshot> {
    let target = parse_target(&target)?;
    memory::add(target, &content)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn memory_replace(
    target: String,
    old_text: String,
    content: String,
) -> IpcResult<MemorySnapshot> {
    let target = parse_target(&target)?;
    memory::replace(target, &old_text, &content)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn memory_remove(target: String, old_text: String) -> IpcResult<MemorySnapshot> {
    let target = parse_target(&target)?;
    memory::remove(target, &old_text)
        .await
        .map_err(|e| e.to_string().into())
}

/// Raw overwrite. Used by the "edit as text" mode of the Memory page so
/// power users can reshape the whole store at once (paste a fresh list,
/// reorder, etc.) without going through `replace` one entry at a time.
/// Still cap-checked so the UI can't bypass the limit.
#[tauri::command]
pub async fn memory_set_raw(target: String, raw: String) -> IpcResult<MemorySnapshot> {
    let target = parse_target(&target)?;
    memory::set_raw(target, &raw)
        .await
        .map_err(|e| e.to_string().into())
}

/// Wire-side target strings are validated here rather than in
/// `crate::memory` so callers without a `serde` round-trip (anyone
/// calling these commands by hand from the JS console, for example) get
/// a clear error instead of a deserialiser noise message.
fn parse_target(s: &str) -> Result<MemoryTarget, crate::error::IpcError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "memory" => Ok(MemoryTarget::Memory),
        "user" => Ok(MemoryTarget::User),
        other => {
            Err(format!("unknown memory target `{other}` (expected `memory` or `user`)").into())
        }
    }
}
