//! Discovery tool — exposes the catalog of currently-enabled tools so the
//! model can learn about other tools on demand instead of being shown the
//! full list up-front.
//!
//! ## Why
//!
//! On a fresh chat turn the system prompt + OpenAI `tools` array can run
//! to several thousand tokens once a handful of MCP servers are wired up.
//! Most turns don't need any tool at all, so paying that cost every turn
//! is wasteful. With `lazy_tool_discovery` flipped on in Settings, the
//! runner only ships `tools.list` in the initial request; the model calls
//! it when (and only when) it decides it needs help, receives the live
//! catalog as a tool result, and then the runner expands subsequent
//! rounds in the same turn to expose the full catalog so the model can
//! actually invoke whatever it discovered.
//!
//! ## How
//!
//! The tool itself is a **placeholder** — the chat runner intercepts
//! dispatch and renders the response directly from the in-memory
//! [`crate::mcp::catalog::EnabledTool`] catalog (which is per-turn and
//! already filtered for per-chat disables + slash gates). The placeholder
//! body is a fallback so a direct invocation through
//! [`crate::commands::mcp::mcp_call_tool`] (the manual "Test" button on
//! the Tools page) still returns something sensible instead of an empty
//! list that would look like a bug.

use crate::mcp::{Tool, ToolResult, ToolSchema};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

/// Canonical tool name. Kept in a constant so the runner's intercept
/// branch and the schema definition stay in sync — a typo here would
/// silently downgrade discovery mode into "model calls the placeholder
/// and gets nothing useful back".
pub const TOOLS_LIST_NAME: &str = "tools.list";

#[derive(Debug, Default)]
pub struct ToolsList;

#[async_trait]
impl Tool for ToolsList {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: TOOLS_LIST_NAME.into(),
            description: "List every tool currently enabled for this chat (built-in + MCP \
                 servers). Returns one entry per tool with its `name`, `server_id`, \
                 `server_name`, short `description`, and `destructive` flag. Pass \
                 `name` (optionally as `server_id/tool_name`) to fetch the full \
                 JSON-Schema for a single tool's arguments before calling it; pass \
                 `server_id` alone to restrict the listing to one server. Use this \
                 whenever you're about to tell the user you can't do something — \
                 the catalog you saw up-front may have been collapsed to save \
                 tokens, and the real list is only known once you call this tool."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Optional. Return the full input_schema for this tool only. Match is by `tool_name` or `server_id/tool_name`."
                    },
                    "server_id": {
                        "type": "string",
                        "description": "Optional. Restrict the listing to tools from this MCP server (e.g. `builtin`)."
                    }
                }
            }),
            // Read-only; never gated by the destructive-confirm prompt.
            destructive: false,
        }
    }

    /// Fallback dispatch path. The chat runner intercepts `tools.list`
    /// before we get here so the live catalog is available; this body
    /// only runs when the tool is called via
    /// [`crate::commands::mcp::mcp_call_tool`] (the manual "Test" button
    /// on the Tools page), where there is no chat context. We return a
    /// friendly explanation rather than an empty list that would look
    /// like a bug to the user.
    async fn call(&self, _args: Value) -> Result<ToolResult> {
        Ok(ToolResult {
            content: "tools.list is only meaningful inside a chat turn. The chat \
                      runner injects the live tool catalog at dispatch time so the \
                      listing reflects per-chat enables + slash gates."
                .into(),
            is_error: false,
        })
    }
}

pub fn all() -> Vec<Box<dyn Tool>> {
    vec![Box::new(ToolsList)]
}

/// Tools that must never be disabled by the user (globally or per-chat)
/// and must never appear on the Tools page or the chat header popover.
///
/// Currently just [`TOOLS_LIST_NAME`]: lazy tool-discovery mode falls
/// over if the model can't see the discovery tool, and exposing a toggle
/// for it would let the user break their own chat in a way the UI
/// couldn't explain. The catalog includes it unconditionally and the
/// `mcp_list_builtins` IPC filters it out before returning to the
/// frontend so it stays out of every list the UI renders.
pub fn is_force_enabled(name: &str) -> bool {
    name == TOOLS_LIST_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_advertises_expected_name_and_non_destructive() {
        let s = ToolsList.schema();
        assert_eq!(s.name, TOOLS_LIST_NAME);
        assert!(!s.destructive);
        // The schema must declare its two optional filter args so OpenAI
        // strict-schema clients accept the function definition.
        let props = &s.input_schema["properties"];
        assert!(props.get("name").is_some());
        assert!(props.get("server_id").is_some());
    }

    #[tokio::test]
    async fn placeholder_call_returns_non_empty_explanation() {
        // Direct invocation (e.g. from the Tools-page Test button) must
        // not panic and must surface *some* output so the user understands
        // the empty-looking result isn't a bug.
        let r = ToolsList.call(serde_json::json!({})).await.unwrap();
        assert!(!r.is_error);
        assert!(!r.content.is_empty());
        assert!(r.content.contains("chat"));
    }

    #[test]
    fn is_force_enabled_matches_only_the_discovery_tool() {
        assert!(is_force_enabled(TOOLS_LIST_NAME));
        // Other built-ins must not be force-enabled — users still need
        // the ability to disable shell.exec, fs.write, … globally.
        assert!(!is_force_enabled("fs.list"));
        assert!(!is_force_enabled("shell.exec"));
        assert!(!is_force_enabled(""));
    }
}
