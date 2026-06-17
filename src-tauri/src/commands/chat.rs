use crate::attachments;
use crate::chat::{self, runner, Attachment, Conversation, Message};
use crate::error::IpcResult;
use crate::settings::SamplingConfig;
use crate::state::AppStateExt;
use serde::Serialize;
use tauri::AppHandle;

#[tauri::command]
pub async fn chat_list_conversations(app: AppHandle) -> IpcResult<Vec<Conversation>> {
    let state = app.zero();
    chat::list_conversations(&state.db)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn chat_list_messages(
    app: AppHandle,
    conversation_id: String,
) -> IpcResult<Vec<Message>> {
    let state = app.zero();
    chat::list_messages(&state.db, &conversation_id)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn chat_create_conversation(app: AppHandle, title: String) -> IpcResult<String> {
    let state = app.zero();
    chat::create_conversation(&state.db, &title)
        .await
        .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn chat_delete_conversation(app: AppHandle, conversation_id: String) -> IpcResult<()> {
    let state = app.zero();
    chat::delete_conversation(&state.db, &conversation_id)
        .await
        .map_err(|e| e.to_string())?;
    // Best-effort: wipe attached files so deleting a conversation doesn't
    // leak its images / documents on disk. Failure is non-fatal.
    let id = conversation_id.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || attachments::purge_conversation(&id))
        .await
        .map_err(|e| e.to_string())?
    {
        tracing::warn!("attachment purge for {conversation_id} failed: {e:#}");
    }
    Ok(())
}

/// Pin a model id to a conversation. Pass an empty string (or omit `model`
/// client-side via `null`) to clear the pin and fall back to the global
/// `settings.default_model` / OVMS loaded model.
#[tauri::command]
pub async fn chat_set_model(
    app: AppHandle,
    conversation_id: String,
    model: Option<String>,
) -> IpcResult<()> {
    let state = app.zero();
    let normalized = model.as_deref().map(str::trim).filter(|s| !s.is_empty());
    chat::set_conversation_model(&state.db, &conversation_id, normalized)
        .await
        .map_err(|e| e.to_string().into())
}

/// Rename a conversation. The new title is trimmed; an empty result falls
/// back to `"New chat"` so the sidebar never shows a blank row.
#[tauri::command]
pub async fn chat_set_title(
    app: AppHandle,
    conversation_id: String,
    title: String,
) -> IpcResult<()> {
    let state = app.zero();
    let trimmed = title.trim();
    let normalized = if trimmed.is_empty() {
        "New chat"
    } else {
        trimmed
    };
    chat::set_conversation_title(&state.db, &conversation_id, normalized)
        .await
        .map_err(|e| e.to_string().into())
}

/// Read the per-conversation tool-disable list. Entries are catalog keys
/// of the form `<server_id>::<tool_name>` and represent tools the user
/// explicitly turned off for this chat (overriding the global enable on
/// the Tools page). An empty list means "inherit Settings".
#[tauri::command]
pub async fn chat_get_disabled_tools(
    app: AppHandle,
    conversation_id: String,
) -> IpcResult<Vec<String>> {
    let state = app.zero();
    chat::conversation_disabled_tools(&state.db, &conversation_id)
        .await
        .map_err(|e| e.to_string().into())
}

/// Replace the per-conversation tool-disable list. Pass an empty array
/// to clear it (back to inheriting Settings). The next chat turn picks
/// up the new filter through `mcp_catalog::fetch_enabled` → runner.
#[tauri::command]
pub async fn chat_set_disabled_tools(
    app: AppHandle,
    conversation_id: String,
    keys: Vec<String>,
) -> IpcResult<()> {
    let state = app.zero();
    chat::set_conversation_disabled_tools(&state.db, &conversation_id, &keys)
        .await
        .map_err(|e| e.to_string().into())
}

/// Read the per-conversation sampling override. Returns the default
/// (all-`None`) [`SamplingConfig`] when the chat has no explicit
/// override — the runner then falls back to the provider's `sampling`
/// block, and finally to the per-model [`ModelProfile`] default.
#[tauri::command]
pub async fn chat_get_sampling(
    app: AppHandle,
    conversation_id: String,
) -> IpcResult<SamplingConfig> {
    let state = app.zero();
    chat::conversation_sampling(&state.db, &conversation_id)
        .await
        .map_err(|e| e.to_string().into())
}

/// Replace the per-conversation sampling override. A fully-empty
/// [`SamplingConfig`] (every field unset) clears the override so the
/// chat falls back to the provider/profile defaults. The next chat
/// turn picks up the new values automatically — the runner re-reads
/// the override at the start of every `run_inner`.
#[tauri::command]
pub async fn chat_set_sampling(
    app: AppHandle,
    conversation_id: String,
    sampling: SamplingConfig,
) -> IpcResult<()> {
    let state = app.zero();
    chat::set_conversation_sampling(&state.db, &conversation_id, &sampling)
        .await
        .map_err(|e| e.to_string().into())
}

/// Response shape for `chat_send_message`: both the persisted user turn and
/// the placeholder assistant row whose `id` the UI uses to anchor incoming
/// `chat://delta` events.
#[derive(Debug, Clone, Serialize)]
pub struct SendResult {
    pub user: Message,
    pub assistant: Message,
}

/// Persist the user message, insert an empty assistant placeholder, then
/// spawn the streaming runner. Returns immediately so the UI can render both
/// turns before the first token arrives.
///
/// `overrides` carries the per-turn capability flags (web search, deep
/// research, thinking opt-in, autonomous loop) the composer captured
/// from the `+`-menu toggles. They're persisted onto the user row so
/// `chat_retry` re-applies the same flags without the frontend having
/// to round-trip them again.
#[tauri::command]
pub async fn chat_send_message(
    app: AppHandle,
    conversation_id: String,
    content: String,
    attachments: Vec<Attachment>,
    overrides: Option<chat::TurnOverrides>,
) -> IpcResult<SendResult> {
    let state = app.zero();

    let user = chat::insert_message(
        &state.db,
        &conversation_id,
        "user",
        &content,
        None,
        if attachments.is_empty() {
            None
        } else {
            Some(&attachments)
        },
        overrides.as_ref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    let assistant = chat::insert_message(
        &state.db,
        &conversation_id,
        "assistant",
        "",
        None,
        None,
        None,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Hand off to the runner. We deliberately don't await — streaming happens
    // entirely via `chat://delta` events.
    let app_clone = app.clone();
    let db = state.db.clone();
    let http = state.http.clone();
    let ovms = state.ovms.clone();
    let jobs = state.chat_jobs.clone();
    let conv_id = conversation_id.clone();
    let asst_id = assistant.id.clone();
    tauri::async_runtime::spawn(async move {
        runner::run(app_clone, db, http, ovms, jobs, conv_id, asst_id).await;
    });

    Ok(SendResult { user, assistant })
}

/// Cancel the in-flight assistant stream identified by `message_id`. No-op if
/// the message has already finished (or was never registered).
#[tauri::command]
pub async fn chat_cancel(app: AppHandle, message_id: String) -> IpcResult<()> {
    let state = app.zero();
    let cancelled = state.chat_jobs.cancel(&message_id).await;
    if !cancelled {
        tracing::debug!("chat_cancel: no in-flight job for {message_id}");
    }
    Ok(())
}

/// Resolve a pending destructive-tool confirmation. `allow = true` runs
/// the tool; `false` (or this never being called) lets the runner record
/// a `[refused by user]` result and continue. No-op when `call_id` is
/// unknown — the confirm may have already been cancelled by chat-level
/// cancellation.
#[tauri::command]
pub async fn chat_tool_confirm(app: AppHandle, call_id: String, allow: bool) -> IpcResult<bool> {
    let state = app.zero();
    Ok(state.tool_confirms.resolve(&call_id, allow).await)
}

/// Re-run the streaming runner against an existing assistant message.
///
/// The frontend calls this after a `chat://error` event with `retryable:
/// true`. We clear the persisted error body, look up the conversation, and
/// spawn `runner::run` again — reusing the same assistant id so the UI's
/// existing message bubble is updated in place instead of getting a new
/// row.
#[tauri::command]
pub async fn chat_retry(app: AppHandle, message_id: String) -> IpcResult<()> {
    use sqlx::Row;
    let state = app.zero();

    // Refuse to retry while a stream is still in flight for this message —
    // would race with the existing runner on the same row.
    if state.chat_jobs.cancel(&message_id).await {
        return Err(
            "a stream is still running for this message; cancel it first"
                .to_string()
                .into(),
        );
    }

    let row =
        sqlx::query("SELECT conversation_id FROM messages WHERE id = ? AND role = 'assistant'")
            .bind(&message_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Err(format!("no assistant message with id `{message_id}`").into());
    };
    let conversation_id: String = row.get("conversation_id");

    // Wipe the previous error content so the bubble starts empty again.
    chat::update_message(&state.db, &message_id, "", None)
        .await
        .map_err(|e| e.to_string())?;

    let app_clone = app.clone();
    let db = state.db.clone();
    let http = state.http.clone();
    let ovms = state.ovms.clone();
    let jobs = state.chat_jobs.clone();
    let conv_id = conversation_id.clone();
    let asst_id = message_id.clone();
    tauri::async_runtime::spawn(async move {
        runner::run(app_clone, db, http, ovms, jobs, conv_id, asst_id).await;
    });

    Ok(())
}
