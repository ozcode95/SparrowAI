//! External MCP (Model Context Protocol) server registry + JSON-RPC client.
//!
//! Two pieces live here:
//!
//! 1. [`tools`] — placeholder for the built-in tool registry (`shell`,
//!    `fs`, ...) we'll wire in alongside the agent loop in phase 2.
//! 2. [`client`] — minimal HTTP/SSE JSON-RPC client used to enumerate
//!    tools on user-configured external MCP servers and to call them
//!    from the Tools page. We deliberately keep this thin so it can be
//!    plugged into the chat runner later as just another tool surface.
//!
//! External server *configuration* is stored in `Settings.mcp_servers` so
//! it round-trips through the same settings file as everything else; only
//! the runtime client / cached schema lives in this module.

pub mod catalog;
pub mod client;
pub mod tools;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Servers/tools we should warn the user about before calling (file
    /// writes, shell exec, ...). Currently set conservatively from the
    /// tool name — refined when the MCP server advertises explicit
    /// annotations.
    #[serde(default)]
    pub destructive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn schema(&self) -> ToolSchema;
    async fn call(&self, args: Value) -> Result<ToolResult>;
}

/// Synthetic server id used for the in-process built-in tool registry.
/// The chat runner watches for this string in [`crate::mcp::catalog::EnabledTool::server_id`]
/// and routes calls through [`builtin_registry`] instead of the HTTP
/// MCP client.
pub const BUILTIN_SERVER_ID: &str = "builtin";

/// Human-readable label paired with [`BUILTIN_SERVER_ID`] in the tool
/// catalog. Shown in the system prompt header (`### server \`builtin\` — Built-in`)
/// and on the Tools page.
pub const BUILTIN_SERVER_NAME: &str = "Built-in (local)";

/// Build the full set of in-process tools. Tools that need access to
/// shared app state (HTTP client, DB pool, notification plugin) capture
/// it from the supplied [`AppHandle`] at construction time so the
/// [`Tool`] trait itself stays free of Tauri types.
pub fn builtin_registry(app: &AppHandle) -> Vec<Box<dyn Tool>> {
    let mut out: Vec<Box<dyn Tool>> = Vec::new();
    out.extend(tools::fs::all());
    out.extend(tools::code::all());
    out.extend(tools::shell::all());
    out.extend(tools::http::all(app));
    out.extend(tools::web::all(app));
    out.extend(tools::clipboard::all());
    out.extend(tools::task::all(app));
    // Persistent memory editor. Always advertised — even on a fresh
    // install with an empty MEMORY.md / USER.md, the agent needs to
    // be able to save its first note.
    out.extend(tools::memory::all());
    // Lazy tool-discovery placeholder. The chat runner intercepts
    // dispatch and renders the live per-turn catalog itself; the
    // placeholder body only runs when the tool is invoked outside a
    // chat (e.g. the Tools-page Test button) and returns a friendly
    // explanation. Registered last so it shows up at the bottom of
    // the Tools page rather than crowding out the more useful entries.
    out.extend(tools::discovery::all());
    out
}
