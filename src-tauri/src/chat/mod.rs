//! Conversation + message CRUD. Wraps SQLite via runtime sqlx queries.

pub mod runner;

pub use crate::attachments::Attachment;
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub thinking: Option<String>,
    pub created_at: String,
    pub attachments: Option<Vec<Attachment>>,
    /// Per-turn capability flags the composer attached to this user
    /// message (web search, deep research, thinking opt-in, autonomous
    /// loop). The runner reads them from the latest `user` row to
    /// decide which tool subset to expose and whether to inject the
    /// model-family-specific thinking control token. Only ever set on
    /// user rows; always `None` on assistant / tool / system messages.
    /// Legacy rows persisted before this column existed deserialise as
    /// `None`; the runner falls back to slash-prefix parsing in that
    /// case so old chats keep working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_overrides: Option<TurnOverrides>,
}

/// Per-turn capability flags sent by the composer. Each field maps to
/// a previously slash-gated behaviour:
///   * `web` → unlock `web.search` + `web.read_page` (formerly `/web`).
///   * `research` → unlock `web.deep_research` + `web.read_page`
///     (formerly `/research`).
///   * `think` → opt **into** the model's reasoning trace for this
///     turn. Default for every chat is *no thinking* regardless of any
///     global setting; this flag is the per-turn opt-in (Gemma 4 gets
///     its `<|think|>` control token; families without an explicit
///     control token just see the absence of `/no_think`).
///   * `loop_mode` → autonomous-agent mode (formerly `/loop`).
///
/// `#[serde(default)]` on each field keeps the wire payload tolerant
/// of older frontends that don't send a particular flag yet, and lets
/// the runner treat "no override JSON at all" as `TurnOverrides::default()`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnOverrides {
    #[serde(default)]
    pub web: bool,
    #[serde(default)]
    pub research: bool,
    #[serde(default)]
    pub think: bool,
    /// Renamed on the wire because `loop` is a reserved word in some
    /// frontend languages and reads more clearly as a verb here.
    #[serde(default, rename = "loop")]
    pub loop_mode: bool,
}

impl TurnOverrides {
    /// `true` when every field is at its default (no opt-ins). Used to
    /// avoid persisting a redundant `{}` blob on every plain user
    /// message.
    pub fn is_default(&self) -> bool {
        *self == TurnOverrides::default()
    }
}

pub async fn create_conversation(pool: &SqlitePool, title: &str) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO conversations (id, title, model, created_at, updated_at)
         VALUES (?, ?, NULL, ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn list_conversations(pool: &SqlitePool) -> Result<Vec<Conversation>> {
    let rows = sqlx::query(
        "SELECT id, title, model, created_at, updated_at
         FROM conversations
         ORDER BY updated_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Conversation {
            id: r.get("id"),
            title: r.get("title"),
            model: r.try_get("model").ok(),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

pub async fn delete_conversation(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_messages(pool: &SqlitePool, conv_id: &str) -> Result<Vec<Message>> {
    let rows = sqlx::query(
        "SELECT id, conversation_id, role, content, thinking, attachments, turn_overrides, created_at
         FROM messages
         WHERE conversation_id = ?
         ORDER BY created_at ASC",
    )
    .bind(conv_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let attachments_json: Option<String> = r.try_get("attachments").ok();
            let attachments =
                attachments_json.and_then(|s| serde_json::from_str::<Vec<Attachment>>(&s).ok());
            let overrides_json: Option<String> = r.try_get("turn_overrides").ok();
            let turn_overrides = overrides_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<TurnOverrides>(s).ok());
            Message {
                id: r.get("id"),
                conversation_id: r.get("conversation_id"),
                role: r.get("role"),
                content: r.get("content"),
                thinking: r.try_get("thinking").ok(),
                created_at: r.get("created_at"),
                attachments,
                turn_overrides,
            }
        })
        .collect())
}

pub async fn insert_message(
    pool: &SqlitePool,
    conv_id: &str,
    role: &str,
    content: &str,
    thinking: Option<&str>,
    attachments: Option<&[Attachment]>,
    turn_overrides: Option<&TurnOverrides>,
) -> Result<Message> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let att_json = attachments.map(serde_json::to_string).transpose()?;
    // Skip the column entirely when no flags are set so the table
    // doesn't accumulate `{}` blobs for every plain message.
    let overrides_json = turn_overrides
        .filter(|o| !o.is_default())
        .map(serde_json::to_string)
        .transpose()?;
    sqlx::query(
        "INSERT INTO messages (id, conversation_id, role, content, thinking, attachments, turn_overrides, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(conv_id)
    .bind(role)
    .bind(content)
    .bind(thinking)
    .bind(att_json.as_deref())
    .bind(overrides_json.as_deref())
    .bind(&now)
    .execute(pool)
    .await?;

    sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(conv_id)
        .execute(pool)
        .await?;

    Ok(Message {
        id,
        conversation_id: conv_id.into(),
        role: role.into(),
        content: content.into(),
        thinking: thinking.map(|s| s.into()),
        created_at: now,
        attachments: attachments.map(|a| a.to_vec()),
        turn_overrides: turn_overrides.copied(),
    })
}

/// Replace the content + thinking of an existing message. Used by the
/// streaming runner to flush the final assistant turn once the stream ends.
pub async fn update_message(
    pool: &SqlitePool,
    message_id: &str,
    content: &str,
    thinking: Option<&str>,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE messages SET content = ?, thinking = ? WHERE id = ?")
        .bind(content)
        .bind(thinking)
        .bind(message_id)
        .execute(pool)
        .await?;

    // Bump the parent conversation so the sidebar re-sorts.
    sqlx::query(
        "UPDATE conversations
           SET updated_at = ?
         WHERE id = (SELECT conversation_id FROM messages WHERE id = ?)",
    )
    .bind(&now)
    .bind(message_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn conversation_model(pool: &SqlitePool, conv_id: &str) -> Result<Option<String>> {
    let row = sqlx::query("SELECT model FROM conversations WHERE id = ?")
        .bind(conv_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.try_get::<Option<String>, _>("model").ok().flatten()))
}

/// Read the per-conversation tool-disable list. Returns an empty `Vec`
/// when the column is `NULL` (the default — conversation inherits the
/// global Tools-page settings). Entries are catalog keys of the form
/// `<server_id>::<tool_name>`.
pub async fn conversation_disabled_tools(pool: &SqlitePool, conv_id: &str) -> Result<Vec<String>> {
    let row = sqlx::query("SELECT disabled_tools FROM conversations WHERE id = ?")
        .bind(conv_id)
        .fetch_optional(pool)
        .await?;
    let raw: Option<String> = row.and_then(|r| r.try_get("disabled_tools").ok()).flatten();
    Ok(raw
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default())
}

/// Replace the per-conversation tool-disable list. An empty `keys` slice
/// clears the column (back to "inherit settings").
pub async fn set_conversation_disabled_tools(
    pool: &SqlitePool,
    conv_id: &str,
    keys: &[String],
) -> Result<()> {
    let payload = if keys.is_empty() {
        None
    } else {
        Some(serde_json::to_string(keys)?)
    };
    sqlx::query("UPDATE conversations SET disabled_tools = ? WHERE id = ?")
        .bind(payload)
        .bind(conv_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Read the per-conversation sampling override. Returns the default
/// (all-`None`) [`SamplingConfig`] when the column is `NULL` or holds
/// invalid JSON — i.e. "no override, inherit provider/profile". Parse
/// failures are logged and treated as the default rather than bubbled
/// up so a corrupt blob can't take a chat hostage; the user can fix it
/// by re-saving from the popover.
pub async fn conversation_sampling(
    pool: &SqlitePool,
    conv_id: &str,
) -> Result<crate::settings::SamplingConfig> {
    let row = sqlx::query("SELECT sampling FROM conversations WHERE id = ?")
        .bind(conv_id)
        .fetch_optional(pool)
        .await?;
    let raw: Option<String> = row.and_then(|r| r.try_get("sampling").ok()).flatten();
    let Some(raw) = raw else {
        return Ok(crate::settings::SamplingConfig::default());
    };
    match serde_json::from_str::<crate::settings::SamplingConfig>(&raw) {
        Ok(s) => Ok(s),
        Err(e) => {
            tracing::warn!(
                "conversation {conv_id}: sampling column unparsable ({e:#}); treating as default"
            );
            Ok(crate::settings::SamplingConfig::default())
        }
    }
}

/// Replace the per-conversation sampling override. A fully-empty
/// [`SamplingConfig`] (every field `None`) clears the column (back to
/// "inherit provider settings") so the row doesn't accumulate empty
/// JSON blobs over time.
pub async fn set_conversation_sampling(
    pool: &SqlitePool,
    conv_id: &str,
    sampling: &crate::settings::SamplingConfig,
) -> Result<()> {
    let payload = if *sampling == crate::settings::SamplingConfig::default() {
        None
    } else {
        Some(serde_json::to_string(sampling)?)
    };
    sqlx::query("UPDATE conversations SET sampling = ? WHERE id = ?")
        .bind(payload)
        .bind(conv_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Pin a model id to a conversation, or clear it when `model` is `None`.
/// Bumps `updated_at` so the sidebar reflects the change immediately.
pub async fn set_conversation_model(
    pool: &SqlitePool,
    conv_id: &str,
    model: Option<&str>,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE conversations SET model = ?, updated_at = ? WHERE id = ?")
        .bind(model)
        .bind(&now)
        .bind(conv_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Rename a conversation. The new title is stored verbatim; callers are
/// responsible for trimming / truncating to whatever the UI considers a
/// reasonable length. Does **not** bump `updated_at` — a rename shouldn't
/// re-order the sidebar away from where the user expects to find the chat.
pub async fn set_conversation_title(pool: &SqlitePool, conv_id: &str, title: &str) -> Result<()> {
    sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
        .bind(title)
        .bind(conv_id)
        .execute(pool)
        .await?;
    Ok(())
}
