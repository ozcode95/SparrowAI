//! Streaming chat runner.
//!
//! Flow per user turn:
//!
//! ```text
//! chat_send_message  ── insert user msg ─┐
//!                                        │
//!                       insert empty     ▼
//!                       assistant msg ── return Message to UI
//!                                        │
//!                                        ▼
//!                       spawn run(...)   ──► provider.chat_stream(...)
//!                                            │
//!                                            ▼
//!                       drain chunks ──► emit `chat://delta`
//!                                        accumulate buffers
//!                                        on end → update_message + `chat://done`
//!                                        on err → `chat://error`
//! ```
//!
//! Cancellation: each in-flight assistant message id is registered in
//! `ChatJobs`. `chat_cancel(message_id)` flips a `Notify`, the runner aborts
//! the streaming task, persists whatever it already collected, and emits a
//! final `chat://done` so the UI's spinner clears.

use crate::attachments;
use crate::chat::{self, Message};
use crate::events;
use crate::llm::openai_compat::UpstreamError;
use crate::llm::{
    ovms::OvmsProvider, ChatChunk, ChatMessage, ChatRequest, ContentPart, ImageUrl, LlmProvider,
    ToolCall,
};
use crate::mcp::catalog::{self as mcp_catalog, EnabledTool};
use crate::mcp::client as mcp_client;
use crate::mcp::{self as mcp, BUILTIN_SERVER_ID};
use crate::ovms::{OvmsController, OvmsStatus};
use crate::settings::{SamplingConfig, Settings};
use crate::skills;
use crate::state::AppStateExt;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot, Notify, RwLock};

/// Per-process registry of in-flight chat completions. Keyed by the assistant
/// message id so `chat_cancel(message_id)` can find the right stream to abort.
#[derive(Default)]
pub struct ChatJobs {
    inner: RwLock<HashMap<String, Arc<Notify>>>,
}

impl ChatJobs {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(&self, message_id: String) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.inner.write().await.insert(message_id, notify.clone());
        notify
    }

    pub async fn finish(&self, message_id: &str) {
        self.inner.write().await.remove(message_id);
    }

    /// Returns `true` if a job was found and notified. Uses `notify_one`
    /// (which stores a permit when no waiter is parked) rather than
    /// `notify_waiters` so cancellation that arrives *between* agent-loop
    /// rounds is still observed by the next round's `cancel.notified()`.
    pub async fn cancel(&self, message_id: &str) -> bool {
        if let Some(n) = self.inner.read().await.get(message_id).cloned() {
            n.notify_one();
            true
        } else {
            false
        }
    }
}

/// Per-process registry of pending tool-confirmation prompts.
///
/// When the agent loop wants to invoke a destructive MCP tool while the
/// `destructive_tool_confirm` setting is on, it registers a oneshot here
/// keyed by a fresh `call_id`, emits a `chat://tool-confirm` event with
/// that id + call details, and waits on the receiver. The frontend
/// resolves the prompt with `chat_tool_confirm(call_id, allow)`, which
/// flows back through [`ToolConfirms::resolve`].
#[derive(Default)]
pub struct ToolConfirms {
    inner: RwLock<HashMap<String, oneshot::Sender<bool>>>,
}

impl ToolConfirms {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve a slot for `call_id` and hand back the receiver the runner
    /// awaits. Dropping the returned receiver while the entry is still
    /// registered just turns the eventual `tx.send` into a no-op — the
    /// receiver-side `tokio::select!` already handles the race.
    pub async fn register(&self, call_id: String) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.inner.write().await.insert(call_id, tx);
        rx
    }

    /// Forget a pending confirm without notifying. Called by the runner
    /// when cancellation wins the race so dangling senders don't leak.
    pub async fn forget(&self, call_id: &str) {
        self.inner.write().await.remove(call_id);
    }

    /// Frontend callback path. Returns `true` if the confirm was known
    /// and the decision was delivered; `false` when there was no pending
    /// prompt (e.g. duplicate click, chat already cancelled).
    pub async fn resolve(&self, call_id: &str, allow: bool) -> bool {
        if let Some(tx) = self.inner.write().await.remove(call_id) {
            let _ = tx.send(allow);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct DeltaPayload<'a> {
    conversation_id: &'a str,
    message_id: &'a str,
    delta: &'a str,
    thinking: bool,
}

/// Payload for [`events::CHAT_REWRITE`]. The frontend handler
/// *replaces* the message's `content` with `content` (rather than
/// appending like [`DeltaPayload`]). Used to scrub legacy
/// ```tool_use``` fenced-JSON that already streamed into the bubble
/// before the runner recognised it as a tool-call protocol marker.
#[derive(Debug, Clone, Serialize)]
struct RewritePayload<'a> {
    conversation_id: &'a str,
    message_id: &'a str,
    content: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct DonePayload<'a> {
    conversation_id: &'a str,
    message_id: &'a str,
    /// `true` when cancellation cut the stream short.
    cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ErrorPayload<'a> {
    conversation_id: &'a str,
    message_id: &'a str,
    error: &'a str,
    /// Machine-tag for the kind of failure. The frontend uses this to pick
    /// an icon / colour without parsing `error`. Mirrors
    /// [`ChatErrorKind`].
    kind: &'static str,
    /// One-line, user-actionable hint. `None` when we can't guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
    /// Provider kind we tried to reach (`ovms`, `vllm`, `ollama`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_kind: Option<String>,
    /// Base URL we attempted (without auth headers).
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    /// True when re-running the same turn might succeed (transient HTTP
    /// failure, OVMS still warming up, etc.). The UI surfaces a retry
    /// affordance only when this is set.
    retryable: bool,
}

/// Stable string tags for the structured error event. Keep in sync with the
/// frontend `ChatErrorKind` union in `src/stores/chat.ts`.
#[derive(Debug, Clone, Copy)]
enum ChatErrorKind {
    NoActiveProvider,
    UnsupportedProvider,
    NoModelSelected,
    OvmsNotRunning,
    UpstreamUnreachable,
    UpstreamHttp,
    Other,
}

impl ChatErrorKind {
    fn tag(self) -> &'static str {
        match self {
            Self::NoActiveProvider => "no_active_provider",
            Self::UnsupportedProvider => "unsupported_provider",
            Self::NoModelSelected => "no_model_selected",
            Self::OvmsNotRunning => "ovms_not_running",
            Self::UpstreamUnreachable => "upstream_unreachable",
            Self::UpstreamHttp => "upstream_http",
            Self::Other => "other",
        }
    }

    fn retryable(self) -> bool {
        matches!(
            self,
            Self::UpstreamUnreachable | Self::OvmsNotRunning | Self::UpstreamHttp | Self::Other
        )
    }
}

/// Carries everything the runner needs to drive a structured error event.
/// Built by [`classify`] from whatever escaped `run_inner`, then surfaced
/// both to the persisted assistant message and the `chat://error` payload.
struct ChatErrorReport {
    kind: ChatErrorKind,
    message: String,
    hint: Option<String>,
    provider_kind: Option<String>,
    base_url: Option<String>,
}

/// Entry point. Called from `chat_send_message` after the user + empty
/// assistant rows have been inserted. Owns the streaming lifecycle.
pub async fn run(
    app: AppHandle,
    db: SqlitePool,
    http: reqwest::Client,
    ovms: Arc<OvmsController>,
    jobs: Arc<ChatJobs>,
    conversation_id: String,
    assistant_message_id: String,
) {
    let cancel = jobs.register(assistant_message_id.clone()).await;

    let outcome = run_inner(
        &app,
        &db,
        &http,
        &ovms,
        &conversation_id,
        &assistant_message_id,
        cancel,
    )
    .await;

    jobs.finish(&assistant_message_id).await;

    match outcome {
        Ok(cancelled) => {
            let _ = app.emit(
                events::CHAT_DONE,
                DonePayload {
                    conversation_id: &conversation_id,
                    message_id: &assistant_message_id,
                    cancelled,
                },
            );
        }
        Err(report) => {
            tracing::warn!(
                "chat run failed [{}]: {}",
                report.kind.tag(),
                report.message
            );
            // Persist whatever message exists with the error appended so the
            // user can see what went wrong in-line.
            let _ = chat::update_message(
                &db,
                &assistant_message_id,
                &format!("[error] {}", report.message),
                None,
            )
            .await;
            let _ = app.emit(
                events::CHAT_ERROR,
                ErrorPayload {
                    conversation_id: &conversation_id,
                    message_id: &assistant_message_id,
                    error: &report.message,
                    kind: report.kind.tag(),
                    hint: report.hint.clone(),
                    provider_kind: report.provider_kind.clone(),
                    base_url: report.base_url.clone(),
                    retryable: report.kind.retryable(),
                },
            );
        }
    }
}

async fn run_inner(
    app: &AppHandle,
    db: &SqlitePool,
    http: &reqwest::Client,
    ovms: &Arc<OvmsController>,
    conversation_id: &str,
    assistant_message_id: &str,
    cancel: Arc<Notify>,
) -> std::result::Result<bool, ChatErrorReport> {
    // ─── 1. Resolve provider + model ─────────────────────────────────────
    let settings = Settings::load().await.map_err(|e| ChatErrorReport {
        kind: ChatErrorKind::Other,
        message: format!("load settings: {e:#}"),
        hint: Some("settings file may be unreadable; check the zero data dir".into()),
        provider_kind: None,
        base_url: None,
    })?;
    let provider_cfg = settings
        .active_provider()
        .ok_or_else(|| ChatErrorReport {
            kind: ChatErrorKind::NoActiveProvider,
            message: "no active provider configured".into(),
            hint: Some("open Settings and pick an active provider".into()),
            provider_kind: None,
            base_url: None,
        })?
        .clone();

    let ovms_info = ovms.info().await;
    let model = chat::conversation_model(db, conversation_id)
        .await
        .ok()
        .flatten()
        .or(settings.default_model.clone())
        .or(ovms_info.loaded_model.clone())
        .ok_or_else(|| ChatErrorReport {
            kind: ChatErrorKind::NoModelSelected,
            message: "no model selected".into(),
            hint: Some(
                "pick a model in the chat header, set settings.default_model, or load one on the Server page".into(),
            ),
            provider_kind: Some(provider_cfg.kind.clone()),
            base_url: Some(provider_cfg.base_url.clone()),
        })?;

    let base_url = if provider_cfg.base_url.is_empty() {
        ovms_info.base_url.clone()
    } else {
        provider_cfg.base_url.clone()
    };

    // If the active provider is the local OVMS instance, make sure the
    // model the conversation wants is in the live serving set. OVMS
    // routes chat completions by the `model` field — sending a name
    // OVMS doesn't have loaded yields `404 Mediapipe graph definition
    // with requested name is not found`. `OvmsController::start` is
    // idempotent when the model is already loaded and otherwise either
    // adds it to the live set via TFS reload (no restart) or spawns
    // OVMS with the full persisted set when it's not running yet.
    if provider_cfg.kind == "ovms" {
        let model_loaded = ovms_info.loaded_models.iter().any(|m| m.id == model);
        let not_running = !matches!(ovms_info.status, OvmsStatus::Running);
        if !model_loaded || not_running {
            // Don't try to auto-start during install / mid-transition —
            // surface a clear status error instead.
            if matches!(
                ovms_info.status,
                OvmsStatus::NotInstalled | OvmsStatus::Installing | OvmsStatus::Stopping
            ) {
                return Err(ChatErrorReport {
                    kind: ChatErrorKind::OvmsNotRunning,
                    message: format!(
                        "OVMS is not ready (status: {:?}){}",
                        ovms_info.status,
                        ovms_info
                            .last_error
                            .as_deref()
                            .map(|e| format!(" — {e}"))
                            .unwrap_or_default()
                    ),
                    hint: Some(
                        "open the Server page and finish installing / starting OVMS, or pick a different provider in Settings"
                            .into(),
                    ),
                    provider_kind: Some(provider_cfg.kind.clone()),
                    base_url: Some(base_url.clone()),
                });
            }

            tracing::info!(
                "chat: staging OVMS model `{model}` for conversation {conversation_id} (currently loaded: {:?})",
                ovms_info
                    .loaded_models
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>()
            );
            if let Err(e) = ovms.start(&model).await {
                return Err(ChatErrorReport {
                    kind: ChatErrorKind::OvmsNotRunning,
                    message: format!("OVMS could not load model `{model}`: {e:#}"),
                    hint: Some(
                        "open the Server page and check the OVMS logs, or pick a different model"
                            .into(),
                    ),
                    provider_kind: Some(provider_cfg.kind.clone()),
                    base_url: Some(base_url.clone()),
                });
            }
        }
    }

    // ─── 2. Build messages + fetch MCP tool catalog ─────────────────────
    // The tool catalog is best-effort: per-server probe failures are logged
    // inside `mcp_catalog::fetch_enabled` and just omit that server's tools
    // from the system prompt — they're never fatal to the chat.
    let mcp_cache = app.zero().mcp_cache.clone();
    let mut tools = mcp_catalog::fetch_enabled(app, http, &settings, &mcp_cache).await;

    // Per-conversation override: drop any tool the user explicitly turned
    // off in this chat's Tools popover. Catalog keys are `<server>::<tool>`
    // (matching `chat_set_disabled_tools` and the frontend popover).
    // Force-enabled tools (currently just `tools.list`) are pinned on so
    // lazy discovery can't be broken by a stale per-chat override left
    // over from before the user flipped the setting on.
    match chat::conversation_disabled_tools(db, conversation_id).await {
        Ok(disabled) if !disabled.is_empty() => {
            tools.retain(|t| {
                if t.server_id == BUILTIN_SERVER_ID
                    && mcp::tools::discovery::is_force_enabled(&t.schema.name)
                {
                    return true;
                }
                let key = format!("{}::{}", t.server_id, t.schema.name);
                !disabled.iter().any(|d| d == &key)
            });
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(
            "chat: failed to read disabled-tools for {conversation_id}: {e:#} — using full catalog"
        ),
    }

    let history = chat::list_messages(db, conversation_id)
        .await
        .map_err(|e| classify_other(&e, &provider_cfg.kind, &base_url))?;

    // Resolve the per-turn capability flags for this turn. Prefer the
    // structured `turn_overrides` column the composer now writes (web
    // search, deep research, thinking opt-in, autonomous loop); fall
    // back to the legacy slash-prefix scan for messages persisted
    // before the column existed so existing chats keep working with
    // the same behaviour they had before.
    let latest_user_msg = history.iter().rev().find(|m| m.role == "user");
    let overrides = resolve_turn_overrides(latest_user_msg);

    let loop_mode = overrides.loop_mode;

    // Web-tool gating. The built-in `web.*` tools can reach arbitrary
    // third-party servers, so we keep them locked out of the catalog by
    // default and only unlock them when the user explicitly asks for
    // browsing on this turn (composer `+` menu toggles, formerly
    // `/web` / `/research` prefixes). Locking happens here (after the
    // per-chat disabled-tools filter) so the model never even sees the
    // tools in its system prompt on ungated turns — it can't be coaxed
    // into calling something it doesn't know exists. Non-builtin
    // servers are untouched: those are user-configured MCP endpoints
    // and have their own enable toggle on the Tools page. The policy
    // itself lives in [`mcp::tools::web::WebUnlocks`] so the UI filter
    // in `mcp_list_builtins` stays in sync with what's reachable here.
    let web_unlocks = mcp::tools::web::WebUnlocks {
        search: overrides.web,
        research: overrides.research,
    };
    tools.retain(|t| {
        if t.server_id != BUILTIN_SERVER_ID {
            return true;
        }
        web_unlocks.allows(&t.schema.name)
    });

    // OVMS only emits structured `delta.tool_calls` when the loaded
    // model has a `tool_parser` configured. As of OVMS v2026.2
    // `src/llm/io_processing/` ships parsers for llama3, hermes3, phi4,
    // mistral, qwen3coder, gpt-oss, devstral, lfm2, and gemma4 — every
    // family with a directory there *except* plain `qwen3`, which has
    // only a reasoning_parser and reuses `hermes3` for tool calls per
    // the OVMS docs. For families OVMS hasn't shipped a parser for
    // (Granite, generic Llama-2, custom fine-tunes, ...) the model
    // still tries to call tools — it just emits its own template
    // tokens, which leak into `content` as garbage like
    // `<tool_call>...</tool_call>`. To stay useful in that case we:
    //   * Don't ship the OpenAI `tools` array (so OVMS doesn't even
    //     forward the catalogue to the chat template).
    //   * Inject the legacy ```tool_use``` fenced-JSON instructions
    //     into the system prompt so the model has a clean text protocol
    //     to follow that we can parse on our side.
    // For non-OVMS providers (vLLM, ollama) we assume tool-call parsing
    // is supported, since both expose the OpenAI-style `tool_calls`
    // field on the response.
    let native_tools_supported = if provider_cfg.kind == "ovms" {
        // Look up the tool_parser for the *specific* model this
        // conversation is using, not just whichever text-gen model the
        // controller flagged as "primary" — with multi-model serving
        // those can differ.
        //
        // Re-read info after the staging step above so a freshly
        // added model is visible here.
        let info = ovms.info().await;
        info.loaded_models
            .iter()
            .find(|m| m.id == model)
            .and_then(|m| m.tool_parser.as_deref())
            .is_some()
    } else {
        true
    };

    // Get the reasoning_parser for qwen3/qwen3.5 detection
    let reasoning_parser = if provider_cfg.kind == "ovms" {
        let info = ovms.info().await;
        info.loaded_models
            .iter()
            .find(|m| m.id == model)
            .and_then(|m| m.reasoning_parser.clone())
    } else {
        None
    };

    // Determine if we should disable thinking via chat_template_kwargs.
    // This happens when:
    //   1. The model has reasoning_parser set to "qwen3" (covers both Qwen3 and Qwen3.5)
    //   2. Thinking is NOT enabled for this turn
    // For Qwen models, we pass {"enable_thinking": false} in chat_template_kwargs
    // instead of appending /no_think to the prompt text.
    let should_disable_thinking =
        matches!(reasoning_parser.as_deref(), Some("qwen3")) && !overrides.think;

    let mut messages = build_request_messages(
        &history,
        &settings,
        &tools,
        loop_mode,
        overrides.think,
        native_tools_supported,
        settings.lazy_tool_discovery,
        &model,
    )
    .await
    .map_err(|e| classify_other(&e, &provider_cfg.kind, &base_url))?;

    // Per-family sampling + multimodal + thinking-token defaults.
    // Resolved once because the model is fixed for the whole turn (the
    // agent loop never switches models mid-conversation).
    let profile = model_profile(&model);

    // Layered sampling overrides. Precedence is conversation → provider
    // → model profile so the most-specific knob always wins. The chat
    // popover writes the conversation layer; the Settings page writes
    // the provider layer; neither needs to know about the other.
    let conv_sampling = chat::conversation_sampling(db, conversation_id)
        .await
        .unwrap_or_default();
    let sampling = resolve_sampling(&conv_sampling, &provider_cfg.sampling, profile);

    // ─── 3. Validate the provider kind once ─────────────────────────────
    if !matches!(provider_cfg.kind.as_str(), "ovms" | "vllm" | "ollama") {
        return Err(ChatErrorReport {
            kind: ChatErrorKind::UnsupportedProvider,
            message: format!("unsupported provider kind: {}", provider_cfg.kind),
            hint: Some(
                "only `ovms`, `vllm`, `ollama` are supported right now — change the provider kind in Settings".into(),
            ),
            provider_kind: Some(provider_cfg.kind.clone()),
            base_url: Some(base_url.clone()),
        });
    }

    // ─── 4. Agent loop ──────────────────────────────────────────────────
    // Each iteration streams one LLM round into the same assistant
    // message id (so deltas keep flowing into one bubble), then checks
    // for tool calls. The model can request tools two ways:
    //   * Native OpenAI protocol (preferred): structured `tool_calls`
    //     on `finish_reason == "tool_calls"`. OVMS forwards these to
    //     the model via the chat template's configured `tool_parser`.
    //   * Legacy fenced JSON (fallback): ```tool_use``` block embedded
    //     in the assistant content. Only used by small models that
    //     ignore the structured protocol; the runner converts it into
    //     a synthetic ToolCall so dispatch is uniform.
    // Results are spliced back as proper `role: "tool"` messages bound
    // to the assistant's `tool_call_id`, so the model sees the answer
    // to its previous request and either replies or chains another
    // call.
    //
    // Two iteration budgets:
    //   * `/loop` opt-in → full autonomous mode bounded by
    //     `settings.agent_max_iterations` so a confused model can't
    //     grind forever.
    //   * default     → exactly two rounds (one tool-call dispatch +
    //     one follow-up where the model summarises the result). With
    //     proper protocol this cap is rarely hit because a well-behaved
    //     model stops calling tools once it has the answer; it's just
    //     a safety net for legacy fenced-JSON models that don't
    //     understand structured `tool` messages.
    //
    // Lazy tool-discovery: when active, the first round of this turn
    // ships only the `tools.list` built-in in the OpenAI `tools` array
    // instead of the full catalogue. Subsequent rounds add a tool to the
    // advertised set the moment the model asks for it by name — either by
    // calling `tools.list({"name": "<tool>"})` to fetch the schema, or by
    // dispatching the tool directly with guessed args (in which case we
    // still want it pinned for any follow-up calls in the same turn).
    // This keeps the OpenAI `tools` array down to the discovery placeholder
    // plus the tool(s) the model is actively using, instead of re-shipping
    // the entire catalogue on every round after the first `tools.list`.
    //
    // The set is keyed by `<server_id>/<tool_name>` so the comparison
    // stays unambiguous when two MCP servers happen to publish a tool
    // under the same short name.
    //
    // `lazy_mode_active` is computed once: it's only true when the user
    // has the feature switched on *and* `tools.list` itself is enabled
    // (it's force-enabled in the UI but the safety-net check stays so a
    // hand-edited settings file can't break the agent loop).
    let lazy_mode_active = settings.lazy_tool_discovery
        && tools.iter().any(|t| {
            t.server_id == BUILTIN_SERVER_ID
                && t.schema.name == mcp::tools::discovery::TOOLS_LIST_NAME
        });
    let mut revealed_tools: HashSet<String> = HashSet::new();

    let max_iters = if loop_mode {
        settings.agent_max_iterations.max(1) as usize
    } else if lazy_mode_active {
        // Lazy discovery now reveals tools one at a time, so a
        // protocol-respecting model spends up to two rounds on
        // discovery before the actual dispatch:
        //   round 1 → `tools.list({})`           (catalogue summary)
        //   round 2 → `tools.list({name: "X"})`  (schema for X)
        //   round 3 → `X(...)`                   (real dispatch)
        //   round 4 → free-form summary of the result
        // A 4-round budget covers the worst case; well-behaved models
        // that skip the schema-fetch step (jumping straight from the
        // catalogue to dispatch) just finish a round early.
        4
    } else {
        2
    };
    let mut full_content = String::new();
    let mut cancelled = false;
    let mut tool_calls_made: usize = 0;

    'agent: for iter in 0..max_iters {
        let provider: Box<dyn LlmProvider> = Box::new(OvmsProvider {
            base_url: base_url.clone(),
            http: http.clone(),
        });
        // Pick the catalogue the *model* sees this round. The full
        // `tools` slice is still used downstream for dispatch (the
        // model can call something it learned about via tools.list
        // even though the function definition wasn't in this round's
        // request) — only the advertised set narrows in lazy mode.
        //
        // In lazy mode we always expose `tools.list` plus whatever
        // tools the model has already requested by name. Initially the
        // revealed set is empty, so only the discovery tool ships; once
        // the model has called `tools.list({name: "X"})` (or dispatched
        // `X` directly), `X` joins the advertised set on the next
        // round.
        let round_tools: Vec<EnabledTool> = if lazy_mode_active {
            tools
                .iter()
                .filter(|t| {
                    let is_discovery = t.server_id == BUILTIN_SERVER_ID
                        && t.schema.name == mcp::tools::discovery::TOOLS_LIST_NAME;
                    if is_discovery {
                        return true;
                    }
                    let key = format!("{}/{}", t.server_id, t.schema.name);
                    revealed_tools.contains(&key)
                })
                .cloned()
                .collect()
        } else {
            tools.clone()
        };
        // Ship the live catalogue as the OpenAI `tools` array so OVMS
        // can hand it to the chat template + tool parser. When no
        // tools are enabled we omit the field entirely (some strict
        // servers reject `"tools": []`). When the loaded model has no
        // `tool_parser` configured we *also* omit the field, because
        // OVMS will surface the catalogue to the model anyway via the
        // chat template but won't be able to parse the model's reply
        // back into structured `tool_calls` — which results in raw
        // template tokens leaking through `delta.content`. The system
        // prompt has been augmented in that case to teach the model
        // the legacy fenced-JSON protocol instead.
        let tool_defs = if round_tools.is_empty() || !native_tools_supported {
            None
        } else {
            Some(mcp_catalog::to_tool_defs(&round_tools))
        };
        let req = ChatRequest {
            model: model.clone(),
            messages: messages.clone(),
            temperature: Some(sampling.temperature),
            max_tokens: None,
            top_p: sampling.top_p,
            top_k: sampling.top_k,
            stream: true,
            tools: tool_defs,
            tool_choice: None,
            chat_template_kwargs: if should_disable_thinking {
                Some(serde_json::json!({"enable_thinking": false}))
            } else {
                None
            },
        };

        let outcome = stream_round(
            app,
            db,
            provider,
            req,
            conversation_id,
            assistant_message_id,
            &full_content,
            cancel.clone(),
        )
        .await
        .map_err(|e| classify(&e, &provider_cfg.kind, &base_url))?;

        // Per-round reasoning is moved out of the separate `thinking`
        // field and inlined into `full_content` as a `[thinking] … [/thinking]`
        // block so the chat bubble shows reasoning *in time order* with
        // the tool calls that follow it, instead of collapsing every
        // round's thinking into one accumulated block at the top of the
        // turn. The block is pushed before `visible_content` because the
        // model thinks before deciding what to say or which tool to call.
        let thinking_block = render_thinking_block(&outcome.thinking);

        if outcome.cancelled {
            cancelled = true;
            full_content.push_str(&thinking_block);
            full_content.push_str(&outcome.content);
            if !thinking_block.is_empty() {
                emit_rewrite(app, conversation_id, assistant_message_id, &full_content);
            }
            break 'agent;
        }

        // Two paths a model can take to request tools:
        //   1. Native OpenAI protocol — `tool_calls` arrive in the
        //      structured `delta.tool_calls` field and the runner sees
        //      them on the terminating chunk's `tool_calls` vector.
        //   2. Legacy fenced-JSON fallback — small / poorly-tuned local
        //      models occasionally ignore the protocol and emit a
        //      ```tool_use``` block in the assistant content instead.
        // We honour the structured path first because that's what the
        // OVMS docs and our system prompt point the model at.
        let mut pending_calls: Vec<ToolCall> = outcome.tool_calls.clone();
        let mut visible_content = outcome.content.clone();
        let mut consumed_legacy_fence = false;

        if pending_calls.is_empty() {
            if let Some(legacy) = parse_tool_call(&outcome.content) {
                // Trim the fence (and anything after it) out of the
                // visible assistant text so the chat bubble doesn't show
                // raw protocol leakage.
                visible_content = outcome.content[..legacy.fence_start].to_string();
                pending_calls.push(legacy_to_tool_call(&legacy));
                consumed_legacy_fence = true;
            }
        }

        if pending_calls.is_empty() {
            // Plain text response — no tool action, this is the final
            // assistant turn.
            full_content.push_str(&thinking_block);
            full_content.push_str(&visible_content);
            if !thinking_block.is_empty() {
                emit_rewrite(app, conversation_id, assistant_message_id, &full_content);
            }
            break 'agent;
        }

        // Anything the model said *before* deciding to call a tool is
        // real assistant text and worth showing.
        full_content.push_str(&thinking_block);
        full_content.push_str(&visible_content);

        // The UI buffer has the live message.content built up from the
        // raw streamed deltas. We now need to (a) splice the inline
        // thinking block in front of `visible_content`, and (b) clear
        // the live `message.thinking` field (which the frontend
        // populated from the per-round thinking deltas and which has
        // now been replaced by the inline block). Both are accomplished
        // by a single `chat://rewrite` event — the frontend handler
        // resets `thinking: null` whenever it processes one.
        //
        // We also rewrite when we consumed a legacy fenced-JSON call,
        // for the original reason: the un-trimmed assistant text was
        // already streamed into the bubble and needs to be replaced
        // with the trimmed canonical view before we append tool banners
        // on top.
        if !thinking_block.is_empty() || consumed_legacy_fence {
            emit_rewrite(app, conversation_id, assistant_message_id, &full_content);
        }

        let last_round = iter + 1 >= max_iters;

        // Persist the assistant turn that requested the tool call(s)
        // *in the working message history* so the next LLM round sees
        // them with the right `tool_call_id` linkage. The DB row keeps
        // accumulating the rendered banners + results so the UI bubble
        // looks coherent.
        let assistant_msg_for_history = if consumed_legacy_fence {
            // Legacy path: the call lives inside the assistant content,
            // so just replay the visible+fenced original text as a
            // plain assistant message.
            ChatMessage::text("assistant", outcome.content.clone())
        } else {
            ChatMessage::assistant_tool_calls(visible_content.clone(), pending_calls.clone())
        };
        messages.push(assistant_msg_for_history);

        // Dispatch every requested call in order. Each one yields a
        // visible banner + result block streamed into the same
        // assistant bubble, plus a structured `tool` message spliced
        // back into the working history for the next LLM round.
        for call in &pending_calls {
            tool_calls_made += 1;

            // Lazy-mode bookkeeping: incrementally reveal tools as the
            // model asks for them so the next round's `tools` array
            // stays focused on what's actively in play instead of
            // re-shipping the entire catalogue.
            //
            //   * `tools.list({name: "X"})` → reveal X (the model has
            //     just fetched its schema in preparation for calling
            //     it).
            //   * `tools.list({})`           → no-op; the model only
            //     asked for the catalogue summary and hasn't picked a
            //     tool yet.
            //   * any other tool call        → reveal that tool so
            //     follow-up calls in the same turn don't have to go
            //     through `tools.list` again.
            //
            // We update `revealed_tools` *before* dispatch so a mid-call
            // cancellation still leaves the right set primed for the
            // next round.
            if lazy_mode_active {
                if let Some(resolved) =
                    mcp_catalog::resolve_function_name(&tools, &call.function.name)
                {
                    let is_discovery = resolved.server_id == BUILTIN_SERVER_ID
                        && resolved.schema.name == mcp::tools::discovery::TOOLS_LIST_NAME;
                    if is_discovery {
                        if let Some(requested_name) =
                            parse_tools_list_name_arg(&call.function.arguments)
                        {
                            if let Some(target) = resolve_revealed_target(&tools, &requested_name) {
                                revealed_tools
                                    .insert(format!("{}/{}", target.server_id, target.schema.name));
                            }
                        }
                    } else {
                        revealed_tools
                            .insert(format!("{}/{}", resolved.server_id, resolved.schema.name));
                    }
                }
            }

            let banner = render_tool_call_banner(&tools, call);
            emit_delta(app, conversation_id, assistant_message_id, &banner);
            full_content.push_str(&banner);

            let result_text = if last_round {
                if loop_mode {
                    format!(
                        "[tool budget exhausted after {tool_calls_made} call(s) — \
                         increase `agent_max_iterations` in Settings to allow more]"
                    )
                } else {
                    format!(
                        "[further tool calls suppressed after {tool_calls_made} call(s) — \
                         prefix your message with `/loop` to let the assistant chain \
                         multiple tool calls autonomously]"
                    )
                }
            } else {
                let (text, was_cancelled) = run_tool_call(
                    app,
                    http,
                    &settings,
                    &tools,
                    call,
                    conversation_id,
                    assistant_message_id,
                    cancel.clone(),
                )
                .await;
                if was_cancelled {
                    cancelled = true;
                }
                text
            };

            let result_block = render_result_block(&result_text);
            emit_delta(app, conversation_id, assistant_message_id, &result_block);
            full_content.push_str(&result_block);

            // Splice the tool's result back into the conversation as a
            // proper `role: "tool"` message bound to the assistant's
            // call id. The model can then correlate result → request and
            // decide whether more tool calls are needed.
            messages.push(ChatMessage::tool_result(
                call.id.clone(),
                call.function.name.clone(),
                result_text,
            ));

            if cancelled {
                break;
            }
        }

        // Persist progress so a crash mid-loop doesn't lose the
        // partial turn the user can already see streaming in. Thinking
        // is now inlined into `full_content` so the DB row's thinking
        // column stays NULL on every persist.
        let _ = chat::update_message(db, assistant_message_id, &full_content, None).await;

        if cancelled || last_round {
            break 'agent;
        }
    }

    // ─── 5. Final write ──────────────────────────────────────
    if cancelled && full_content.is_empty() {
        full_content.push_str("[cancelled]");
    }

    // Wire-log the assembled final assistant message. This is the
    // post-processed view the user sees in the bubble and what we
    // persist to the DB: tool-call banners, result blocks, inlined
    // thinking blocks, legacy fence stripping, and the [cancelled]
    // fallback have all been applied at this point. Complements the
    // per-upstream-turn `llm::wire` log in `openai_compat` so a single
    // tail can show both "what the model actually said" (raw) and
    // "what the chat layer ended up with" (post-processed).
    tracing::debug!(
        target: "chat::final",
        conversation_id = %conversation_id,
        message_id = %assistant_message_id,
        cancelled = cancelled,
        tool_calls = tool_calls_made,
        content_len = full_content.len(),
        "assistant final message\ncontent: {}",
        full_content,
    );

    chat::update_message(db, assistant_message_id, &full_content, None)
        .await
        .map_err(|e| classify_other(&e, &provider_cfg.kind, &base_url))?;

    Ok(cancelled)
}

/// Outcome of one LLM round. The runner stitches multiple of these
/// together to drive the agent loop.
struct RoundOutcome {
    content: String,
    /// Per-round thinking text (already streamed to the UI). Cumulative
    /// concatenation is left to the caller.
    thinking: String,
    cancelled: bool,
    /// Tool calls the model emitted via the native OpenAI protocol
    /// (`delta.tool_calls`). Empty when the model only produced free
    /// text — the caller then falls back to scanning [`content`] for a
    /// legacy fenced-JSON tool_use block.
    tool_calls: Vec<ToolCall>,
}

/// Stream a single LLM round into the open assistant message id. Deltas
/// are emitted as they arrive; periodic DB flushes write
/// `full_content_prefix + acc.content` so a crash mid-stream preserves
/// whatever the user has already seen across prior rounds too — not
/// just this one. The thinking buffer is **not** prefixed: per-round
/// reasoning is moved into `full_content` as an inline
/// `[thinking] … [/thinking]` block by [`run_inner`] after the round
/// finishes, so the live `message.thinking` field only ever carries the
/// reasoning from the round that's currently streaming.
#[allow(clippy::too_many_arguments)]
async fn stream_round(
    app: &AppHandle,
    db: &SqlitePool,
    provider: Box<dyn LlmProvider>,
    req: ChatRequest,
    conversation_id: &str,
    assistant_message_id: &str,
    full_content_prefix: &str,
    cancel: Arc<Notify>,
) -> Result<RoundOutcome> {
    let (tx, mut rx) = mpsc::channel::<Result<ChatChunk>>(64);
    let stream_task = {
        let err_tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = provider.chat_stream(req, tx).await {
                tracing::warn!("provider chat_stream returned err: {e:#}");
                let _ = err_tx.send(Err(e)).await;
            }
        })
    };

    let mut acc = StreamAccumulator::new();
    let mut last_flush = Instant::now();
    let mut cancelled = false;
    let mut error: Option<anyhow::Error> = None;

    loop {
        tokio::select! {
            biased;
            _ = cancel.notified() => {
                cancelled = true;
                stream_task.abort();
                break;
            }
            maybe = rx.recv() => {
                match maybe {
                    None => break,
                    Some(Err(e)) => {
                        stream_task.abort();
                        error = Some(e);
                        break;
                    }
                    Some(Ok(chunk)) => {
                        let outcome = acc.step(chunk, |delta, thinking| {
                            let _ = app.emit(
                                events::CHAT_DELTA,
                                DeltaPayload {
                                    conversation_id,
                                    message_id: assistant_message_id,
                                    delta,
                                    thinking,
                                },
                            );
                        });
                        if matches!(outcome, StepOutcome::EndOfStream) {
                            break;
                        }
                        if last_flush.elapsed() >= Duration::from_millis(750) {
                            last_flush = Instant::now();
                            let combined_content =
                                format!("{full_content_prefix}{}", acc.content);
                            // Thinking is moved inline into
                            // `full_content` at the round boundary, so
                            // mid-stream flushes only need to persist
                            // the live thinking from the round that's
                            // currently in flight. Passing `Some` here
                            // (even just the in-progress reasoning)
                            // keeps the bubble's top "Thinking" panel
                            // populated while the round is streaming;
                            // it's cleared by the runner's
                            // `chat://rewrite` once the round ends.
                            let live_thinking = if acc.thinking.is_empty() {
                                None
                            } else {
                                Some(acc.thinking.as_str())
                            };
                            let _ = chat::update_message(
                                db,
                                assistant_message_id,
                                &combined_content,
                                live_thinking,
                            )
                            .await;
                        }
                    }
                }
            }
        }
    }

    // Wait for the stream task to wind down (aborted or naturally finished).
    // We don't surface a JoinError; the channel already told us what we need.
    let _ = stream_task.await;

    if let Some(e) = error {
        return Err(e);
    }
    Ok(RoundOutcome {
        content: acc.content,
        thinking: acc.thinking,
        cancelled,
        tool_calls: acc.tool_calls,
    })
}

/// Fire a synthetic `chat://delta` for runner-generated text (tool call
/// banners, tool result blocks, budget-exhausted notes). The frontend
/// renders these into the same assistant bubble alongside model output.
fn emit_delta(app: &AppHandle, conversation_id: &str, message_id: &str, delta: &str) {
    if delta.is_empty() {
        return;
    }
    let _ = app.emit(
        events::CHAT_DELTA,
        DeltaPayload {
            conversation_id,
            message_id,
            delta,
            thinking: false,
        },
    );
}

/// Fire a `chat://rewrite` so the frontend overwrites the live
/// streaming buffer for `message_id` with `content`. Used after the
/// runner consumes a legacy ```tool_use``` fence — the raw fence has
/// already streamed into the UI bubble via [`emit_delta`] and the
/// canonical text is shorter, so an append-only delta can't fix it.
fn emit_rewrite(app: &AppHandle, conversation_id: &str, message_id: &str, content: &str) {
    let _ = app.emit(
        events::CHAT_REWRITE,
        RewritePayload {
            conversation_id,
            message_id,
            content,
        },
    );
}

/// One model-emitted tool invocation, parsed out of the streaming response.
#[derive(Debug, Clone)]
struct ParsedToolCall {
    /// Byte offset into the original content where the fence starts. We
    /// use this to trim the call (and anything after it) out of the
    /// visible portion of the model's reply.
    fence_start: usize,
    server: String,
    tool: String,
    arguments: Value,
}

/// Scan `text` for the first `tool_use`-style fenced block. We accept a
/// few common labellings (`tool_use`, `tool_call`, plain `tool`) because
/// instruction-following on small local models is inconsistent. The JSON
/// payload must carry at least a `tool` (or `name`) field; `server` and
/// `arguments` are optional.
fn parse_tool_call(text: &str) -> Option<ParsedToolCall> {
    const LABELS: &[&str] = &["```tool_use", "```tool_call", "```tool"];

    // Find the earliest matching opener so we honour the first call the
    // model emitted even if it later tried a different label.
    let mut best: Option<(usize, &str)> = None;
    for label in LABELS {
        if let Some(idx) = text.find(label) {
            match best {
                Some((b, _)) if b <= idx => {}
                _ => best = Some((idx, *label)),
            }
        }
    }
    let (fence_start, label) = best?;
    let after_open = &text[fence_start + label.len()..];
    let after_open = after_open.trim_start_matches('\r').trim_start_matches('\n');
    let close = after_open.find("```")?;
    let payload = after_open[..close].trim();
    if payload.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    let tool = value
        .get("tool")
        .or_else(|| value.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if tool.is_empty() {
        return None;
    }
    let server = value
        .get("server")
        .or_else(|| value.get("server_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let arguments = value
        .get("arguments")
        .or_else(|| value.get("args"))
        .or_else(|| value.get("input"))
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    Some(ParsedToolCall {
        fence_start,
        server,
        tool,
        arguments,
    })
}

/// Resolve a parsed call against the live catalog. When the model omitted
/// the `server` field we fall back to the first tool with a matching name
/// across all enabled servers.
///
/// Only used by the legacy fenced-JSON test fixture now — the production
/// agent loop resolves through [`mcp_catalog::resolve_function_name`]
/// instead, which understands the wire-format `<server>__<tool>` shape.
#[cfg(test)]
fn resolve_tool<'a>(tools: &'a [EnabledTool], call: &ParsedToolCall) -> Option<&'a EnabledTool> {
    if !call.server.is_empty() {
        if let Some(t) = tools
            .iter()
            .find(|t| t.server_id == call.server && t.schema.name == call.tool)
        {
            return Some(t);
        }
    }
    tools.iter().find(|t| t.schema.name == call.tool)
}

/// Extract the `name` argument from a `tools.list` call so the runner
/// knows which tool the model just asked for. Returns `None` when the
/// model called `tools.list({})` (catalogue-only summary, no specific
/// tool requested yet) or when the arguments are malformed.
///
/// The value is returned verbatim and may be either a bare `tool_name`
/// or a `server_id/tool_name` shorthand; resolution against the live
/// catalogue happens in [`resolve_revealed_target`].
fn parse_tools_list_name_arg(arguments: &str) -> Option<String> {
    let v: Value = serde_json::from_str(arguments).ok()?;
    v.get("name")
        .and_then(|n| n.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Look up an enabled tool by the user-facing name the model passed to
/// `tools.list({name: ...})`. Accepts both bare `tool_name` and the
/// `server_id/tool_name` shorthand, matching the format documented in
/// the `tools.list` schema and used by [`mcp_catalog::render_tools_list_result`].
fn resolve_revealed_target<'a>(
    tools: &'a [EnabledTool],
    user_name: &str,
) -> Option<&'a EnabledTool> {
    let (server_hint, bare_name) = match user_name.split_once('/') {
        Some((srv, tool)) => (Some(srv), tool),
        None => (None, user_name),
    };
    tools.iter().find(|t| match server_hint {
        Some(s) => t.server_id == s && t.schema.name == bare_name,
        None => t.schema.name == bare_name,
    })
}

/// Convert a legacy ` ```tool_use ``` ` block into the OpenAI-shaped
/// [`ToolCall`] the new dispatcher expects. Synthesises a random call id
/// so the structured `role: "tool"` follow-up still has something to bind
/// to even though no upstream protocol assigned one. The function name
/// is derived through the same `<server>__<tool>` encoding the structured
/// path uses so resolution is identical for both paths.
fn legacy_to_tool_call(legacy: &ParsedToolCall) -> ToolCall {
    let func_name = if legacy.server.is_empty() {
        mcp_catalog::function_name_for(BUILTIN_SERVER_ID, &legacy.tool)
        // Fallback resolver also matches by bare tool name, so this
        // is safe even when the actual server is something else.
    } else {
        mcp_catalog::function_name_for(&legacy.server, &legacy.tool)
    };
    ToolCall {
        id: format!("legacy_{}", uuid::Uuid::new_v4()),
        kind: "function".into(),
        function: crate::llm::FunctionCall {
            name: func_name,
            arguments: serde_json::to_string(&legacy.arguments).unwrap_or_else(|_| "{}".into()),
        },
    }
}

/// Payload for the `chat://tool-confirm` event. The frontend stores
/// these keyed by `call_id` and replies with `chat_tool_confirm`.
#[derive(Debug, Clone, Serialize)]
struct ToolConfirmPayload<'a> {
    conversation_id: &'a str,
    message_id: &'a str,
    call_id: &'a str,
    server_id: &'a str,
    server_name: &'a str,
    tool: &'a str,
    description: &'a str,
    arguments: &'a Value,
    destructive: bool,
}

/// Run a parsed tool call end-to-end. Owns all the orchestration the
/// agent loop wants to keep out of `run_inner`:
///
/// 1. Resolve the catalog entry (or synthesize a rejection string).
/// 2. For destructive tools with `destructive_tool_confirm` on, emit the
///    `chat://tool-confirm` event and block on the user's reply — racing
///    against chat cancellation so a cancel during the prompt still wins.
/// 3. Execute the call via the MCP client, again racing against
///    cancellation so a `Stop` mid-HTTP-call doesn't have to wait for the
///    server to finish.
///
/// Returns `(result_text, cancelled)`. `result_text` is always safe to
/// stream into the assistant bubble *and* re-feed to the model as a
/// `tool` message — even rejection paths are written for the model's
/// consumption.
#[allow(clippy::too_many_arguments)]
async fn run_tool_call(
    app: &AppHandle,
    http: &reqwest::Client,
    settings: &Settings,
    tools: &[EnabledTool],
    call: &ToolCall,
    conversation_id: &str,
    assistant_message_id: &str,
    cancel: Arc<Notify>,
) -> (String, bool) {
    // OpenAI function names go through our `<server>__<tool>`
    // sanitised encoding; resolve back to the live catalogue entry.
    let func_name = call.function.name.as_str();
    let Some(resolved) = mcp_catalog::resolve_function_name(tools, func_name) else {
        return (format!("[no enabled tool matches `{func_name}`]"), false);
    };

    // OpenAI spec: `arguments` is a JSON-encoded **string**. Parse to a
    // JSON value before dispatch. An empty string is equivalent to `{}`;
    // anything that fails to parse falls back to an empty object plus a
    // warning so the tool still has a chance to run with defaults.
    let raw_args = call.function.arguments.as_str();
    let arguments: Value = if raw_args.is_empty() {
        Value::Object(Default::default())
    } else {
        match serde_json::from_str::<Value>(raw_args) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "tool `{}` arguments did not parse as JSON ({e}); raw: {raw_args}",
                    resolved.schema.name
                );
                return (
                    format!(
                        "[invalid arguments for `{}`: not valid JSON — {e}]",
                        resolved.schema.name
                    ),
                    false,
                );
            }
        }
    };

    // ── tools.list intercept ────────────────────────────────────
    // The discovery tool is a placeholder in the built-in registry
    // (its `call` body returns a generic explanation) because the
    // registry has no access to the per-turn `EnabledTool` view. The
    // chat runner does, so we render the live catalogue directly here
    // before falling into the regular dispatch path. Keeps the
    // listing accurate with respect to per-chat disables, web
    // slash-gates, and the lazy-mode collapse itself.
    if resolved.server_id == BUILTIN_SERVER_ID
        && resolved.schema.name == mcp::tools::discovery::TOOLS_LIST_NAME
    {
        // `name` and `server_id` are both optional filter strings; an
        // unparseable shape just degrades to the no-filter listing
        // rather than erroring — the model can always re-call with
        // tighter arguments.
        let name_filter = arguments
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let server_filter = arguments
            .get("server_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let body = mcp_catalog::render_tools_list_result(tools, name_filter, server_filter);
        return (body, false);
    }

    // ── Confirm gate ─────────────────────────────────────────
    if resolved.schema.destructive && settings.destructive_tool_confirm {
        let confirms = app.zero().tool_confirms.clone();
        let confirm_id = uuid::Uuid::new_v4().to_string();
        let rx = confirms.register(confirm_id.clone()).await;

        let _ = app.emit(
            events::CHAT_TOOL_CONFIRM,
            ToolConfirmPayload {
                conversation_id,
                message_id: assistant_message_id,
                call_id: &confirm_id,
                server_id: &resolved.server_id,
                server_name: &resolved.server_name,
                tool: &resolved.schema.name,
                description: &resolved.schema.description,
                arguments: &arguments,
                destructive: true,
            },
        );

        let allow = tokio::select! {
            biased;
            _ = cancel.notified() => {
                // Chat was cancelled while we waited. Clean up the
                // dangling sender so the registry doesn't slowly grow,
                // and surface as cancellation to the caller.
                confirms.forget(&confirm_id).await;
                return (
                    format!(
                        "[cancelled while waiting for user to confirm `{}`]",
                        resolved.schema.name
                    ),
                    true,
                );
            }
            decision = rx => decision.unwrap_or(false),
        };
        if !allow {
            return (
                format!(
                    "[refused by user: `{}` on `{}` is marked destructive]",
                    resolved.schema.name, resolved.server_id
                ),
                false,
            );
        }
    }

    // ── Execute ─────────────────────────────────────────────────
    // Built-in tools live in-process and are dispatched through
    // `mcp::builtin_registry`; everything else is a JSON-RPC call to a
    // configured MCP server. The cancel race wraps both so a `Stop`
    // mid-execution wins immediately.
    if resolved.server_id == BUILTIN_SERVER_ID {
        let registry = mcp::builtin_registry(app);
        let Some(tool) = registry
            .into_iter()
            .find(|t| t.schema().name == resolved.schema.name)
        else {
            return (
                format!(
                    "[built-in tool `{}` is no longer registered]",
                    resolved.schema.name
                ),
                false,
            );
        };
        let call_fut = async move { tool.call(arguments).await };
        tokio::pin!(call_fut);
        return tokio::select! {
            biased;
            _ = cancel.notified() => (
                format!(
                    "[cancelled while executing built-in `{}`]",
                    resolved.schema.name
                ),
                true,
            ),
            result = &mut call_fut => match result {
                Ok(out) if out.is_error => (format!("[tool reported error]\n{}", out.content), false),
                Ok(out) => (out.content, false),
                Err(e) => (format!("[built-in tool failed: {e:#}]"), false),
            },
        };
    }

    let Some(cfg) = settings
        .mcp_servers
        .iter()
        .find(|s| s.id == resolved.server_id && s.enabled)
    else {
        return (
            format!(
                "[MCP server `{}` is no longer enabled — cannot dispatch `{}`]",
                resolved.server_id, resolved.schema.name
            ),
            false,
        );
    };

    let call_fut = mcp_client::call_tool(http, cfg, &resolved.schema.name, arguments);
    tokio::pin!(call_fut);
    tokio::select! {
        biased;
        _ = cancel.notified() => (
            format!(
                "[cancelled while executing `{}` on `{}`]",
                resolved.schema.name, resolved.server_id
            ),
            true,
        ),
        result = &mut call_fut => match result {
            Ok(out) if out.is_error => (format!("[tool reported error]\n{}", out.content), false),
            Ok(out) => (out.content, false),
            Err(e) => (format!("[tool call failed: {e:#}]"), false),
        },
    }
}

// Unused, retained as the simpler test surface for the resolver. The
// real agent loop now goes through `run_tool_call` to integrate cancel
// + confirm flows.
#[cfg(test)]
#[allow(dead_code)]
async fn dispatch_tool(
    http: &reqwest::Client,
    settings: &Settings,
    tools: &[EnabledTool],
    call: &ParsedToolCall,
) -> String {
    let Some(resolved) = resolve_tool(tools, call) else {
        return format!("[no enabled MCP tool matches `{}`]", call.tool);
    };
    let Some(cfg) = settings
        .mcp_servers
        .iter()
        .find(|s| s.id == resolved.server_id && s.enabled)
    else {
        return format!("[MCP server `{}` is no longer enabled]", resolved.server_id);
    };
    match mcp_client::call_tool(http, cfg, &resolved.schema.name, call.arguments.clone()).await {
        Ok(out) if out.is_error => format!("[tool reported error]\n{}", out.content),
        Ok(out) => out.content,
        Err(e) => format!("[tool call failed: {e:#}]"),
    }
}

/// Render the user-visible "calling tool" banner streamed into the chat
/// bubble. Format is plain markdown so the existing `whitespace-pre-wrap`
/// renderer in `Chat.tsx` shows it sensibly without any UI changes.
///
/// We resolve the structured [`ToolCall`] back to the catalogue entry
/// when we can so the banner shows the *original* `server/tool` pair
/// (e.g. `builtin/fs.list`) instead of the sanitised wire-format
/// function name (`builtin__fs_list`).
fn render_tool_call_banner(tools: &[EnabledTool], call: &ToolCall) -> String {
    let func_name = call.function.name.as_str();
    let (server, tool) = match mcp_catalog::resolve_function_name(tools, func_name) {
        Some(t) => (t.server_id.as_str(), t.schema.name.as_str()),
        None => match mcp_catalog::split_function_name(func_name) {
            Some((s, t)) => (s, t),
            None => ("?", func_name),
        },
    };

    // Re-pretty-print the arguments so the banner reads cleanly even
    // when the model emitted a single-line JSON string.
    let args_pretty = match serde_json::from_str::<Value>(&call.function.arguments) {
        Ok(v) => {
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| call.function.arguments.clone())
        }
        Err(_) => call.function.arguments.clone(),
    };
    format!("\n\n[tool call: {server}/{tool}]\n```json\n{args_pretty}\n```\n")
}

/// Legacy banner builder retained so the unit test fixture continues to
/// exercise the [`ParsedToolCall`] formatting code path even though the
/// production agent loop now goes through [`render_tool_call_banner`].
#[cfg(test)]
fn render_call_block(call: &ParsedToolCall) -> String {
    let args = serde_json::to_string_pretty(&call.arguments).unwrap_or_else(|_| "{}".into());
    let server = if call.server.is_empty() {
        "?"
    } else {
        call.server.as_str()
    };
    format!(
        "\n\n[tool call: {}/{}]\n```json\n{}\n```\n",
        server, call.tool, args
    )
}

/// Render the tool's response inline. We cap the visible text at 4 KB so
/// a chatty tool can't blow the chat bubble, but the *full* text is still
/// what the model sees on the next round via the `tool` message we push
/// into `messages`.
fn render_result_block(result: &str) -> String {
    const MAX_VISIBLE: usize = 4096;
    let trimmed = if result.len() > MAX_VISIBLE {
        let mut cut = MAX_VISIBLE;
        while !result.is_char_boundary(cut) && cut > 0 {
            cut -= 1;
        }
        format!("{}\u{2026}\n[truncated]", &result[..cut])
    } else {
        result.to_string()
    };
    format!("\n[tool result]\n```\n{trimmed}\n```\n\n")
}

/// Format a per-round reasoning trace as an inline markdown-ish block
/// the frontend's assistant-content parser splits out and renders as a
/// collapsible "Thinking" card. The block format mirrors the
/// `[tool call: …]` / `[tool result]` markers in
/// [`render_tool_call_banner`] and [`render_result_block`] so all three
/// kinds of "runner-emitted aside" use the same protocol.
///
/// Returns an empty string when `thinking` is empty so the caller can
/// unconditionally `push_str` and skip the rewrite when nothing changed.
fn render_thinking_block(thinking: &str) -> String {
    if thinking.is_empty() {
        return String::new();
    }
    format!("\n[thinking]\n{}\n[/thinking]\n\n", thinking.trim_end())
}

/// Per-chunk outcome from [`StreamAccumulator::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepOutcome {
    /// Keep waiting for more chunks.
    Continue,
    /// Provider signalled end-of-stream (either a `done = true` chunk or a
    /// `data: [DONE]` sentinel translated into one).
    EndOfStream,
}

/// Pure state machine for the chat drain loop. Owns the running content +
/// thinking buffers and decides what each incoming [`ChatChunk`] means.
/// Kept free of [`AppHandle`] / [`SqlitePool`] so it can be unit-tested
/// without standing up a Tauri runtime or a SQLite pool — the live runner
/// supplies side-effects via the `on_delta` callback passed to [`step`].
#[derive(Debug, Default)]
struct StreamAccumulator {
    content: String,
    thinking: String,
    /// Tool calls collected from terminating chunks. The transport layer
    /// in [`openai_compat`] assembles streamed `tool_calls[].index`
    /// fragments into complete entries, so by the time we see them here
    /// they're ready to dispatch.
    tool_calls: Vec<ToolCall>,
    /// Per-stream filter that scrubs chat-template special tokens out of
    /// the visible content stream. See [`SpecialTokenStripper`].
    stripper: SpecialTokenStripper,
}

impl StreamAccumulator {
    fn new() -> Self {
        Self::default()
    }

    /// Borrow the thinking buffer as `Option<&str>` (returning `None` when
    /// empty) for the DB write helper, which encodes "no thinking" as NULL.
    #[allow(dead_code)]
    fn thinking_as_opt(&self) -> Option<&str> {
        if self.thinking.is_empty() {
            None
        } else {
            Some(&self.thinking)
        }
    }

    /// Apply a single chunk. `on_delta` is invoked at most once per
    /// non-empty visible delta (so the runner can emit a frontend event).
    /// Empty deltas, `done` markers, and chunks whose entire payload is
    /// being held back by the [`SpecialTokenStripper`] (waiting for the
    /// rest of a possibly-leaked token to arrive) do not trigger it.
    fn step(&mut self, chunk: ChatChunk, mut on_delta: impl FnMut(&str, bool)) -> StepOutcome {
        if chunk.done {
            // The stream is over — any text still held back by the
            // stripper cannot be the start of a special token after all,
            // so flush it through as visible content.
            let tail = self.stripper.flush();
            if !tail.is_empty() {
                on_delta(&tail, false);
                self.content.push_str(&tail);
            }
            // Terminating chunks may carry the round's accumulated
            // tool_calls (when finish_reason == "tool_calls"). Capture
            // them before returning so the runner sees a complete batch.
            if !chunk.tool_calls.is_empty() {
                self.tool_calls.extend(chunk.tool_calls);
            }
            return StepOutcome::EndOfStream;
        }
        if chunk.delta.is_empty() {
            return StepOutcome::Continue;
        }
        if chunk.thinking {
            // Thinking text bypasses the stripper. Reasoning parsers
            // already extract a clean `reasoning_content` field, and
            // legitimate template markers (e.g. Gemma 4's `<|think|>`)
            // can appear in the raw thinking buffer for diagnostic
            // purposes — we don't want to silently swallow them.
            on_delta(&chunk.delta, true);
            self.thinking.push_str(&chunk.delta);
        } else {
            let visible = self.stripper.push(&chunk.delta);
            if !visible.is_empty() {
                on_delta(&visible, false);
                self.content.push_str(&visible);
            }
        }
        StepOutcome::Continue
    }

    /// Convert the live buffers into the final `(content, thinking)` the
    /// runner persists. If we were cancelled with nothing collected, the
    /// content gets a `[cancelled]` marker so the UI shows the turn ran
    /// instead of an empty assistant bubble.
    #[allow(dead_code)]
    fn finalize(mut self, cancelled: bool) -> (String, Option<String>) {
        let tail = self.stripper.flush();
        if !tail.is_empty() {
            self.content.push_str(&tail);
        }
        if cancelled && self.content.is_empty() && self.thinking.is_empty() {
            self.content.push_str("[cancelled]");
        }
        let thinking = if self.thinking.is_empty() {
            None
        } else {
            Some(self.thinking)
        };
        (self.content, thinking)
    }
}

/// Chat-template special tokens that occasionally leak through OVMS's
/// `delta.content` stream and end up in the visible assistant bubble.
///
/// The root cause is upstream: when the tokenizer doesn't have a given
/// token registered as `special: true`, or when a tool/reasoning parser
/// fails to consume one of its own sentinels, the literal text shows up
/// in the OpenAI-compat `delta.content` field. Gemma 4 is the worst
/// offender today — the `gemma4` tool_parser occasionally emits a
/// trailing `<|tool_response>` before a tool call and a closing `<eos>`
/// at end-of-turn, both of which are pure protocol leakage with no
/// semantic value to the user.
///
/// We strip these client-side rather than ask the user to wait for an
/// OVMS fix. Tokens are scrubbed from the *content* stream only; the
/// thinking stream is left alone (a reasoning parser may surface
/// markers there on purpose).
const LEAKED_SPECIAL_TOKENS: &[&str] = &[
    // Gemma 3 / Gemma 4 tool protocol
    "<|tool_response|>",
    "</|tool_response|>",
    "<|tool_response>",
    "</|tool_response>",
    "<|tool_call|>",
    "</|tool_call|>",
    "<|tool_call>",
    "</|tool_call>",
    // Gemma turn / sequence markers
    "<start_of_turn>",
    "<end_of_turn>",
    "<eos>",
    "<bos>",
    // ChatML (Qwen / Hermes / phi-4 style)
    "<|im_start|>",
    "<|im_end|>",
    // GPT-OSS / generic
    "<|endoftext|>",
];

/// Streaming filter that removes [`LEAKED_SPECIAL_TOKENS`] from a
/// chunked text stream.
///
/// Because special tokens can be split across delta boundaries
/// (`<|too` then `l_response>...`), the stripper keeps a small
/// lookback buffer: whenever a chunk ends with something that *could*
/// be the prefix of a known token, that trailing slice is held back
/// until the next chunk disambiguates it (or [`flush`] is called at
/// end-of-stream and the held-back text turns out to be innocuous).
#[derive(Debug, Default)]
struct SpecialTokenStripper {
    pending: String,
}

impl SpecialTokenStripper {
    /// Append `delta` to the working buffer, remove every complete
    /// known token from it, and return the prefix that is now safe to
    /// emit. Anything that might still grow into a special token stays
    /// in `pending` for the next call.
    fn push(&mut self, delta: &str) -> String {
        self.pending.push_str(delta);
        self.pending = strip_complete_tokens(&self.pending);
        let split = safe_split_point(&self.pending);
        let emit: String = self.pending[..split].to_string();
        self.pending.drain(..split);
        emit
    }

    /// Drain the lookback buffer at end-of-stream. One final scrub
    /// catches tokens that only completed on the very last chunk;
    /// whatever remains can't possibly grow into a known token so it
    /// is returned verbatim.
    fn flush(&mut self) -> String {
        let stripped = strip_complete_tokens(&self.pending);
        self.pending.clear();
        stripped
    }
}

/// Remove every occurrence of every token in [`LEAKED_SPECIAL_TOKENS`]
/// from `s`. Each pass picks the earliest match across all tokens so
/// overlapping aliases (e.g. `<|tool_call>` vs `<|tool_call|>`) are
/// resolved by position rather than list order.
fn strip_complete_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let mut found: Option<(usize, &str)> = None;
        for tok in LEAKED_SPECIAL_TOKENS {
            if let Some(idx) = rest.find(tok) {
                match found {
                    Some((best, _)) if best <= idx => {}
                    _ => found = Some((idx, *tok)),
                }
            }
        }
        match found {
            Some((idx, tok)) => {
                out.push_str(&rest[..idx]);
                rest = &rest[idx + tok.len()..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// Return the byte offset before which `s` is unambiguously *not* part
/// of a leaked special token. The suffix `s[offset..]` is everything
/// the stripper needs to hold back until the next chunk arrives.
///
/// Every token in [`LEAKED_SPECIAL_TOKENS`] starts with `<`, so any
/// suffix that doesn't begin with `<` is automatically safe. This
/// keeps the held-back window to whatever fragment actually looks like
/// the start of a tag.
fn safe_split_point(s: &str) -> usize {
    let max_tok = LEAKED_SPECIAL_TOKENS
        .iter()
        .map(|t| t.len())
        .max()
        .unwrap_or(0);
    let lookback = max_tok.saturating_sub(1).min(s.len());
    for k in (1..=lookback).rev() {
        let start = s.len() - k;
        if !s.is_char_boundary(start) {
            continue;
        }
        let suffix = &s[start..];
        if !suffix.starts_with('<') {
            continue;
        }
        if LEAKED_SPECIAL_TOKENS.iter().any(|t| t.starts_with(suffix)) {
            return start;
        }
    }
    s.len()
}

/// Inspect an `anyhow::Error` for known typed variants and turn it into a
/// [`ChatErrorReport`] with a user-actionable hint. Falls back to
/// [`classify_other`] for everything else.
fn classify(err: &anyhow::Error, provider_kind: &str, base_url: &str) -> ChatErrorReport {
    if let Some(up) = err.downcast_ref::<UpstreamError>() {
        return match up {
            UpstreamError::Unreachable { attempts, last } => ChatErrorReport {
                kind: ChatErrorKind::UpstreamUnreachable,
                message: format!("could not reach {base_url} after {attempts} attempts ({last})"),
                hint: Some(if provider_kind == "ovms" {
                    "OVMS is unreachable — check it's running on the Server page and that the REST port in Settings matches".into()
                } else {
                    format!("check that the {provider_kind} server at {base_url} is reachable")
                }),
                provider_kind: Some(provider_kind.to_string()),
                base_url: Some(base_url.to_string()),
            },
            UpstreamError::Http { status, body } => {
                let code = status.as_u16();
                let hint = match code {
                    401 | 403 => Some(
                        "authentication failed — set the provider API key in Settings".into(),
                    ),
                    404 => Some(
                        "model not found at this endpoint — check the model id and base URL"
                            .into(),
                    ),
                    413 => Some("request too large — try a shorter prompt or smaller context".into()),
                    422 => Some(
                        "upstream rejected the request shape — provider may not be OpenAI-compatible"
                            .into(),
                    ),
                    _ => None,
                };
                ChatErrorReport {
                    kind: ChatErrorKind::UpstreamHttp,
                    message: format!("upstream {status}: {}", truncate(body, 512)),
                    hint,
                    provider_kind: Some(provider_kind.to_string()),
                    base_url: Some(base_url.to_string()),
                }
            }
        };
    }
    classify_other(err, provider_kind, base_url)
}

fn classify_other<E: std::fmt::Display>(
    err: &E,
    provider_kind: &str,
    base_url: &str,
) -> ChatErrorReport {
    ChatErrorReport {
        kind: ChatErrorKind::Other,
        message: format!("{err}"),
        hint: None,
        provider_kind: Some(provider_kind.to_string()),
        base_url: Some(base_url.to_string()),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Per-model defaults that affect on-the-wire requests and prompt
/// mechanics. Looked up once per chat turn via [`model_profile`] and
/// consumed by the request builder + the system-prompt assembler.
///
/// Centralising these decisions here keeps the agent loop free of
/// `if is_gemma { … } else if is_llama { … }` branches and makes the
/// per-family tuning easy to spot in code review.
#[derive(Debug, Clone, Copy)]
struct ModelProfile {
    /// Sampling temperature. Forwarded verbatim to the upstream.
    temperature: f32,
    /// Optional nucleus-sampling cutoff. `None` lets the upstream use
    /// its own default.
    top_p: Option<f32>,
    /// Optional top-k cutoff. `None` lets the upstream use its own
    /// default; some families (Gemma 4) require an explicit value to
    /// reach their documented quality bar.
    top_k: Option<u32>,
    /// When `true`, multimodal user turns are laid out with image
    /// parts before the user text. Required by Gemma 4 for "optimal
    /// performance with multimodal inputs" per its model card; other
    /// families are insensitive to ordering.
    images_before_text: bool,
    /// Control token prepended verbatim to the system prompt when the
    /// user has `thinking_enabled` on. Used by families that gate their
    /// reasoning trace on an explicit marker (Gemma 4's `<|think|>`).
    /// Families that always expose thinking (Qwen3, gpt-oss, …) leave
    /// this as `None`.
    thinking_control_token: Option<&'static str>,
}

const DEFAULT_MODEL_PROFILE: ModelProfile = ModelProfile {
    temperature: 0.7,
    top_p: None,
    top_k: None,
    images_before_text: false,
    thinking_control_token: None,
};

/// Sampling + prompt profile recommended by the Gemma 4 model card:
///   * `temperature = 1.0`, `top_p = 0.95`, `top_k = 64`
///   * image/audio parts placed before user text in multimodal turns
///   * `<|think|>` system-prompt prefix toggles the reasoning trace on
const GEMMA4_MODEL_PROFILE: ModelProfile = ModelProfile {
    temperature: 1.0,
    top_p: Some(0.95),
    top_k: Some(64),
    images_before_text: true,
    thinking_control_token: Some("<|think|>"),
};

/// Resolve a model id to its [`ModelProfile`].
///
/// Matching is case-insensitive and works on substrings so the various
/// id conventions Hugging Face / OpenVINO use (`google/gemma-4-E2B-it`,
/// `OpenVINO/gemma-4-E4B-it-int4-ov`, plain `gemma4`, …) all land on
/// the same profile. Falls back to [`DEFAULT_MODEL_PROFILE`] when no
/// family-specific tuning is known — those models simply keep the
/// previous behaviour (temp 0.7, no top-p/top-k override, text-first
/// modality, no thinking control token).
fn model_profile(model: &str) -> ModelProfile {
    if is_gemma4_family(model) {
        GEMMA4_MODEL_PROFILE
    } else {
        DEFAULT_MODEL_PROFILE
    }
}

/// Narrow Gemma 4 detector. We intentionally do NOT match plain
/// `gemma` / `gemma-2` / `gemma-3` because the best-practice profile
/// here (sampling + `<|think|>` control token + image-first modality)
/// is specific to Gemma 4 — enabling it on older Gemmas would either
/// be ignored at best or leak the literal `<|think|>` text into the
/// prompt at worst.
fn is_gemma4_family(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("gemma-4") || lower.contains("gemma4") || lower.contains("gemma_4")
}

/// Concrete sampling values handed to the per-turn [`ChatRequest`].
/// Distinct from [`ModelProfile`] so the resolver can express
/// "temperature is always set, top_p/top_k may legitimately stay
/// unset" without leaking the multi-layer override structure into the
/// request builder.
#[derive(Debug, Clone, Copy)]
struct ResolvedSampling {
    temperature: f32,
    top_p: Option<f32>,
    top_k: Option<u32>,
}

/// Merge the conversation → provider → profile sampling layers into
/// the concrete values sent on the wire. The first non-`None` override
/// for each field wins; `temperature` always lands on a concrete value
/// because the profile's default acts as the floor.
///
/// Out-of-range values are accepted as-is and forwarded to the
/// upstream — OVMS / vLLM / ollama all have their own clamps, and
/// we'd rather surface a 4xx from the server than silently rewrite
/// what the user typed.
fn resolve_sampling(
    conv: &SamplingConfig,
    provider: &SamplingConfig,
    profile: ModelProfile,
) -> ResolvedSampling {
    ResolvedSampling {
        temperature: conv
            .temperature
            .or(provider.temperature)
            .unwrap_or(profile.temperature),
        top_p: conv.top_p.or(provider.top_p).or(profile.top_p),
        top_k: conv.top_k.or(provider.top_k).or(profile.top_k),
    }
}

/// Build the `messages` array for the provider request. Prepends a system
/// prompt augmented with any enabled skills, drops the trailing empty
/// assistant placeholder we just inserted, and — for the last user turn —
/// folds attachments into the OpenAI Vision multimodal content shape
/// (images become `image_url` data URLs; text documents are inlined as
/// `[file: name]` fenced blocks; binary attachments are mentioned by name).
///
/// History turns with attachments only get a one-line `[file: name]`
/// marker — we deliberately don't re-encode large images on every
/// follow-up. The model still sees that something was attached without
/// blowing the context window on duplicate base64 blobs.
async fn build_request_messages(
    history: &[Message],
    settings: &Settings,
    tools: &[EnabledTool],
    loop_mode: bool,
    // `true` when the composer's per-turn "Thinking" toggle is on for
    // this round — forwarded to `build_system_prompt` so the
    // per-family thinking control token (e.g. Gemma 4's `<|think|>`)
    // is emitted only when the user actually opted in.
    think_enabled: bool,
    native_tools_supported: bool,
    lazy_tool_discovery: bool,
    model: &str,
) -> Result<Vec<ChatMessage>> {
    let profile = model_profile(model);
    let mut out = Vec::with_capacity(history.len() + 1);
    out.push(ChatMessage::text(
        "system",
        build_system_prompt(
            settings,
            tools,
            loop_mode,
            think_enabled,
            native_tools_supported,
            lazy_tool_discovery,
            model,
        )
        .await,
    ));

    // Index of the last non-empty user message — its attachments get the
    // full multimodal treatment; earlier ones get a name-only marker.
    let last_user_idx = history
        .iter()
        .rposition(|m| m.role == "user" && !m.content.is_empty());

    for (idx, m) in history.iter().enumerate() {
        if m.role == "assistant" && m.content.is_empty() {
            continue;
        }
        // Per the Gemma 4 model card and most other chat-tuned families,
        // previous-turn assistant thinking content must NOT be replayed
        // on the next turn (it confuses both the chat template and the
        // model's own attention over its prior reasoning). We've always
        // satisfied this constraint by storing `thinking` in a separate
        // DB column and only sending `m.content` here — keep that
        // invariant in mind when touching history serialisation.
        let role = match m.role.as_str() {
            "user" | "assistant" | "system" | "tool" => m.role.clone(),
            _ => "user".into(),
        };

        let is_last_user = Some(idx) == last_user_idx;
        let atts = m.attachments.as_deref().unwrap_or(&[]);

        if atts.is_empty() {
            out.push(ChatMessage::text(role, m.content.clone()));
            continue;
        }

        if !is_last_user {
            // Earlier turn — keep the text, mention attachments by name.
            let mut text = m.content.clone();
            for a in atts {
                text.push_str(&format!("\n\n[file attached previously: {}]", a.name));
            }
            out.push(ChatMessage::text(role, text));
            continue;
        }

        // Last user turn — build the multimodal content array per the
        // OVMS chat/completions vision spec.
        let mut leading_text = m.content.clone();

        for a in atts {
            match a.kind.as_str() {
                "image" => { /* defer — images are emitted in their own loop below */ }
                _ => {
                    // Try to inline text docs into the user prompt; binary
                    // docs get a placeholder so the model knows they exist.
                    match attachments::read_as_text(a).await {
                        Ok(Some(text)) => {
                            leading_text
                                .push_str(&format!("\n\n[file: {}]\n```\n{}\n```", a.name, text));
                        }
                        Ok(None) => {
                            leading_text.push_str(&format!(
                                "\n\n[binary file attached: {} ({}, {} bytes)]",
                                a.name, a.mime, a.bytes
                            ));
                        }
                        Err(e) => {
                            tracing::warn!("could not read attachment {}: {e:#}", a.path);
                            leading_text
                                .push_str(&format!("\n\n[attachment unavailable: {}]", a.name));
                        }
                    }
                }
            }
        }

        // Encode every image attachment in catalogue order. We pull this
        // out into its own pass so the modality order (image-first vs.
        // text-first) can be selected by [`ModelProfile`] without
        // duplicating the read loop.
        let mut image_parts: Vec<ContentPart> = Vec::new();
        for a in atts.iter().filter(|a| a.kind == "image") {
            match attachments::read_as_data_url(a).await {
                Ok(url) => image_parts.push(ContentPart::ImageUrl {
                    image_url: ImageUrl { url },
                }),
                Err(e) => {
                    tracing::warn!("could not encode image {}: {e:#}", a.path);
                    image_parts.push(ContentPart::Text {
                        text: format!("[image unavailable: {}]", a.name),
                    });
                }
            }
        }

        let text_part =
            (!leading_text.is_empty()).then_some(ContentPart::Text { text: leading_text });

        // Gemma 4 explicitly calls for image (and audio) content to
        // come *before* the text in multimodal prompts. Other families
        // are insensitive to ordering; we keep the original text-first
        // layout there to minimise the diff to existing behaviour.
        let mut parts: Vec<ContentPart> = Vec::new();
        if profile.images_before_text {
            parts.extend(image_parts);
            if let Some(t) = text_part {
                parts.push(t);
            }
        } else {
            if let Some(t) = text_part {
                parts.push(t);
            }
            parts.extend(image_parts);
        }

        if parts.is_empty() {
            out.push(ChatMessage::text(role, m.content.clone()));
        } else {
            out.push(ChatMessage::parts(role, parts));
        }
    }
    Ok(out)
}

/// Assemble the system prompt: the base persona, plus a compact
/// catalog of every enabled skill (id + short description only). Skills
/// are intentionally NOT inlined; the model loads a skill's full body
/// on demand via the built-in `skill` tool. This mirrors the pattern
/// used by Claude Code's `SkillTool` and keeps turn-1 token cost flat
/// even when the user has many skills enabled.
///
/// Skills that don't exist on disk are silently dropped (we log a
/// warning so the user can investigate if they're surprised).
async fn build_system_prompt(
    settings: &Settings,
    tools: &[EnabledTool],
    loop_mode: bool,
    // Per-turn thinking opt-in. Default for every chat is **off**
    // regardless of any global setting — the composer's `+` menu
    // toggle is the only place this gets flipped on, and it scopes to
    // a single turn (the runner persists the flag on the user row so
    // `chat_retry` re-applies it).
    think_enabled: bool,
    native_tools_supported: bool,
    lazy_tool_discovery: bool,
    model: &str,
) -> String {
    let profile = model_profile(model);
    let mut out = String::new();
    // Some families gate their reasoning trace on an explicit control
    // token at the very start of the system prompt (Gemma 4's
    // `<|think|>`). We emit it iff the user actually opted into
    // thinking for this turn *and* the loaded model knows what to do
    // with it; other families would see it as literal junk text in
    // turn-1. `settings.thinking_enabled` is intentionally not
    // consulted here — every chat now defaults to no thinking and the
    // composer's per-turn `Thinking` toggle is the sole opt-in.
    if think_enabled {
        if let Some(tok) = profile.thinking_control_token {
            out.push_str(tok);
            out.push('\n');
        }
    }
    out.push_str(SYSTEM_PROMPT);
    out.push_str("\n\n");
    out.push_str(if loop_mode {
        AGENT_LOOP_MODE_HINT
    } else if lazy_tool_discovery {
        LAZY_SINGLE_SHOT_MODE_HINT
    } else {
        SINGLE_SHOT_MODE_HINT
    });
    // When the runtime can't parse OpenAI-native `tool_calls` (no
    // `tool_parser` configured on the loaded OVMS model) we have to
    // teach the model a text-only protocol the runner can scrape on
    // its side. Inject the fenced-JSON instructions only in that
    // mode — well-tooled models would otherwise be tempted to ignore
    // their proper protocol and emit raw text instead.
    if !native_tools_supported && !tools.is_empty() {
        out.push_str("\n\n");
        out.push_str(LEGACY_TOOL_PROTOCOL_HINT);
    }
    out.push_str(&render_skills_catalog(settings).await);
    // Tool-catalogue section. In lazy mode we replace the full inline
    // catalogue with a one-paragraph hint pointing the model at the
    // `tools.list` built-in; the model can drill into it on demand and
    // the runner expands the OpenAI `tools` array on subsequent rounds.
    // Falls back to the verbose section when lazy mode is on but the
    // discovery tool isn't actually in the catalogue (user disabled
    // it) — see [`mcp_catalog::render_lazy_prompt_section`].
    let lazy_section = if lazy_tool_discovery {
        mcp_catalog::render_lazy_prompt_section(tools)
    } else {
        String::new()
    };
    if !lazy_section.is_empty() {
        out.push_str(&lazy_section);
    } else {
        out.push_str(&mcp_catalog::render_prompt_section(tools));
    }
    // Persistent memory snapshot. Frozen at session start (the runner
    // re-reads it on every turn, but the chat-template / KV cache only
    // see whatever we splice in here). When both stores are empty this
    // renders to "" so a fresh install isn't burdened with a useless
    // header. See `crate::memory` for the curation model.
    match crate::memory::load_state().await {
        Ok(state) => out.push_str(&crate::memory::render_prompt_block(&state)),
        Err(e) => tracing::warn!("memory load for system prompt failed: {e:#}"),
    }
    // Teach the model how to write to the memory tool, but only when
    // the tool is actually exposed (it always is via the built-in
    // registry, but skipping the hint when `tools` is empty keeps
    // model output focused for users who run with all tools disabled).
    // In lazy mode we *also* skip this — the memory tool is not in
    // the initial advertised set, and the model will learn its schema
    // through `tools.list` if/when it decides it needs to write.
    if !lazy_section.is_empty() {
        // Lazy mode: memory hint omitted on purpose.
    } else if tools.iter().any(|t| t.schema.name == "memory") {
        out.push_str("\n\n");
        out.push_str(MEMORY_HINT);
    }
    out
}

/// Render the `# Skills` section that tells the model which skills are
/// available and how to load one. We list only `id: <short description>`
/// per skill — the full body is fetched on demand via the `skill`
/// built-in tool when the model decides to use one. Returns the empty
/// string when no skills are enabled or every enabled id failed to
/// load, so the prompt has no dangling header.
async fn render_skills_catalog(settings: &Settings) -> String {
    if settings.skills_enabled.is_empty() {
        return String::new();
    }

    let mut entries: Vec<(String, Option<String>)> =
        Vec::with_capacity(settings.skills_enabled.len());
    for id in &settings.skills_enabled {
        match skills::load_meta(id).await {
            Ok(meta) => entries.push((meta.id, meta.description)),
            Err(e) => {
                tracing::warn!("skill `{id}` enabled but could not be loaded: {e:#}");
            }
        }
    }
    if entries.is_empty() {
        return String::new();
    }

    let mut out = String::from("\n\n# Skills\n");
    out.push_str(
        "The following skills are available. Each skill is a set of \
         instructions you can load on demand by calling the `skill` \
         built-in tool with `{\"name\": \"<id>\"}`. The tool will return \
         the skill's full body; read it carefully and follow the steps \
         using your other enabled tools. When a listed skill plainly \
         matches the user's request, load it BEFORE attempting to answer \
         — do not invent tool names or fabricate procedures from the \
         short description below.\n\n",
    );
    for (id, desc) in entries {
        match desc {
            Some(d) => {
                let d = truncate_skill_description(&d);
                out.push_str(&format!("- {id}: {d}\n"));
            }
            None => out.push_str(&format!("- {id}\n")),
        }
    }
    out
}

/// Hard cap on a single skill's catalog entry description. The full
/// body is fetched via the `skill` tool, so a verbose `whenToUse`-style
/// blurb here just wastes turn-1 tokens without improving match rate.
/// Mirrors Claude Code's `MAX_LISTING_DESC_CHARS = 250`.
const MAX_SKILL_DESC_CHARS: usize = 250;

fn truncate_skill_description(desc: &str) -> String {
    let trimmed = desc.trim();
    if trimmed.chars().count() <= MAX_SKILL_DESC_CHARS {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX_SKILL_DESC_CHARS - 1).collect();
    out.push('\u{2026}'); // …
    out
}

const SYSTEM_PROMPT: &str = "You are ZerØ, a local, privacy-respecting AI assistant. \
Be concise and helpful. You have access to a set of tools (listed below if any are \
enabled) for tasks like local file I/O, fetching content from an MCP server, or \
running user-approved commands. Prefer calling a tool over apologising when a \
request can be satisfied with one. If no tool fits the request, say so plainly \
rather than guessing.";

/// Appended whenever the built-in `memory` tool is exposed. Teaches the
/// model the curation pattern Hermes Agent established: be proactive
/// about saving durable facts, skip ephemera, consolidate when the
/// store is near its cap. We intentionally do NOT tell the model to
/// poll memory — the frozen snapshot at the top of the prompt already
/// gives it the current contents.
const MEMORY_HINT: &str = "# Persistent memory policy\n\
You have a `memory` tool that writes to two small, character-bounded \
stores: `memory` (your personal notes — environment, conventions, \
lessons) and `user` (the user's preferences, style, identity). The \
current contents are shown above under \"# Persistent memory\" — do \
NOT call the tool just to read; you already see it.\n\
\n\
Save proactively when you learn something durable: user preferences \
(\"call me X\", \"prefers TypeScript\"), environment facts (\"this box is \
WSL Ubuntu with Docker\"), project conventions, or corrections (\"don't \
use sudo for docker here\"). Skip trivia, raw data dumps, easily \
re-discovered facts, or anything ephemeral to this turn.\n\
\n\
Keep entries compact and information-dense — one durable fact per \
entry. If an `add` returns an over-capacity error, consolidate \
overlapping entries with `replace` or drop low-value ones with \
`remove`, then retry the `add` in the same turn. Memory written this \
turn won't appear in your frozen snapshot until the next turn, but the \
tool's success result shows the updated state.";

/// Appended to the system prompt when the user has *not* prefixed their
/// message with `/loop`. Tells the model that it gets one tool-call cycle
/// and then must give a final answer — the runner enforces this with a
/// hard 2-iteration cap so a misbehaving model can't grind on the same
/// tool forever, but the prompt nudge keeps well-behaved models on track.
const SINGLE_SHOT_MODE_HINT: &str = "# Tool-use policy\n\
This turn is in single-shot mode. You may call at most one tool. As soon as \
the tool result comes back, write your final reply to the user using that \
result — do NOT call the same tool again or chain into another tool call. \
If the user wants you to chain multiple tool calls autonomously, they will \
prefix their message with `/loop`.";

/// Single-shot policy as it applies when lazy tool discovery is on.
/// Mirrors [`SINGLE_SHOT_MODE_HINT`] but explicitly carves out the
/// `tools.list` discovery call as a free first step — otherwise a
/// strictly-obedient model would refuse to call `tools.list` *and* the
/// real tool in the same turn, leaving lazy mode useless.
const LAZY_SINGLE_SHOT_MODE_HINT: &str = "# Tool-use policy\n\
This turn is in single-shot mode with lazy tool discovery. You may call \
`tools.list` once to discover what's available, then call at most one of \
the discovered tools. As soon as that tool's result comes back, write \
your final reply to the user using it — do NOT call the same tool again \
or chain into a third tool call. If you don't need any tool, just answer \
the user directly without calling `tools.list`. If the user wants you to \
chain multiple tool calls autonomously, they will prefix their message \
with `/loop`.";

/// Appended when the user *did* prefix with `/loop`. Permits autonomous
/// chaining, bounded by `agent_max_iterations` in settings.
const AGENT_LOOP_MODE_HINT: &str = "# Tool-use policy\n\
This turn is in `/loop` mode — you may chain multiple tool calls to fulfil \
the user's goal. Stop as soon as the goal is reached and reply to the user; \
never repeat the same tool call with the same arguments after it already \
succeeded.";

/// Appended when the loaded model has no OVMS `tool_parser` configured
/// (Granite, generic Llama-2 chat, custom fine-tunes, etc. — note Gemma
/// is *not* in this list because OVMS v2026.2 ships a `gemma4` parser).
/// The runtime can't extract structured `tool_calls` for these families,
/// so instead of letting the model emit its own template tokens (which
/// leak as raw text) we teach it a strict text protocol that
/// [`parse_tool_call`] can scrape reliably.
const LEGACY_TOOL_PROTOCOL_HINT: &str = "# Tool-call protocol\n\
IMPORTANT: this runtime cannot parse your model's native tool-call \
tokens (no `tool_parser` is configured). To call a tool you MUST emit a \
single fenced block exactly like this and then stop — wait for the \
tool's result before continuing:\n\n\
```tool_use\n\
{\"server\": \"<server_id>\", \"tool\": \"<tool_name>\", \"arguments\": { ... }}\n\
```\n\n\
Do NOT use any other tool-call syntax (no `<tool_call>` tokens, no \
function-style calls, no inline JSON outside the fence). The result \
will arrive as a follow-up `tool` role message.";

/// Returns true when `content` starts with `marker` (case-insensitive)
/// followed by whitespace or end-of-input. Used to detect opt-in slash
/// commands like `/loop` and `/web` without false-positive matches on
/// lookalike prefixes (e.g. `/looper`, `/research-grade`).
fn is_slash_prefixed(content: &str, marker: &str) -> bool {
    debug_assert!(
        marker.starts_with('/') && !marker[1..].is_empty(),
        "marker must be a non-empty slash-command"
    );
    let trimmed = content.trim_start();
    let Some(rest) = trimmed.get(..marker.len()) else {
        return false;
    };
    if !rest.eq_ignore_ascii_case(marker) {
        return false;
    }
    match trimmed[marker.len()..].chars().next() {
        None => true,
        Some(c) => c.is_whitespace(),
    }
}

/// Returns true when `content` starts with the `/loop` opt-in marker. The
/// marker is matched case-insensitively and must be followed by whitespace
/// or end-of-input so that, e.g., a sentence starting with "/looper" or
/// "/loops" isn't misread as the opt-in.
fn is_loop_prefixed(content: &str) -> bool {
    is_slash_prefixed(content, "/loop")
}

/// Returns true when the user opted into web search for this turn with
/// `/web`. See [`is_slash_prefixed`] for the matching rules.
fn is_web_prefixed(content: &str) -> bool {
    is_slash_prefixed(content, "/web")
}

/// Returns true when the user opted into multi-source deep research for
/// this turn with `/research`. See [`is_slash_prefixed`] for the matching
/// rules.
fn is_research_prefixed(content: &str) -> bool {
    is_slash_prefixed(content, "/research")
}

/// Resolve the effective per-turn capability flags for the round about
/// to run. Prefers the structured `turn_overrides` column the composer
/// now writes; falls back to the legacy slash-prefix scan on the
/// message content for rows persisted before the column existed so
/// existing chats keep working without a data migration.
///
/// `/no_think` is intentionally **not** part of the fallback: the new
/// default for every turn is "no thinking" regardless of model family,
/// so a legacy message that typed `/no_think` will just be redundantly
/// silent. Users who want thinking opt in via the composer toggle
/// (`overrides.think = true`).
fn resolve_turn_overrides(latest_user: Option<&Message>) -> chat::TurnOverrides {
    let Some(msg) = latest_user else {
        return chat::TurnOverrides::default();
    };
    if let Some(o) = msg.turn_overrides {
        return o;
    }
    chat::TurnOverrides {
        web: is_web_prefixed(&msg.content),
        research: is_research_prefixed(&msg.content),
        // Legacy messages never opted into thinking via a slash command
        // (`/no_think` only suppressed it, which is now the default).
        think: false,
        loop_mode: is_loop_prefixed(&msg.content),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn classify_unreachable_produces_provider_specific_hint() {
        let err: anyhow::Error = UpstreamError::Unreachable {
            attempts: 4,
            last: "connection refused".into(),
        }
        .into();
        let report = classify(&err, "ovms", "http://127.0.0.1:8000/v3");
        assert!(matches!(report.kind, ChatErrorKind::UpstreamUnreachable));
        assert!(report.message.contains("4 attempts"));
        let hint = report.hint.expect("hint");
        assert!(hint.to_ascii_lowercase().contains("ovms"));
        assert_eq!(report.provider_kind.as_deref(), Some("ovms"));
        assert_eq!(report.base_url.as_deref(), Some("http://127.0.0.1:8000/v3"));
        assert!(report.kind.retryable());
    }

    #[test]
    fn classify_http_401_suggests_api_key() {
        let err: anyhow::Error = UpstreamError::Http {
            status: StatusCode::UNAUTHORIZED,
            body: "missing token".into(),
        }
        .into();
        let report = classify(&err, "vllm", "http://example/v1");
        assert!(matches!(report.kind, ChatErrorKind::UpstreamHttp));
        let hint = report.hint.expect("hint");
        assert!(hint.to_ascii_lowercase().contains("api key"));
    }

    #[test]
    fn classify_http_unknown_status_has_no_specific_hint() {
        let err: anyhow::Error = UpstreamError::Http {
            status: StatusCode::IM_A_TEAPOT,
            body: "weird".into(),
        }
        .into();
        let report = classify(&err, "ovms", "http://example");
        assert!(matches!(report.kind, ChatErrorKind::UpstreamHttp));
        assert!(report.hint.is_none());
    }

    #[test]
    fn classify_unknown_error_falls_through_to_other() {
        let err = anyhow::anyhow!("db locked");
        let report = classify(&err, "ovms", "http://example");
        assert!(matches!(report.kind, ChatErrorKind::Other));
        assert_eq!(report.message, "db locked");
        assert!(report.kind.retryable());
    }

    #[test]
    fn truncate_keeps_short_strings_intact() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_clips_long_strings_with_ellipsis() {
        let out = truncate("abcdefghij", 4);
        assert_eq!(out, "abcd…");
    }

    #[test]
    fn truncate_handles_multibyte_chars() {
        // 4 char-count limit, 4 multi-byte chars -> kept as-is.
        assert_eq!(truncate("éééé", 4), "éééé");
        // 6 chars trimmed to 3 + ellipsis.
        assert_eq!(truncate("éééééé", 3), "ééé…");
    }

    #[test]
    fn retryable_kinds_match_expected_set() {
        assert!(!ChatErrorKind::NoActiveProvider.retryable());
        assert!(!ChatErrorKind::UnsupportedProvider.retryable());
        assert!(!ChatErrorKind::NoModelSelected.retryable());
        assert!(ChatErrorKind::OvmsNotRunning.retryable());
        assert!(ChatErrorKind::UpstreamUnreachable.retryable());
        assert!(ChatErrorKind::UpstreamHttp.retryable());
        assert!(ChatErrorKind::Other.retryable());
    }

    #[tokio::test]
    async fn build_request_messages_skips_empty_assistant_placeholder() {
        let history = vec![
            Message {
                id: "u1".into(),
                conversation_id: "c".into(),
                role: "user".into(),
                content: "hello".into(),
                thinking: None,
                created_at: "now".into(),
                attachments: None,
                turn_overrides: None,
            },
            Message {
                id: "a1".into(),
                conversation_id: "c".into(),
                role: "assistant".into(),
                content: "".into(),
                thinking: None,
                created_at: "now".into(),
                attachments: None,
                turn_overrides: None,
            },
        ];
        let settings = Settings::default();
        let out = build_request_messages(&history, &settings, &[], false, false, true, false, "")
            .await
            .unwrap();
        // system + user only (empty assistant placeholder dropped).
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].role, "system");
        assert_eq!(out[1].role, "user");
        match out[1].content.as_ref() {
            Some(crate::llm::MessageContent::Text(t)) => assert_eq!(t, "hello"),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    /// When the loaded model lacks an OVMS `tool_parser`, the runner
    /// must inject the legacy fenced-JSON instructions into the system
    /// prompt so the model has a parseable text protocol. With a
    /// parser present, those instructions must NOT appear — they would
    /// only confuse a model that already speaks the OpenAI native
    /// protocol.
    #[tokio::test]
    async fn build_system_prompt_injects_legacy_protocol_only_when_native_unsupported() {
        use crate::mcp::ToolSchema;
        let settings = Settings::default();
        let tool = EnabledTool {
            server_id: "builtin".into(),
            server_name: "Built-in".into(),
            schema: ToolSchema {
                name: "fs.list".into(),
                description: "List a directory".into(),
                input_schema: serde_json::json!({"type": "object"}),
                destructive: false,
            },
        };

        let with_parser = build_system_prompt(
            &settings,
            std::slice::from_ref(&tool),
            false,
            false,
            true,
            false,
            "",
        )
        .await;
        assert!(
            !with_parser.contains("```tool_use"),
            "native-supported prompt must not teach the legacy protocol"
        );
        assert!(
            !with_parser.contains("Tool-call protocol"),
            "native-supported prompt must not include the legacy header"
        );

        let without_parser = build_system_prompt(
            &settings,
            std::slice::from_ref(&tool),
            false,
            false,
            false,
            false,
            "",
        )
        .await;
        assert!(
            without_parser.contains("```tool_use"),
            "unsupported-runtime prompt must teach the legacy protocol"
        );
        assert!(
            without_parser.contains("Tool-call protocol"),
            "unsupported-runtime prompt must include the legacy header"
        );
        // Catalogue listing is still rendered in both cases; the
        // protocol section is purely additive.
        assert!(without_parser.contains("# Available tools"));
        assert!(with_parser.contains("# Available tools"));
    }

    /// Belt-and-braces: when the catalogue is empty (no tools enabled)
    /// the legacy protocol hint must be skipped even if the runtime
    /// can't parse native tool calls — there's nothing for the model
    /// to call so the section would be noise.
    #[tokio::test]
    async fn build_system_prompt_omits_legacy_protocol_when_no_tools_enabled() {
        let settings = Settings::default();
        let prompt = build_system_prompt(&settings, &[], false, false, false, false, "").await;
        assert!(!prompt.contains("```tool_use"));
        assert!(!prompt.contains("Tool-call protocol"));
    }

    /// With no skills enabled, the `# Skills` catalog section must be
    /// entirely absent — no dangling header, no instructions about a
    /// `skill` tool the model couldn't call anyway (the catalog layer
    /// also hides the built-in in that case).
    #[tokio::test]
    async fn build_system_prompt_omits_skills_section_when_none_enabled() {
        let settings = Settings::default();
        let prompt = build_system_prompt(&settings, &[], false, false, true, false, "").await;
        assert!(
            !prompt.contains("# Skills"),
            "skills section must not appear when no skills are enabled"
        );
        assert!(
            !prompt.contains("`skill` built-in tool"),
            "on-demand-load instructions must not appear when no skills are enabled"
        );
    }

    /// Enabled-but-missing skills must be dropped silently so a typo in
    /// `skills_enabled` doesn't leave a half-rendered section. If every
    /// id fails to load the whole header should be omitted.
    #[tokio::test]
    async fn build_system_prompt_drops_missing_skills_without_header() {
        let mut settings = Settings::default();
        // Ids that are valid (URL-safe) but extremely unlikely to exist
        // on the developer / CI machine running this test.
        settings.skills_enabled = vec![
            "__zero_test_missing_a__".into(),
            "__zero_test_missing_b__".into(),
        ];
        let prompt = build_system_prompt(&settings, &[], false, false, true, false, "").await;
        assert!(
            !prompt.contains("# Skills"),
            "header must be omitted when every enabled skill is missing on disk"
        );
        assert!(
            !prompt.contains("__zero_test_missing_a__"),
            "missing ids must not leak into the prompt"
        );
    }

    // ─── ModelProfile ──────────────────────────────────────────────────────
    //
    // Pin the per-family sampling / multimodal / prompt-token decisions
    // so a refactor of [`model_profile`] can't silently regress the
    // documented Gemma 4 best-practice numbers.

    #[test]
    fn is_gemma4_family_matches_documented_id_conventions() {
        for id in [
            "google/gemma-4-E2B-it",
            "google/gemma-4-E4B-it",
            "OpenVINO/gemma-4-E2B-it-int8-ov",
            "gemma4",
            "GEMMA-4-9b",
            "my_local/gemma_4_finetune",
        ] {
            assert!(is_gemma4_family(id), "{id}: should match Gemma 4");
        }
    }

    #[test]
    fn is_gemma4_family_rejects_older_gemmas_and_unrelated_models() {
        // We intentionally exclude Gemma 1/2/3: their chat template does
        // NOT understand the `<|think|>` control token, so applying the
        // Gemma 4 profile to them would leak the token as literal text.
        for id in [
            "google/gemma-2-9b-it",
            "google/gemma-3-4b-it",
            "OpenVINO/gemma-2b-it-int4-ov",
            "meta-llama/Llama-3.1-8B-Instruct",
            "Qwen/Qwen3-7B-Instruct",
            "",
        ] {
            assert!(!is_gemma4_family(id), "{id}: must not match Gemma 4");
        }
    }

    #[test]
    fn gemma4_profile_matches_documented_sampling_numbers() {
        let p = model_profile("google/gemma-4-E2B-it");
        // temperature=1.0, top_p=0.95, top_k=64 per the Gemma 4 docs.
        assert!((p.temperature - 1.0).abs() < 1e-6, "got {}", p.temperature);
        assert_eq!(p.top_p, Some(0.95));
        assert_eq!(p.top_k, Some(64));
        // Multimodal turns must place image content before text.
        assert!(p.images_before_text);
        // Thinking is gated on the `<|think|>` system-prompt prefix.
        assert_eq!(p.thinking_control_token, Some("<|think|>"));
    }

    // ─── resolve_sampling ────────────────────────────────────────────
    //
    // Conversation → provider → profile precedence is the contract the
    // chat popover + Settings page both depend on; pin it explicitly so
    // a refactor of the merge can't silently swap layers.

    #[test]
    fn resolve_sampling_falls_through_to_profile_when_no_overrides() {
        let profile = model_profile("google/gemma-4-E2B-it");
        let out = resolve_sampling(
            &SamplingConfig::default(),
            &SamplingConfig::default(),
            profile,
        );
        // Bare resolution → profile values verbatim.
        assert!((out.temperature - profile.temperature).abs() < 1e-6);
        assert_eq!(out.top_p, profile.top_p);
        assert_eq!(out.top_k, profile.top_k);
    }

    #[test]
    fn resolve_sampling_provider_overrides_profile_per_field() {
        let profile = model_profile("google/gemma-4-E2B-it");
        let provider = SamplingConfig {
            temperature: Some(0.2),
            top_p: None, // intentionally left alone
            top_k: Some(40),
        };
        let out = resolve_sampling(&SamplingConfig::default(), &provider, profile);
        // temperature + top_k come from the provider; top_p falls back
        // to the Gemma 4 profile default (0.95) since neither override
        // touched it.
        assert!(
            (out.temperature - 0.2).abs() < 1e-6,
            "got {}",
            out.temperature
        );
        assert_eq!(out.top_p, profile.top_p);
        assert_eq!(out.top_k, Some(40));
    }

    #[test]
    fn resolve_sampling_conversation_wins_over_provider() {
        let profile = model_profile("google/gemma-4-E2B-it");
        let provider = SamplingConfig {
            temperature: Some(0.2),
            top_p: Some(0.5),
            top_k: Some(40),
        };
        let conv = SamplingConfig {
            temperature: Some(1.5),
            top_p: None, // chat doesn't touch top_p
            top_k: Some(8),
        };
        let out = resolve_sampling(&conv, &provider, profile);
        // Chat wins for the fields it sets.
        assert!((out.temperature - 1.5).abs() < 1e-6);
        assert_eq!(out.top_k, Some(8));
        // Chat left top_p alone → provider's value carries through.
        assert_eq!(out.top_p, Some(0.5));
    }

    #[test]
    fn resolve_sampling_default_profile_can_have_no_top_p_top_k() {
        // Non-Gemma 4 model with no overrides anywhere → top_p / top_k
        // stay `None` so the request omits them on the wire (preserves
        // the historical OVMS-friendly default).
        let profile = model_profile("meta-llama/Llama-3.1-8B-Instruct");
        let out = resolve_sampling(
            &SamplingConfig::default(),
            &SamplingConfig::default(),
            profile,
        );
        assert!(out.top_p.is_none());
        assert!(out.top_k.is_none());
        assert!((out.temperature - 0.7).abs() < 1e-6);
    }

    #[test]
    fn default_profile_preserves_legacy_runner_behaviour() {
        let p = model_profile("meta-llama/Llama-3.1-8B-Instruct");
        // Old hardcoded temperature was 0.7 — keep it for non-Gemma 4
        // models so this change is invisible to existing chats.
        assert!((p.temperature - 0.7).abs() < 1e-6, "got {}", p.temperature);
        assert!(p.top_p.is_none());
        assert!(p.top_k.is_none());
        assert!(!p.images_before_text);
        assert!(p.thinking_control_token.is_none());
    }

    /// When the loaded model is Gemma 4 and the per-turn `think_enabled`
    /// flag is on, the system prompt MUST start with the literal `<|think|>`
    /// control token (the model's documented mechanism for enabling the
    /// reasoning trace).
    #[tokio::test]
    async fn build_system_prompt_prepends_thinking_token_for_gemma4_when_enabled() {
        let settings = Settings::default();
        let prompt = build_system_prompt(
            &settings,
            &[],
            false,
            // think_enabled = true — simulates the composer's per-turn
            // "Thinking" toggle being on for this round.
            true,
            true,
            false,
            "google/gemma-4-E2B-it",
        )
        .await;
        assert!(
            prompt.starts_with("<|think|>"),
            "expected Gemma 4 thinking token prefix, got: {prompt:?}"
        );
    }

    /// Conversely, when the per-turn `think_enabled` flag is off (the
    /// default for every turn now), the token must NOT appear —
    /// omitting it is the documented way to suppress the reasoning
    /// trace on Gemma 4.
    #[tokio::test]
    async fn build_system_prompt_omits_thinking_token_for_gemma4_when_disabled() {
        let settings = Settings::default();
        let prompt = build_system_prompt(
            &settings,
            &[],
            false,
            false,
            true,
            false,
            "google/gemma-4-E2B-it",
        )
        .await;
        assert!(
            !prompt.contains("<|think|>"),
            "Gemma 4 thinking token must not appear when thinking is disabled"
        );
    }

    /// Non-Gemma 4 models must never see the `<|think|>` token — it
    /// would land in their prompt as literal junk text.
    #[tokio::test]
    async fn build_system_prompt_never_emits_thinking_token_for_other_families() {
        let settings = Settings::default();
        for model in [
            "",
            "meta-llama/Llama-3.1-8B-Instruct",
            "Qwen/Qwen3-7B-Instruct",
            "google/gemma-3-4b-it",
        ] {
            let prompt = build_system_prompt(&settings, &[], false, true, true, false, model).await;
            assert!(
                !prompt.contains("<|think|>"),
                "{model}: must not emit Gemma 4 thinking token"
            );
        }
    }

    /// Gemma 4 multimodal turns place image content before the user
    /// text, per the model card's "modality order" guidance.
    #[tokio::test]
    async fn build_request_messages_puts_images_first_for_gemma4() {
        use crate::chat::Attachment;
        let img = Attachment {
            kind: "image".into(),
            path: nonexistent_image_path(),
            mime: "image/png".into(),
            bytes: 8,
            name: "diagram.png".into(),
        };
        let history = vec![Message {
            id: "u1".into(),
            conversation_id: "c".into(),
            role: "user".into(),
            content: "describe this".into(),
            thinking: None,
            created_at: "now".into(),
            attachments: Some(vec![img]),
            turn_overrides: None,
        }];
        let settings = Settings::default();

        // Gemma 4 → image part should land before the text part.
        let out = build_request_messages(
            &history,
            &settings,
            &[],
            false,
            false,
            true,
            false,
            "google/gemma-4-E2B-it",
        )
        .await
        .unwrap();
        let user = &out[1];
        assert_eq!(user.role, "user");
        match user.content.as_ref() {
            Some(crate::llm::MessageContent::Parts(parts)) => {
                assert_eq!(parts.len(), 2, "expected image + text parts");
                // First slot must be the image (or its `[image unavailable]`
                // text fallback when the file can't be read — the test
                // only cares about ordering, not the encoder outcome).
                let first_is_image_or_fallback = match &parts[0] {
                    crate::llm::ContentPart::ImageUrl { .. } => true,
                    crate::llm::ContentPart::Text { text } => {
                        text.starts_with("[image unavailable")
                    }
                };
                assert!(
                    first_is_image_or_fallback,
                    "Gemma 4 must put image first, got {:?}",
                    parts[0]
                );
                assert!(
                    matches!(&parts[1], crate::llm::ContentPart::Text { text } if text == "describe this"),
                    "second part must be the user text, got {:?}",
                    parts[1]
                );
            }
            other => panic!("expected multimodal parts, got {other:?}"),
        }
    }

    /// Non-Gemma 4 models keep the historical text-first layout so this
    /// change can't regress callers we already tuned against.
    #[tokio::test]
    async fn build_request_messages_keeps_text_first_for_other_families() {
        use crate::chat::Attachment;
        let img = Attachment {
            kind: "image".into(),
            path: nonexistent_image_path(),
            mime: "image/png".into(),
            bytes: 8,
            name: "diagram.png".into(),
        };
        let history = vec![Message {
            id: "u1".into(),
            conversation_id: "c".into(),
            role: "user".into(),
            content: "describe this".into(),
            thinking: None,
            created_at: "now".into(),
            attachments: Some(vec![img]),
            turn_overrides: None,
        }];
        let settings = Settings::default();
        let out = build_request_messages(
            &history,
            &settings,
            &[],
            false,
            false,
            true,
            false,
            "meta-llama/Llama-3.1-8B-Instruct",
        )
        .await
        .unwrap();
        let user = &out[1];
        match user.content.as_ref() {
            Some(crate::llm::MessageContent::Parts(parts)) => {
                assert_eq!(parts.len(), 2);
                assert!(
                    matches!(&parts[0], crate::llm::ContentPart::Text { text } if text == "describe this"),
                    "first part must be the user text for non-Gemma 4, got {:?}",
                    parts[0]
                );
            }
            other => panic!("expected multimodal parts, got {other:?}"),
        }
    }

    /// Returns a path guaranteed not to exist on disk so the image
    /// encoder falls through to its `[image unavailable]` Text part.
    /// Lets the multimodal-ordering tests exercise the real code path
    /// without needing a fixture PNG.
    fn nonexistent_image_path() -> String {
        std::env::temp_dir()
            .join("zero-test-does-not-exist-9f3b7c2a.png")
            .to_string_lossy()
            .into_owned()
    }

    /// The descriptions in the `# Skills` listing are hard-capped so a
    /// verbose skill blurb can't dominate the system prompt. The full
    /// body is fetched on demand anyway, so a long catalogue entry just
    /// wastes turn-1 tokens without helping the model decide whether to
    /// load the skill.
    #[test]
    fn truncate_skill_description_caps_at_max_chars() {
        let big = "a".repeat(MAX_SKILL_DESC_CHARS * 3);
        let out = truncate_skill_description(&big);
        assert!(out.chars().count() <= MAX_SKILL_DESC_CHARS);
        assert!(out.ends_with('\u{2026}'));

        let small = "short";
        assert_eq!(truncate_skill_description(small), "short");

        // Multi-byte chars must not be sliced mid-codepoint.
        let unicode = "\u{1f600}".repeat(MAX_SKILL_DESC_CHARS + 50);
        let out = truncate_skill_description(&unicode);
        assert!(out.chars().count() <= MAX_SKILL_DESC_CHARS);
    }

    /// When using qwen3/qwen3.5 models with thinking disabled,
    /// the runner automatically sets chat_template_kwargs with
    /// {"enable_thinking": false} in the API request.
    #[tokio::test]
    async fn chat_request_includes_enable_thinking_false_for_qwen3_when_thinking_disabled() {
        // This test verifies that the chat_template_kwargs field is properly
        // set in the ChatRequest, which is what actually controls thinking
        // for Qwen models (not appending /no_think to the text).
        // The actual test would need access to the ChatRequest being built,
        // which happens inside run_inner. For now, we document the expected
        // behavior: when should_disable_thinking is true (qwen3 reasoning_parser
        // + thinking disabled), the ChatRequest should include:
        // chat_template_kwargs: Some(json!({"enable_thinking": false}))
    }

    // ─── StreamAccumulator ────────────────────────────────────────────────
    //
    // The accumulator owns the per-chunk state machine that the runner's
    // select! loop drives. These tests pin its semantics without standing
    // up a Tauri runtime, a SQLite pool, or even a tokio executor.

    use std::cell::RefCell;

    fn chunk(delta: &str, thinking: bool) -> ChatChunk {
        ChatChunk {
            delta: delta.into(),
            thinking,
            done: false,
            ..ChatChunk::default()
        }
    }

    fn done_chunk() -> ChatChunk {
        ChatChunk {
            done: true,
            ..ChatChunk::default()
        }
    }

    #[test]
    fn step_content_chunk_pushes_to_content_buffer_and_emits_once() {
        let mut acc = StreamAccumulator::new();
        let calls: RefCell<Vec<(String, bool)>> = RefCell::new(Vec::new());
        let outcome = acc.step(chunk("hello ", false), |d, t| {
            calls.borrow_mut().push((d.to_string(), t));
        });
        assert_eq!(outcome, StepOutcome::Continue);
        assert_eq!(acc.content, "hello ");
        assert!(acc.thinking.is_empty());
        assert_eq!(calls.borrow().as_slice(), &[("hello ".to_string(), false)]);
    }

    #[test]
    fn step_thinking_chunk_routes_to_thinking_buffer() {
        let mut acc = StreamAccumulator::new();
        let mut calls: Vec<(String, bool)> = Vec::new();
        acc.step(chunk("reflecting…", true), |d, t| {
            calls.push((d.to_string(), t));
        });
        assert_eq!(acc.thinking, "reflecting…");
        assert!(acc.content.is_empty());
        assert_eq!(calls, vec![("reflecting…".to_string(), true)]);
    }

    #[test]
    fn step_empty_delta_is_continue_and_does_not_emit() {
        let mut acc = StreamAccumulator::new();
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let outcome = acc.step(chunk("", false), |d, _| {
            calls.borrow_mut().push(d.to_string());
        });
        assert_eq!(outcome, StepOutcome::Continue);
        assert!(acc.content.is_empty());
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn step_done_chunk_signals_end_of_stream_without_emitting() {
        let mut acc = StreamAccumulator::new();
        let calls: RefCell<u32> = RefCell::new(0);
        let outcome = acc.step(done_chunk(), |_, _| *calls.borrow_mut() += 1);
        assert_eq!(outcome, StepOutcome::EndOfStream);
        assert!(acc.content.is_empty());
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn step_done_chunk_with_payload_still_ends_stream_and_keeps_prior_buffers() {
        // Some providers attach a final partial delta to the same frame as
        // `done: true`. The accumulator currently treats `done` as
        // authoritative — the trailing delta is dropped. Lock that in so
        // future refactors don't accidentally duplicate the last token.
        let mut acc = StreamAccumulator::new();
        acc.step(chunk("abc", false), |_, _| {});
        let trailing = ChatChunk {
            delta: "DROP_ME".into(),
            thinking: false,
            done: true,
            ..ChatChunk::default()
        };
        let outcome = acc.step(trailing, |_, _| panic!("on_delta should not fire on done"));
        assert_eq!(outcome, StepOutcome::EndOfStream);
        assert_eq!(acc.content, "abc");
    }

    #[test]
    fn step_sequence_accumulates_in_order() {
        let mut acc = StreamAccumulator::new();
        let calls: RefCell<Vec<(String, bool)>> = RefCell::new(Vec::new());
        let emit = |d: &str, t: bool| calls.borrow_mut().push((d.to_string(), t));

        acc.step(chunk("think ", true), emit);
        acc.step(chunk("more.", true), emit);
        acc.step(chunk("hello ", false), emit);
        acc.step(chunk("world", false), emit);
        let final_outcome = acc.step(done_chunk(), emit);

        assert_eq!(final_outcome, StepOutcome::EndOfStream);
        assert_eq!(acc.content, "hello world");
        assert_eq!(acc.thinking, "think more.");
        assert_eq!(
            calls.borrow().as_slice(),
            &[
                ("think ".to_string(), true),
                ("more.".to_string(), true),
                ("hello ".to_string(), false),
                ("world".to_string(), false),
            ]
        );
    }

    #[test]
    fn thinking_as_opt_returns_none_when_empty_and_some_otherwise() {
        let mut acc = StreamAccumulator::new();
        assert_eq!(acc.thinking_as_opt(), None);
        acc.step(chunk("x", true), |_, _| {});
        assert_eq!(acc.thinking_as_opt(), Some("x"));
    }

    #[test]
    fn finalize_normal_returns_buffers_as_is() {
        let mut acc = StreamAccumulator::new();
        acc.step(chunk("hi", false), |_, _| {});
        acc.step(chunk("th", true), |_, _| {});
        let (content, thinking) = acc.finalize(false);
        assert_eq!(content, "hi");
        assert_eq!(thinking.as_deref(), Some("th"));
    }

    #[test]
    fn finalize_cancelled_empty_writes_cancelled_marker() {
        let acc = StreamAccumulator::new();
        let (content, thinking) = acc.finalize(true);
        assert_eq!(content, "[cancelled]");
        assert_eq!(thinking, None);
    }

    #[test]
    fn finalize_cancelled_with_partial_buffers_preserves_them() {
        let mut acc = StreamAccumulator::new();
        acc.step(chunk("partial", false), |_, _| {});
        let (content, thinking) = acc.finalize(true);
        // Cancellation should NOT clobber content the model already produced.
        assert_eq!(content, "partial");
        assert_eq!(thinking, None);
    }

    #[test]
    fn finalize_returns_none_thinking_when_only_content_present() {
        let mut acc = StreamAccumulator::new();
        acc.step(chunk("only content", false), |_, _| {});
        let (content, thinking) = acc.finalize(false);
        assert_eq!(content, "only content");
        assert_eq!(thinking, None);
    }

    // ─── special-token stripping ─────────────────────────────────
    //
    // Locks in the leak-prevention behaviour for tokens like `<eos>` and
    // `<|tool_response>` that occasionally arrive verbatim in OVMS's
    // `delta.content` stream. The contract is: tokens must never reach
    // the on_delta callback or the persisted content buffer, even when
    // they are split across chunk boundaries.

    #[test]
    fn stripper_removes_token_embedded_in_a_single_chunk() {
        let mut s = SpecialTokenStripper::default();
        let out = s.push("hello<eos>world");
        assert_eq!(out, "helloworld");
        // No held-back tail at end-of-stream.
        assert_eq!(s.flush(), "");
    }

    #[test]
    fn stripper_strips_token_split_across_chunk_boundary() {
        let mut s = SpecialTokenStripper::default();
        // First chunk ends mid-token — nothing visible yet.
        assert_eq!(s.push("abc<|tool_resp"), "abc");
        // Second chunk completes the token; only the post-token text is emitted.
        assert_eq!(s.push("onse>def"), "def");
        assert_eq!(s.flush(), "");
    }

    #[test]
    fn stripper_holds_back_then_flushes_innocuous_lookalike_prefix() {
        let mut s = SpecialTokenStripper::default();
        // Looks like the start of `<eos>` so it gets held back …
        assert_eq!(s.push("text <eo"), "text ");
        // … but the stream ends before any token completes, so the
        // held-back fragment must reach the user as plain text.
        assert_eq!(s.flush(), "<eo");
    }

    #[test]
    fn stripper_passes_text_with_no_specials_through_immediately() {
        let mut s = SpecialTokenStripper::default();
        assert_eq!(
            s.push("just regular content, no tags here"),
            "just regular content, no tags here"
        );
        assert_eq!(s.flush(), "");
    }

    #[test]
    fn stripper_handles_overlapping_partial_then_different_token() {
        let mut s = SpecialTokenStripper::default();
        // `<|to` is the prefix of both `<|tool_call>` and `<|tool_response>`.
        assert_eq!(s.push("<|to"), "");
        // The disambiguating bytes complete `<|tool_call>`; nothing visible.
        assert_eq!(s.push("ol_call>{}"), "{}");
        assert_eq!(s.flush(), "");
    }

    #[test]
    fn step_strips_eos_from_a_content_chunk_before_emitting() {
        let mut acc = StreamAccumulator::new();
        let calls: RefCell<Vec<(String, bool)>> = RefCell::new(Vec::new());
        acc.step(chunk("done<eos>", false), |d, t| {
            calls.borrow_mut().push((d.to_string(), t));
        });
        acc.step(done_chunk(), |d, t| {
            calls.borrow_mut().push((d.to_string(), t));
        });
        assert_eq!(acc.content, "done");
        assert_eq!(calls.borrow().as_slice(), &[("done".to_string(), false)]);
    }

    #[test]
    fn step_holds_back_partial_token_and_emits_after_disambiguation() {
        let mut acc = StreamAccumulator::new();
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let mut emit = |d: &str, _: bool| calls.borrow_mut().push(d.to_string());

        // Pure prefix — nothing visible yet.
        acc.step(chunk("prefix<|too", false), &mut emit);
        assert_eq!(*calls.borrow(), vec!["prefix".to_string()]);
        assert_eq!(acc.content, "prefix");

        // Completes `<|tool_response>` and adds trailing text.
        acc.step(chunk("l_response>suffix", false), &mut emit);
        assert_eq!(
            *calls.borrow(),
            vec!["prefix".to_string(), "suffix".to_string()],
        );
        assert_eq!(acc.content, "prefixsuffix");
    }

    #[test]
    fn step_flushes_innocuous_held_back_text_on_done() {
        let mut acc = StreamAccumulator::new();
        let calls: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let mut emit = |d: &str, _: bool| calls.borrow_mut().push(d.to_string());

        // Trailing `<eo` looks like the start of `<eos>` and is held back.
        acc.step(chunk("keep <eo", false), &mut emit);
        assert_eq!(*calls.borrow(), vec!["keep ".to_string()]);
        // The stream ends without `<eo` ever growing into `<eos>` — the
        // held fragment must surface as visible text on the done chunk
        // so the UI buffer matches `acc.content`.
        acc.step(done_chunk(), &mut emit);
        assert_eq!(
            *calls.borrow(),
            vec!["keep ".to_string(), "<eo".to_string()],
        );
        assert_eq!(acc.content, "keep <eo");
    }

    #[test]
    fn step_does_not_strip_special_tokens_from_thinking_stream() {
        let mut acc = StreamAccumulator::new();
        let calls: RefCell<Vec<(String, bool)>> = RefCell::new(Vec::new());
        acc.step(chunk("<|think|>internal<eos>", true), |d, t| {
            calls.borrow_mut().push((d.to_string(), t));
        });
        // Thinking content is forwarded verbatim.
        assert_eq!(acc.thinking, "<|think|>internal<eos>");
        assert_eq!(
            calls.borrow().as_slice(),
            &[("<|think|>internal<eos>".to_string(), true)],
        );
    }

    // ─── tool-call protocol ─────────────────────────────────────────────

    #[test]
    fn parse_tool_call_extracts_server_tool_and_arguments() {
        let text = "sure, let me check\n\n```tool_use\n{\"server\":\"alpha\",\"tool\":\"search\",\"arguments\":{\"q\":\"rust\"}}\n```\n";
        let call = parse_tool_call(text).expect("should parse");
        assert_eq!(call.server, "alpha");
        assert_eq!(call.tool, "search");
        assert_eq!(call.arguments["q"], serde_json::json!("rust"));
        // fence_start lets the runner trim everything from the fence onward.
        assert!(&text[..call.fence_start].ends_with("check\n\n"));
    }

    #[test]
    fn parse_tool_call_accepts_alt_keys_and_alt_label() {
        let text = "```tool_call\n{\"name\":\"fetch\",\"args\":{\"url\":\"https://x\"}}\n```";
        let call = parse_tool_call(text).expect("should parse name/args/tool_call");
        assert_eq!(call.tool, "fetch");
        assert_eq!(call.server, "");
        assert_eq!(call.arguments["url"], serde_json::json!("https://x"));
    }

    #[test]
    fn parse_tool_call_returns_none_for_plain_text() {
        assert!(parse_tool_call("just a normal reply").is_none());
        // Fenced JSON that isn't a tool_use label is ignored.
        assert!(parse_tool_call("```json\n{\"tool\":\"x\"}\n```").is_none());
        // Missing `tool` / `name` is rejected even with a valid envelope.
        assert!(parse_tool_call("```tool_use\n{\"server\":\"a\"}\n```").is_none());
    }

    #[test]
    fn is_loop_prefixed_detects_the_opt_in_marker() {
        // Canonical forms.
        assert!(is_loop_prefixed("/loop list everything under C:/"));
        assert!(is_loop_prefixed("/loop\nplease chain tools"));
        assert!(is_loop_prefixed("/loop"));
        // Leading whitespace before the marker is tolerated — users often
        // hit space before typing the slash.
        assert!(is_loop_prefixed("   /loop do the thing"));
        // Case-insensitive.
        assert!(is_loop_prefixed("/LOOP do it"));
        assert!(is_loop_prefixed("/Loop do it"));
    }

    #[test]
    fn is_loop_prefixed_rejects_lookalike_prefixes() {
        // No marker at all.
        assert!(!is_loop_prefixed("list files in C:/"));
        // Marker appears mid-message, not at the start.
        assert!(!is_loop_prefixed("please /loop this"));
        // Substring matches that aren't the standalone marker must not
        // count, otherwise `/looper` would silently flip into agent mode.
        assert!(!is_loop_prefixed("/looper run"));
        assert!(!is_loop_prefixed("/loops"));
        // Different slash-command.
        assert!(!is_loop_prefixed("/help"));
        // Empty / whitespace-only input.
        assert!(!is_loop_prefixed(""));
        assert!(!is_loop_prefixed("   "));
    }

    #[test]
    fn is_web_prefixed_detects_marker_with_same_rules_as_loop() {
        // Canonical forms.
        assert!(is_web_prefixed("/web latest openvino release"));
        assert!(is_web_prefixed("/web"));
        assert!(is_web_prefixed("  /WEB rust async book"));
        // Lookalikes must not trigger.
        assert!(!is_web_prefixed("/website rebuild"));
        assert!(!is_web_prefixed("/webhook listener"));
        assert!(!is_web_prefixed("please /web this"));
        assert!(!is_web_prefixed(""));
    }

    #[test]
    fn is_research_prefixed_detects_marker_with_same_rules_as_loop() {
        assert!(is_research_prefixed("/research compare GPT-4 vs Claude"));
        assert!(is_research_prefixed("/RESEARCH"));
        assert!(is_research_prefixed("  /Research llama 3 benchmarks"));
        // Lookalikes must not trigger.
        assert!(!is_research_prefixed("/researcher-mode on"));
        assert!(!is_research_prefixed("/researched topics"));
        assert!(!is_research_prefixed("please /research this"));
        assert!(!is_research_prefixed(""));
    }

    #[test]
    fn resolve_tool_prefers_exact_server_match_then_falls_back_by_name() {
        let tools = vec![
            EnabledTool {
                server_id: "alpha".into(),
                server_name: "".into(),
                schema: crate::mcp::ToolSchema {
                    name: "search".into(),
                    description: String::new(),
                    input_schema: serde_json::json!({}),
                    destructive: false,
                },
            },
            EnabledTool {
                server_id: "beta".into(),
                server_name: "".into(),
                schema: crate::mcp::ToolSchema {
                    name: "search".into(),
                    description: String::new(),
                    input_schema: serde_json::json!({}),
                    destructive: false,
                },
            },
        ];
        let exact = ParsedToolCall {
            fence_start: 0,
            server: "beta".into(),
            tool: "search".into(),
            arguments: serde_json::json!({}),
        };
        assert_eq!(resolve_tool(&tools, &exact).unwrap().server_id, "beta");

        // Missing server falls back to first matching tool name.
        let no_server = ParsedToolCall {
            fence_start: 0,
            server: String::new(),
            tool: "search".into(),
            arguments: serde_json::json!({}),
        };
        assert_eq!(resolve_tool(&tools, &no_server).unwrap().server_id, "alpha");

        // Unknown name → None.
        let unknown = ParsedToolCall {
            fence_start: 0,
            server: String::new(),
            tool: "nope".into(),
            arguments: serde_json::json!({}),
        };
        assert!(resolve_tool(&tools, &unknown).is_none());
    }

    #[test]
    fn render_call_block_contains_server_tool_and_pretty_args() {
        let call = ParsedToolCall {
            fence_start: 0,
            server: "alpha".into(),
            tool: "search".into(),
            arguments: serde_json::json!({ "q": "rust" }),
        };
        let out = render_call_block(&call);
        assert!(out.contains("[tool call: alpha/search]"));
        assert!(out.contains("```json"));
        assert!(out.contains("\"q\": \"rust\""));
    }

    #[test]
    fn render_result_block_truncates_oversized_output() {
        let huge = "x".repeat(8000);
        let out = render_result_block(&huge);
        assert!(out.contains("[truncated]"));
        assert!(out.len() < 8000);
    }

    #[test]
    fn render_thinking_block_returns_empty_string_for_empty_input() {
        // No reasoning trace this round → caller can `push_str` the
        // result unconditionally without producing an empty fence.
        assert_eq!(render_thinking_block(""), "");
    }

    #[test]
    fn render_thinking_block_wraps_text_in_inline_markers() {
        // The format is the contract the frontend parser relies on:
        // `\n[thinking]\n<text>\n[/thinking]\n\n`. The leading newline
        // gives the parser a clean break from the preceding content,
        // and the trailing blank line separates this block from the
        // tool-call banner that follows.
        let out = render_thinking_block("reasoning about the request");
        assert_eq!(
            out,
            "\n[thinking]\nreasoning about the request\n[/thinking]\n\n"
        );
        assert!(out.starts_with("\n[thinking]\n"));
        assert!(out.ends_with("\n[/thinking]\n\n"));
    }

    #[test]
    fn render_thinking_block_trims_trailing_whitespace_only() {
        // Stream chunks routinely leave a trailing newline or space on
        // the accumulated buffer; we strip the right side so the
        // `[/thinking]` marker sits flush against the last line of
        // reasoning. The left side is preserved as-is because models
        // sometimes indent their thinking deliberately.
        let out = render_thinking_block("  one\ntwo  \n");
        assert!(out.contains("  one\ntwo\n[/thinking]"));
        assert!(!out.contains("two  "));
    }

    // ─── tool confirm registry ────────────────────────────────────

    #[tokio::test]
    async fn tool_confirms_register_and_resolve_round_trip() {
        let confirms = ToolConfirms::new();
        let rx = confirms.register("call-1".into()).await;
        assert!(confirms.resolve("call-1", true).await);
        assert_eq!(rx.await.unwrap(), true);
    }

    #[tokio::test]
    async fn tool_confirms_resolve_unknown_id_is_noop() {
        let confirms = ToolConfirms::new();
        assert!(!confirms.resolve("missing", true).await);
    }

    #[tokio::test]
    async fn tool_confirms_forget_drops_sender_silently() {
        let confirms = ToolConfirms::new();
        let rx = confirms.register("call-2".into()).await;
        confirms.forget("call-2").await;
        // Sender was dropped without sending; receiver should see RecvError.
        assert!(rx.await.is_err());
        // A subsequent resolve for the same id is a no-op.
        assert!(!confirms.resolve("call-2", true).await);
    }
}
