//! Tauri commands for the MCP (external + built-in) tool surface.

use crate::error::IpcResult;
use crate::mcp::{self, client, ToolSchema};
use crate::settings::{McpServerConfig, Settings};
use crate::state::AppStateExt;
use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;

/// One row in the Tools page server list — config + live status.
#[derive(Debug, Clone, Serialize)]
pub struct McpServerSummary {
    #[serde(flatten)]
    pub config: McpServerConfig,
    /// `null` until the first probe attempt, then `true` (catalog
    /// returned) or `false` (probe failed). Persisted in-memory only.
    pub reachable: Option<bool>,
    /// Tool count from the most recent successful probe, or `0`.
    pub tool_count: u32,
    /// Last probe error message — surfaced verbatim so config typos
    /// (`/v3/mcp` vs `/mcp`, missing API key, ...) are obvious.
    pub last_error: Option<String>,
}

/// List configured MCP servers. We deliberately do *not* probe here — the
/// Tools page calls `mcp_list_tools(server_id)` lazily so opening the page
/// stays instant even when a configured server is down.
#[tauri::command]
pub async fn mcp_list_servers() -> IpcResult<Vec<McpServerSummary>> {
    let s = Settings::load().await.map_err(|e| e.to_string())?;
    Ok(s.mcp_servers
        .into_iter()
        .map(|c| McpServerSummary {
            config: c,
            reachable: None,
            tool_count: 0,
            last_error: None,
        })
        .collect())
}

/// Add or update a single MCP server config (matched by `id`).
#[tauri::command]
pub async fn mcp_upsert_server(app: AppHandle, server: McpServerConfig) -> IpcResult<()> {
    if server.id.trim().is_empty() {
        return Err("server id cannot be empty".into());
    }
    let mut s = Settings::load().await.map_err(|e| e.to_string())?;
    let server_id = server.id.clone();
    if let Some(existing) = s.mcp_servers.iter_mut().find(|m| m.id == server.id) {
        *existing = server;
    } else {
        s.mcp_servers.push(server);
    }
    s.save().await.map_err(|e| e.to_string())?;
    app.zero().mcp_cache.invalidate(&server_id).await;
    Ok(())
}

#[tauri::command]
pub async fn mcp_delete_server(app: AppHandle, id: String) -> IpcResult<()> {
    let mut s = Settings::load().await.map_err(|e| e.to_string())?;
    s.mcp_servers.retain(|m| m.id != id);
    s.save().await.map_err(|e| e.to_string())?;
    app.zero().mcp_cache.invalidate(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn mcp_set_enabled(app: AppHandle, id: String, enabled: bool) -> IpcResult<()> {
    let mut s = Settings::load().await.map_err(|e| e.to_string())?;
    let Some(srv) = s.mcp_servers.iter_mut().find(|m| m.id == id) else {
        return Err(format!("no MCP server `{id}`").into());
    };
    srv.enabled = enabled;
    s.save().await.map_err(|e| e.to_string())?;
    app.zero().mcp_cache.invalidate(&id).await;
    Ok(())
}

/// Probe a single server with `tools/list`. Returns the discovered tool
/// catalog or a hint about what's broken.
#[tauri::command]
pub async fn mcp_list_tools(app: AppHandle, server_id: String) -> IpcResult<Vec<ToolSchema>> {
    let s = Settings::load().await.map_err(|e| e.to_string())?;
    let cfg = s
        .mcp_servers
        .into_iter()
        .find(|m| m.id == server_id)
        .ok_or_else(|| format!("no MCP server `{server_id}`"))?;
    let http = app.zero().http.clone();
    client::list_tools(&http, &cfg)
        .await
        .map_err(|e| e.to_string().into())
}

/// Invoke a tool on the named server. Surface the raw text response;
/// the UI renders it in the tools detail panel.
#[tauri::command]
pub async fn mcp_call_tool(
    app: AppHandle,
    server_id: String,
    name: String,
    arguments: Value,
) -> IpcResult<mcp::ToolResult> {
    let s = Settings::load().await.map_err(|e| e.to_string())?;
    let cfg = s
        .mcp_servers
        .into_iter()
        .find(|m| m.id == server_id)
        .ok_or_else(|| format!("no MCP server `{server_id}`"))?;
    let http = app.zero().http.clone();
    client::call_tool(&http, &cfg, &name, arguments)
        .await
        .map_err(|e| e.to_string().into())
}

/// Return the built-in tool schemas the user is allowed to see and
/// toggle on the Tools page / per-chat Tools popover. Slash-gated
/// tools (currently the `web.*` family) are deliberately filtered out
/// here — they're hidden from every UI surface and only become
/// available to the model when the user opts in for that turn with the
/// matching slash command. The chat runner still constructs them via
/// [`crate::mcp::builtin_registry`] and unlocks them per-turn through
/// [`crate::mcp::tools::web::WebUnlocks`].
///
/// Force-enabled tools (currently `tools.list` for lazy discovery) are
/// also hidden so the user can't toggle off something the runner needs
/// to keep working — see
/// [`crate::mcp::tools::discovery::is_force_enabled`].
#[tauri::command]
pub async fn mcp_list_builtins(app: AppHandle) -> IpcResult<Vec<ToolSchema>> {
    Ok(mcp::builtin_registry(&app)
        .into_iter()
        .map(|t| t.schema())
        .filter(|s| !mcp::tools::web::is_slash_gated(&s.name))
        .filter(|s| !mcp::tools::discovery::is_force_enabled(&s.name))
        .collect())
}
