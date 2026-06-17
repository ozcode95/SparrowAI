//! Canonical event names emitted from Rust to the frontend.
//! Keep in sync with `src/lib/tauri.ts`.

pub const CHAT_DELTA: &str = "chat://delta";
pub const CHAT_DONE: &str = "chat://done";
pub const CHAT_ERROR: &str = "chat://error";
/// Emitted by the agent loop when it needs to replace the live
/// streaming buffer for an assistant message with a canonical version
/// — used today to clean up legacy ```tool_use``` fenced-JSON blocks
/// that already streamed into the UI before we knew they were a
/// tool-call protocol marker. The frontend handler overwrites the
/// message's `content` rather than appending to it.
pub const CHAT_REWRITE: &str = "chat://rewrite";
/// Emitted when the agent loop wants to invoke a destructive MCP tool
/// while `settings.destructive_tool_confirm` is on. The frontend shows a
/// confirm modal; the user's choice round-trips back via the
/// `chat_tool_confirm` command. The runner blocks the stream waiting for
/// the decision (or chat cancellation).
pub const CHAT_TOOL_CONFIRM: &str = "chat://tool-confirm";
pub const MODELS_DOWNLOAD_PROGRESS: &str = "models://download-progress";
pub const OVMS_LOG: &str = "ovms://log";
pub const OVMS_STATUS: &str = "ovms://status";
pub const OVMS_INSTALL_PROGRESS: &str = "ovms://install-progress";
pub const TASKS_TICK: &str = "tasks://tick";
