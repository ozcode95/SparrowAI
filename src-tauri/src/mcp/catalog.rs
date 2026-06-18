//! Aggregated MCP tool catalog used by the chat runner.
//!
//! The chat agent loop needs to know *which* tools are currently available
//! across every enabled MCP server so it can (a) describe them in the
//! system prompt and (b) route `tool_use` blocks emitted by the model to
//! the right server. Probing every server on every chat turn is wasteful
//! — most servers expose a stable catalog — so we keep a tiny in-process
//! cache keyed by `(server_id)` with a short TTL.
//!
//! Probe failures are *non-fatal*: a single broken server doesn't take
//! down the whole catalog. The agent just won't see that server's tools
//! that turn.

use crate::llm::ToolDef;
use crate::mcp::tools::discovery::TOOLS_LIST_NAME;
use crate::mcp::{client, ToolSchema, BUILTIN_SERVER_ID, BUILTIN_SERVER_NAME};
use crate::settings::{McpServerConfig, Settings};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::sync::Mutex;

/// One row in the catalog the agent loop sees. Pairs the tool schema with
/// the server it came from so we can dispatch `tools/call` later without
/// re-resolving by name (tool names can collide across servers).
#[derive(Debug, Clone)]
pub struct EnabledTool {
    pub server_id: String,
    pub server_name: String,
    pub schema: ToolSchema,
}

#[derive(Debug, Clone)]
struct Entry {
    fetched_at: Instant,
    tools: Vec<ToolSchema>,
}

/// Process-wide TTL cache. `Arc<Mutex<...>>` because the chat runner is
/// spawned per-turn and may overlap with the Tools page calling
/// `mcp_list_tools` directly; both write through the same lock.
#[derive(Debug, Default)]
pub struct McpToolsCache {
    /// Per-server cache. `None` slot = never probed.
    inner: Mutex<HashMap<String, Entry>>,
}

impl McpToolsCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Drop a single server's cached entry. Called when the user toggles
    /// or edits a server config so the next chat turn re-probes.
    pub async fn invalidate(&self, server_id: &str) {
        self.inner.lock().await.remove(server_id);
    }

    /// Drop every cached entry. Cheap escape hatch for "Settings changed,
    /// reload everything".
    pub async fn clear(&self) {
        self.inner.lock().await.clear();
    }
}

/// TTL for cache entries. Long enough that a chatty multi-turn agent
/// session doesn't re-probe every reply, short enough that the user gets
/// a new tool quickly if they just added one on the Tools page.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Per-server probe timeout. We keep this aggressive because a stuck
/// server would otherwise gate the very first delta the user sees.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Walk every enabled MCP server and return the union of their tool
/// catalogs plus the in-process built-in registry. Servers whose probes
/// fail are logged and skipped — the returned vector only contains
/// tools we can actually call.
///
/// Built-ins always come first so the model sees them at the top of the
/// catalog (slight bias toward calling local file I/O over a remote
/// MCP-equivalent when both are available).
pub async fn fetch_enabled(
    app: &AppHandle,
    http: &reqwest::Client,
    settings: &Settings,
    cache: &McpToolsCache,
) -> Vec<EnabledTool> {
    let mut out = Vec::new();
    for tool in crate::mcp::builtin_registry(app) {
        let schema = tool.schema();
        // Globally-disabled built-ins (toggled off on the Tools page)
        // are dropped here so every chat sees a consistent catalog. The
        // per-conversation override (`disabled_tools` on the chat
        // record) layers on top of this — see the filter further down
        // in `chat::runner`.
        //
        // Force-enabled tools (currently just `tools.list`) bypass the
        // disable filter entirely — they're hidden from the Tools page
        // so the user can't have added them to `builtin_tools_disabled`
        // through the UI, but we still guard against a hand-edited
        // settings file taking the discovery tool offline.
        if !crate::mcp::tools::discovery::is_force_enabled(&schema.name)
            && settings
                .builtin_tools_disabled
                .iter()
                .any(|n| n == &schema.name)
        {
            continue;
        }
        out.push(EnabledTool {
            server_id: BUILTIN_SERVER_ID.into(),
            server_name: BUILTIN_SERVER_NAME.into(),
            schema,
        });
    }
    for srv in settings.mcp_servers.iter().filter(|s| s.enabled) {
        let schemas = match probe(http, srv, cache).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("mcp catalog probe `{}` failed: {e:#}", srv.id);
                continue;
            }
        };
        for schema in schemas {
            out.push(EnabledTool {
                server_id: srv.id.clone(),
                server_name: srv.name.clone(),
                schema,
            });
        }
    }
    out
}

async fn probe(
    http: &reqwest::Client,
    srv: &McpServerConfig,
    cache: &McpToolsCache,
) -> anyhow::Result<Vec<ToolSchema>> {
    // Fast path: cache hit within TTL.
    {
        let guard = cache.inner.lock().await;
        if let Some(entry) = guard.get(&srv.id) {
            if entry.fetched_at.elapsed() < CACHE_TTL {
                return Ok(entry.tools.clone());
            }
        }
    }

    // Slow path: probe with a hard timeout so a hung server can't gate
    // the chat. `client::list_tools` already sets its own per-request
    // timeout, but we add a belt-and-braces upper bound here.
    let tools = match tokio::time::timeout(PROBE_TIMEOUT, client::list_tools(http, srv)).await {
        Ok(res) => res?,
        Err(_) => {
            return Err(anyhow::anyhow!(
                "tools/list timed out after {:?}",
                PROBE_TIMEOUT
            ));
        }
    };

    cache.inner.lock().await.insert(
        srv.id.clone(),
        Entry {
            fetched_at: Instant::now(),
            tools: tools.clone(),
        },
    );
    Ok(tools)
}

/// Pretty-print the catalog as a short markdown section for the system
/// prompt. Empty input → empty string (the runner can append
/// unconditionally).
///
/// We deliberately keep this *brief* now that the runner ships the
/// catalog via the OpenAI `tools` array on the request (which OVMS
/// surfaces to the model through its chat template + configured
/// `tool_parser`). The prompt section is just a human-readable index so
/// the user (and any prompt-debugging logs) can see what was on offer;
/// the model itself learns the schemas from the structured `tools`
/// field, not from this text.
pub fn render_prompt_section(tools: &[EnabledTool]) -> String {
    if tools.is_empty() {
        return String::new();
    }

    let mut by_server: Vec<(&str, &str, Vec<&EnabledTool>)> = Vec::new();
    for t in tools {
        match by_server
            .iter_mut()
            .find(|(id, _, _)| *id == t.server_id.as_str())
        {
            Some((_, _, list)) => list.push(t),
            None => by_server.push((&t.server_id, &t.server_name, vec![t])),
        }
    }

    let mut out = String::new();
    out.push_str("\n\n# Available tools\n");
    out.push_str(
        "Below is the live tool catalogue. Each entry shows a \
         human-readable name followed by a single backticked token \
         labelled `function name:`. To call that tool, put exactly \
         that token in `tool_calls[].function.name` and put the \
         arguments in `tool_calls[].function.arguments` as a JSON \
         string. The token is opaque — use it as-is, character for \
         character.\n",
    );
    for (id, name, list) in by_server {
        out.push_str(&format!("\n## server `{id}` — {name}\n"));
        for t in list {
            let desc = if t.schema.description.is_empty() {
                "(no description)"
            } else {
                t.schema.description.as_str()
            };
            let wire = wire_function_name(&t.server_id, &t.schema.name, tools);
            out.push_str(&format!(
                "- **{}** — function name: `{wire}` — {desc}\n",
                t.schema.name
            ));
            if t.schema.destructive {
                out.push_str("  - ⚠ destructive\n");
            }
        }
    }
    out
}

/// Drop-in replacement for [`render_prompt_section`] used when the runner
/// is in **lazy tool-discovery mode**. Instead of dumping the full
/// catalogue inline (which can run to several KB of system-prompt text
/// once a few MCP servers are wired up) we advertise only the
/// [`tools::discovery`](crate::mcp::tools::discovery) tool and tell the
/// model how to use it to discover the rest on demand. The runner
/// expands the OpenAI `tools` array back to the full catalogue once
/// `tools.list` has been called this turn, so the model can actually
/// dispatch whatever it just discovered.
///
/// Returns the empty string when `tools` is empty or doesn't contain the
/// discovery tool (e.g. the user disabled it on the Tools page) — the
/// runner falls back to [`render_prompt_section`] in that case so the
/// model isn't left with no tool surface at all.
pub fn render_lazy_prompt_section(tools: &[EnabledTool]) -> String {
    if !tools
        .iter()
        .any(|t| t.server_id == BUILTIN_SERVER_ID && t.schema.name == TOOLS_LIST_NAME)
    {
        return String::new();
    }
    let total = tools.len();
    let discovery_wire = wire_function_name(BUILTIN_SERVER_ID, TOOLS_LIST_NAME, tools);
    let mut out = String::new();
    out.push_str("\n\n# Available tools\n");
    out.push_str(&format!(
        "The full catalogue is fetched on demand. {total} tool(s) are \
         currently enabled.\n\n\
         To discover them, emit a tool call whose `function.name` is \
         exactly this token:\n\n\
         ```\n{discovery_wire}\n```\n\n\
         Use that token verbatim, character for character. It is \
         opaque — treat it as a single identifier, not as two parts \
         joined by `__`. The arguments are a JSON object; pass `{{}}` \
         for the full listing, or `{{\"name\": \"<tool>\"}}` to fetch \
         the input schema (and exact call token) for one specific \
         tool before invoking it. The reply from the discovery call \
         shows the exact `function name:` token for every other tool \
         — reuse those tokens the same way when you call them. If \
         after listing nothing fits, just answer the user directly.\n",
    ));
    out
}

/// Render the live tool catalogue as a `tools.list` result payload. The
/// chat runner intercepts dispatch for the `tools.list` built-in and
/// calls this directly so the listing reflects per-turn filtering
/// (per-chat disables, web slash-gates, lazy-mode initial collapse, …)
/// instead of the static [`crate::mcp::builtin_registry`] view.
///
/// Behaviour:
///
/// * No filters → markdown listing grouped by server, one bullet per
///   tool. Mirrors [`render_prompt_section`] so users debugging the
///   model's view see a consistent format.
/// * `name` filter (optionally `server_id/tool_name`) → pretty JSON for
///   the matching tool, including the full `input_schema` so the model
///   knows how to construct arguments before calling it.
/// * `server_id` filter alone → markdown listing limited to that server.
/// * No matches → short error-shaped message so the model can recover
///   without us having to flag `is_error`.
pub fn render_tools_list_result(
    tools: &[EnabledTool],
    name_filter: Option<&str>,
    server_filter: Option<&str>,
) -> String {
    // Split a `server_id/tool_name` shorthand into its parts so the
    // caller can address an exact tool without having to also pass
    // `server_id`. Falls back to the bare name when no slash is present.
    let (name_server_hint, bare_name) = match name_filter {
        Some(n) => match n.split_once('/') {
            Some((srv, tool)) => (Some(srv), Some(tool)),
            None => (None, Some(n)),
        },
        None => (None, None),
    };

    let filtered: Vec<&EnabledTool> = tools
        .iter()
        .filter(|t| match server_filter {
            Some(s) => t.server_id == s,
            None => true,
        })
        .filter(|t| match name_server_hint {
            Some(s) => t.server_id == s,
            None => true,
        })
        .filter(|t| match bare_name {
            Some(n) => t.schema.name == n,
            None => true,
        })
        // Hide the discovery built-in from its own output. Listing
        // `tools.list` in the catalogue (or letting the model fetch
        // its own schema by name) is pure noise — the model already
        // knows how to call it because it was given the definition
        // up front.
        .filter(|t| {
            !(t.server_id == BUILTIN_SERVER_ID && t.schema.name == TOOLS_LIST_NAME)
        })
        .collect();

    if filtered.is_empty() {
        return match (name_filter, server_filter) {
            (Some(n), _) => format!("No enabled tool matches `{n}`."),
            (None, Some(s)) => format!("No enabled tools on server `{s}`."),
            _ => "No tools are currently enabled for this chat.".to_string(),
        };
    }

    // Detail mode: a specific tool was requested by name and resolved
    // to exactly one entry — emit the full JSON schema so the model can
    // construct arguments without a second round-trip.
    if name_filter.is_some() && filtered.len() == 1 {
        let t = filtered[0];
        let payload = serde_json::json!({
            "name": t.schema.name,
            // Exact token to put in `tool_calls[].function.name`. We
            // surface it explicitly so the model never has to derive
            // it from `server_id` + `name` (small models routinely
            // double-prefix when asked to reconstruct).
            "function_name": wire_function_name(&t.server_id, &t.schema.name, tools),
            "server_id": t.server_id,
            "server_name": t.server_name,
            "description": t.schema.description,
            "destructive": t.schema.destructive,
            "input_schema": t.schema.input_schema,
        });
        // Pretty-print; the result is shown to the model verbatim so
        // extra whitespace barely matters but it makes the wire log
        // far easier to skim when debugging.
        return serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    }

    // Listing mode: markdown grouped by server. Same shape as
    // `render_prompt_section` so a user comparing the prompt to the
    // tools.list output doesn't have to translate formats.
    let mut by_server: Vec<(&str, &str, Vec<&EnabledTool>)> = Vec::new();
    for t in filtered {
        match by_server
            .iter_mut()
            .find(|(id, _, _)| *id == t.server_id.as_str())
        {
            Some((_, _, list)) => list.push(t),
            None => by_server.push((&t.server_id, &t.server_name, vec![t])),
        }
    }

    let mut out = String::new();
    for (id, name, list) in by_server {
        out.push_str(&format!("## server `{id}` — {name}\n"));
        for t in list {
            let desc = if t.schema.description.is_empty() {
                "(no description)"
            } else {
                t.schema.description.as_str()
            };
            let wire = wire_function_name(&t.server_id, &t.schema.name, tools);
            out.push_str(&format!(
                "- **{}** — function name: `{wire}` — {desc}\n",
                t.schema.name
            ));
            if t.schema.destructive {
                out.push_str("  - ⚠ destructive\n");
            }
        }
        out.push('\n');
    }
    let discovery_wire = wire_function_name(BUILTIN_SERVER_ID, TOOLS_LIST_NAME, tools);
    out.push_str(&format!(
        "Each tool above lists a `function name:` token. To invoke a \
         tool, put that token in `tool_calls[].function.name` exactly \
         as written — use the token as a single opaque identifier. To \
         fetch the full `input_schema` for a single tool, call \
         `{discovery_wire}` again with `{{\"name\": \"<tool_name>\"}}` \
         (or `\"server_id/tool_name\"` to disambiguate). The reply \
         includes a `function_name` field with the exact token to \
         pass on the next call."
    ));
    out
}

/// Pick out the entries the model should see when lazy tool discovery is
/// active and `tools.list` has not yet been called this turn. Returns
/// just the discovery placeholder (the only tool we want the model to
/// reach for first) so the OpenAI `tools` array on the initial request
/// stays tiny.
///
/// If the user has explicitly disabled `tools.list` (toggled it off on
/// the Tools page or for this specific chat) we return an empty slice;
/// the runner falls back to the full catalogue in that case so the
/// model isn't left with literally no tools.
pub fn collapse_to_discovery(tools: &[EnabledTool]) -> Vec<EnabledTool> {
    tools
        .iter()
        .filter(|t| t.server_id == BUILTIN_SERVER_ID && t.schema.name == TOOLS_LIST_NAME)
        .cloned()
        .collect()
}

/// Convert the enabled catalog into the OpenAI `tools` array shape that
/// rides on `ChatRequest::tools`. Function names are produced by
/// [`wire_function_name`]: tools whose sanitised name is globally
/// unique across every enabled server get the bare sanitised form
/// (e.g. `tools_list`, `fs_list`); only colliding names get the
/// full `<server>__<tool>` encoding for disambiguation. This
/// sidesteps a Gemma 4 pathology where the model, shown a function
/// named `builtin__tools_list`, re-prepends `builtin__` when emitting
/// the call and produces `builtin__builtin__tools_list`. The resolver
/// accepts both shapes either way.
pub fn to_tool_defs(tools: &[EnabledTool]) -> Vec<ToolDef> {
    tools
        .iter()
        .map(|t| {
            ToolDef::function(
                wire_function_name(&t.server_id, &t.schema.name, tools),
                t.schema.description.clone(),
                tool_parameters_schema(&t.schema),
            )
        })
        .collect()
}

/// Pick the wire-format function name shown to the model for a
/// catalogue entry. Returns the bare sanitised tool name when no
/// other enabled tool would sanitise to the same identifier; falls
/// back to the fully-qualified `<server>__<tool>` encoding from
/// [`function_name_for`] only when disambiguation is actually needed.
///
/// Centralising the decision here keeps three call sites (`to_tool_defs`,
/// `render_prompt_section`, `render_tools_list_result`) presenting the
/// model a single consistent token for each tool. The resolver in
/// [`resolve_function_name`] handles both shapes transparently, so
/// changing the strategy is safe end-to-end.
pub fn wire_function_name(server_id: &str, tool_name: &str, all_tools: &[EnabledTool]) -> String {
    let bare = sanitize_segment(tool_name);
    let collisions = all_tools
        .iter()
        .filter(|t| sanitize_segment(&t.schema.name) == bare)
        .count();
    if collisions <= 1 {
        // Globally unique — ship the bare name and let the resolver's
        // bare-name fallback route the call back. Truncate to the
        // 64-char OpenAI cap defensively even though most tool names
        // are short.
        let mut name = bare;
        if name.len() > 64 {
            name.truncate(64);
        }
        name
    } else {
        // Two or more enabled tools share this sanitised name (e.g.
        // `search` on a built-in plus a third-party MCP server). Use
        // the fully-qualified form so the resolver's strict path can
        // pick the right one.
        function_name_for(server_id, tool_name)
    }
}

/// Strict OpenAI function names must match `^[a-zA-Z0-9_-]{1,64}$`, but
/// our server ids and tool names routinely contain `.` (e.g. `fs.list`).
/// We flatten with `__` as the separator and replace anything outside
/// the allowed alphabet with `_`. Round-trip safe via
/// [`split_function_name`] because both halves are sanitised before the
/// `__` is inserted.
pub fn function_name_for(server_id: &str, tool_name: &str) -> String {
    let lhs = sanitize_segment(server_id);
    let rhs = sanitize_segment(tool_name);
    let mut combined = format!("{lhs}__{rhs}");
    // 64-char hard cap from the OpenAI spec. Truncating the right-hand
    // side keeps the server prefix intact so collisions stay rare even
    // for very long upstream tool names.
    if combined.len() > 64 {
        combined.truncate(64);
    }
    combined
}

fn sanitize_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Best-effort reverse of [`function_name_for`]: split a flattened
/// `server__tool` name back into the sanitised `(server_segment,
/// tool_segment)` pair. Splits on the **first** `__` so tool names that
/// themselves contain `__` (rare, but possible after sanitisation) stay
/// intact in the right-hand side.
pub fn split_function_name(name: &str) -> Option<(&str, &str)> {
    name.find("__").map(|idx| (&name[..idx], &name[idx + 2..]))
}

/// Resolve a model-emitted function name back to the live catalog entry.
/// Tries the deterministic `server__tool` split first; if that fails
/// (small models occasionally ignore the prefix and emit just the tool
/// name, or invent a plausible-looking server like `local__` /
/// `mcp__` / `system__`), falls back to matching the right-hand
/// `tool` segment alone, then finally the bare function name across
/// all enabled servers.
///
/// Each segment is matched against both the **sanitised** form (e.g.
/// `fs_list`) and the **original** form (e.g. `fs.list`) because small
/// local models routinely echo the human-readable schema name into
/// `tool_calls.function.name` instead of the wire-encoded one we shipped
/// in the OpenAI `tools` array. Accepting both keeps dispatch working
/// when a model emits `builtin__fs.list` for what we advertised as
/// `builtin__fs_list`, or `local__tools_list` for `builtin__tools_list`.
pub fn resolve_function_name<'a>(
    tools: &'a [EnabledTool],
    func_name: &str,
) -> Option<&'a EnabledTool> {
    if let Some((srv_seg, tool_seg)) = split_function_name(func_name) {
        // Strict path: both the server prefix and the tool segment
        // resolve. This is what well-behaved models emit verbatim from
        // the wire-encoded `to_tool_defs` output.
        if let Some(t) = tools.iter().find(|t| {
            (sanitize_segment(&t.server_id) == srv_seg || t.server_id == srv_seg)
                && (sanitize_segment(&t.schema.name) == tool_seg || t.schema.name == tool_seg)
        }) {
            return Some(t);
        }
        // Forgiving path: the tool segment alone is enough to pick a
        // unique tool. Triggered when the model invents a server
        // prefix (e.g. `local__tools_list`, `mcp__fs_list`) that
        // doesn't correspond to any enabled server. We log so the
        // mismatch is visible in diagnostics, then dispatch through
        // the right tool anyway so the user's turn isn't wasted.
        if let Some(t) = tools
            .iter()
            .find(|t| sanitize_segment(&t.schema.name) == tool_seg || t.schema.name == tool_seg)
        {
            tracing::debug!(
                "tool resolver: model emitted unknown server prefix `{srv_seg}__` \
                 for tool `{tool_seg}` — routing to `{}/{}`",
                t.server_id,
                t.schema.name
            );
            return Some(t);
        }
        // Duplicated-prefix recovery. Small models (Gemma 4 in
        // particular) routinely re-encode the already-wire-encoded
        // function name a second time, emitting things like
        // `builtin__builtin__tools_list` or even three-deep nestings.
        // Once the strict + forgiving paths above have failed, recurse
        // on `tool_seg` so each round peels one duplicated prefix off
        // the front. The recursion terminates naturally because
        // `split_function_name` only fires while a `__` separator is
        // present, and the bare-name fallback below handles the leaf.
        if tool_seg.contains("__") {
            if let Some(t) = resolve_function_name(tools, tool_seg) {
                tracing::debug!(
                    "tool resolver: model emitted duplicated prefix in `{func_name}` \
                     — recovered via `{tool_seg}` → `{}/{}`",
                    t.server_id,
                    t.schema.name
                );
                return Some(t);
            }
        }
    }
    tools
        .iter()
        .find(|t| sanitize_segment(&t.schema.name) == func_name || t.schema.name == func_name)
}

/// Normalise a tool's `input_schema` into something OVMS / strict OpenAI
/// clones accept as `function.parameters`. The spec wants a JSON-schema
/// object (`{"type": "object", "properties": {...}}`); tools that ship
/// an empty schema or a non-object schema get a permissive
/// `{"type": "object"}` substituted in so the request never fails
/// validation upstream.
fn tool_parameters_schema(schema: &ToolSchema) -> serde_json::Value {
    use serde_json::Value;
    match &schema.input_schema {
        Value::Object(map) if !map.is_empty() => Value::Object(map.clone()),
        _ => serde_json::json!({ "type": "object" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(server: &str, name: &str, desc: &str, destructive: bool) -> EnabledTool {
        EnabledTool {
            server_id: server.to_string(),
            server_name: format!("{server}-srv"),
            schema: ToolSchema {
                name: name.to_string(),
                description: desc.to_string(),
                input_schema: json!({ "type": "object" }),
                destructive,
            },
        }
    }

    #[test]
    fn render_empty_returns_empty_string() {
        assert!(render_prompt_section(&[]).is_empty());
    }

    #[test]
    fn render_groups_by_server_and_marks_destructive() {
        let tools = vec![
            tool("alpha", "search", "search the web", false),
            tool("alpha", "fetch", "fetch a url", false),
            tool("beta", "shell.exec", "run a shell command", true),
        ];
        let out = render_prompt_section(&tools);
        assert!(out.contains("# Available tools"));
        // The runner now ships tools via the OpenAI `tools` field, so
        // the prompt section must NOT instruct fenced-JSON protocol.
        assert!(
            !out.contains("```tool_use"),
            "prompt section should no longer teach the legacy fenced-JSON protocol"
        );
        assert!(out.contains("## server `alpha`"));
        assert!(out.contains("## server `beta`"));
        assert!(out.contains("**search**"));
        assert!(out.contains("**shell.exec**"));
        assert!(out.contains("destructive"));
        // alpha section should list both alpha tools before the beta section.
        let alpha_idx = out.find("## server `alpha`").unwrap();
        let beta_idx = out.find("## server `beta`").unwrap();
        let search_idx = out.find("**search**").unwrap();
        let fetch_idx = out.find("**fetch**").unwrap();
        assert!(alpha_idx < search_idx && search_idx < beta_idx);
        assert!(alpha_idx < fetch_idx && fetch_idx < beta_idx);
    }

    #[test]
    fn function_name_for_sanitises_dots_and_flattens_with_double_underscore() {
        assert_eq!(function_name_for("builtin", "fs.list"), "builtin__fs_list");
        assert_eq!(function_name_for("builtin", "fs.read"), "builtin__fs_read");
        // Already-clean inputs are passed through unchanged.
        assert_eq!(function_name_for("alpha", "search"), "alpha__search");
        // Slashes / colons / spaces all collapse to `_`.
        assert_eq!(
            function_name_for("http://srv", "do thing"),
            "http___srv__do_thing"
        );
    }

    #[test]
    fn function_name_for_caps_at_64_chars() {
        let long_tool = "x".repeat(80);
        let name = function_name_for("srv", &long_tool);
        assert_eq!(name.len(), 64);
        assert!(
            name.starts_with("srv__"),
            "server prefix should survive truncation: {name}"
        );
    }

    #[test]
    fn resolve_function_name_routes_prefixed_calls_back_to_the_right_server() {
        let tools = vec![
            tool("builtin", "fs.list", "", false),
            tool("builtin", "fs.read", "", false),
            tool("alpha", "search", "", false),
        ];
        let got = resolve_function_name(&tools, "builtin__fs_list").unwrap();
        assert_eq!(got.server_id, "builtin");
        assert_eq!(got.schema.name, "fs.list");

        // Bare-name fallback for sloppy models that drop the prefix.
        let got = resolve_function_name(&tools, "search").unwrap();
        assert_eq!(got.server_id, "alpha");

        // Unknown name resolves to None.
        assert!(resolve_function_name(&tools, "does_not_exist").is_none());
    }

    #[test]
    fn resolve_function_name_accepts_unsanitised_tool_segment_emitted_by_the_model() {
        // Small local models routinely echo the human-readable schema
        // name (`fs.list`) into the wire-format `tool_calls.function.name`
        // instead of the sanitised form (`fs_list`) we shipped. The
        // resolver must accept both so dispatch doesn't fail with
        // "no enabled tool matches" purely because of a punctuation slip.
        let tools = vec![tool("builtin", "fs.list", "", false)];

        // Server prefix + dotted tool name (the failure mode observed in
        // gemma4 with lazy tool discovery).
        let got = resolve_function_name(&tools, "builtin__fs.list").unwrap();
        assert_eq!(got.schema.name, "fs.list");

        // Bare dotted tool name (no `server__` prefix at all).
        let got = resolve_function_name(&tools, "fs.list").unwrap();
        assert_eq!(got.schema.name, "fs.list");
    }

    #[test]
    fn resolve_function_name_recovers_from_hallucinated_server_prefix() {
        // Failure mode observed with smaller text-gen models on lazy
        // tool discovery: the model invents a plausible-looking server
        // prefix (`local__`, `mcp__`, `system__`) that doesn't match
        // any enabled server. The resolver should still route the call
        // by the unambiguous tool segment instead of returning None
        // and stalling the agent loop with `[no enabled tool matches …]`.
        let tools = vec![
            tool("builtin", "tools.list", "", false),
            tool("builtin", "fs.list", "", false),
            tool("alpha", "search", "", false),
        ];

        let got = resolve_function_name(&tools, "local__tools_list").unwrap();
        assert_eq!(got.server_id, "builtin");
        assert_eq!(got.schema.name, "tools.list");

        let got = resolve_function_name(&tools, "mcp__search").unwrap();
        assert_eq!(got.server_id, "alpha");
        assert_eq!(got.schema.name, "search");

        // A wrong server prefix paired with a tool name that doesn't
        // exist anywhere must still fail loudly — the recovery path is
        // strictly a fallback for the prefix, not the tool segment.
        assert!(resolve_function_name(&tools, "local__does_not_exist").is_none());
    }

    #[test]
    fn resolve_function_name_recovers_from_duplicated_server_prefix() {
        // Failure mode observed with Gemma 4 (E4B-it-int8) on lazy
        // tool discovery: the model reads the already-wire-encoded
        // function name `builtin__tools_list` out of the OpenAI
        // `tools` array, then *re-encodes* it a second time before
        // emitting the call, producing `builtin__builtin__tools_list`.
        // Three-deep variants (`builtin__builtin__builtin__tools_list`)
        // surface too. The resolver must peel the duplicates off
        // recursively and still route to the real tool, otherwise the
        // model just retries the same broken call until the iteration
        // cap runs out.
        let tools = vec![
            tool("builtin", "tools.list", "", false),
            tool("builtin", "fs.list", "", false),
            tool("alpha", "search", "", false),
        ];

        // Two-deep duplicated prefix on a built-in tool.
        let got = resolve_function_name(&tools, "builtin__builtin__tools_list").unwrap();
        assert_eq!(got.server_id, "builtin");
        assert_eq!(got.schema.name, "tools.list");

        // Three-deep nesting still recovers.
        let got =
            resolve_function_name(&tools, "builtin__builtin__builtin__tools_list").unwrap();
        assert_eq!(got.schema.name, "tools.list");

        // Duplicated prefix on a non-built-in MCP server resolves the
        // same way (the recovery path is server-agnostic).
        let got = resolve_function_name(&tools, "alpha__alpha__search").unwrap();
        assert_eq!(got.server_id, "alpha");
        assert_eq!(got.schema.name, "search");

        // Mixed hallucinated + duplicated prefix: the model invents
        // a fake outer prefix on top of an already-encoded name. The
        // hallucinated-prefix path runs first, so the inner encoded
        // name resolves cleanly.
        let got = resolve_function_name(&tools, "local__builtin__tools_list").unwrap();
        assert_eq!(got.schema.name, "tools.list");
    }

    #[test]
    fn to_tool_defs_emits_openai_function_shape_with_object_parameters() {
        let mut t = tool("builtin", "fs.list", "List a directory.", false);
        t.schema.input_schema = json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        });
        let defs = to_tool_defs(&[t]);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, "function");
        // Single tool in the catalogue → globally unique sanitised
        // name → ships without the `builtin__` server prefix so small
        // models can't double-prefix it.
        assert_eq!(defs[0].function.name, "fs_list");
        assert_eq!(defs[0].function.description, "List a directory.");
        assert_eq!(defs[0].function.parameters["type"], "object");
        assert_eq!(defs[0].function.parameters["required"], json!(["path"]));
    }

    #[test]
    fn to_tool_defs_prefixes_only_colliding_tool_names() {
        // Two enabled servers happen to expose a tool that sanitises
        // to the same name (`search`) — both entries get the full
        // `<server>__<tool>` encoding so the resolver's strict path
        // can disambiguate. A unique sibling (`fs.list`) still ships
        // bare.
        let tools = vec![
            tool("builtin", "fs.list", "", false),
            tool("builtin", "search", "", false),
            tool("alpha", "search", "", false),
        ];
        let defs = to_tool_defs(&tools);
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert_eq!(names, vec!["fs_list", "builtin__search", "alpha__search"]);
    }

    #[test]
    fn to_tool_defs_substitutes_permissive_schema_when_input_schema_is_missing() {
        let mut t = tool("alpha", "ping", "", false);
        // Non-object schema — must not be passed through verbatim.
        t.schema.input_schema = json!(null);
        let defs = to_tool_defs(&[t]);
        assert_eq!(defs[0].function.parameters, json!({ "type": "object" }));
    }

    #[test]
    fn collapse_to_discovery_keeps_only_the_discovery_placeholder() {
        let tools = vec![
            tool("builtin", "fs.list", "", false),
            tool("builtin", TOOLS_LIST_NAME, "", false),
            tool("alpha", "search", "", false),
        ];
        let got = collapse_to_discovery(&tools);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].schema.name, TOOLS_LIST_NAME);
        assert_eq!(got[0].server_id, BUILTIN_SERVER_ID);
    }

    #[test]
    fn collapse_to_discovery_returns_empty_when_user_disabled_it() {
        let tools = vec![
            tool("builtin", "fs.list", "", false),
            tool("alpha", "search", "", false),
        ];
        // No tools.list → caller (the runner) should fall back to the
        // full catalogue rather than ship an empty `tools` array.
        assert!(collapse_to_discovery(&tools).is_empty());
    }

    #[test]
    fn render_lazy_prompt_section_advertises_discovery_with_total_count() {
        let tools = vec![
            tool("builtin", TOOLS_LIST_NAME, "", false),
            tool("builtin", "fs.list", "", false),
            tool("alpha", "search", "", false),
        ];
        let out = render_lazy_prompt_section(&tools);
        assert!(out.contains("# Available tools"));
        // Prompt advertises the discovery tool's wire name verbatim.
        // Now that names ship bare when globally unique, the discovery
        // token is `tools_list` (no `builtin__` prefix) so small
        // models have nothing to re-prepend and accidentally
        // double-prefix.
        assert!(out.contains("tools_list"));
        assert!(!out.contains("builtin__tools_list"));
        assert!(!out.contains("tools.list"));
        assert!(out.contains("3 tool(s)"));
        // The full per-tool names must NOT leak — the whole point of
        // lazy mode is to keep them out of turn-1 context.
        assert!(!out.contains("fs.list"));
        assert!(!out.contains("alpha"));
    }

    #[test]
    fn render_lazy_prompt_section_returns_empty_when_discovery_tool_is_absent() {
        let tools = vec![tool("alpha", "search", "", false)];
        // Discovery tool was disabled — caller should fall back to the
        // full prompt section.
        assert!(render_lazy_prompt_section(&tools).is_empty());
    }

    #[test]
    fn render_tools_list_result_lists_every_tool_grouped_by_server_when_no_filter() {
        let tools = vec![
            tool("builtin", "fs.list", "List a dir", false),
            tool("builtin", "shell.exec", "Run a cmd", true),
            tool("alpha", "search", "Search", false),
        ];
        let out = render_tools_list_result(&tools, None, None);
        assert!(out.contains("server `builtin`"));
        assert!(out.contains("server `alpha`"));
        assert!(out.contains("**fs.list**"));
        assert!(out.contains("**shell.exec**"));
        assert!(out.contains("destructive"));
        assert!(out.contains("**search**"));
    }

    #[test]
    fn render_tools_list_result_returns_full_schema_when_filtered_by_name() {
        let mut t = tool("builtin", "fs.read", "Read a file", false);
        t.schema.input_schema = json!({
            "type": "object",
            "required": ["path"],
            "properties": {"path": {"type": "string"}}
        });
        let tools = vec![t, tool("alpha", "search", "", false)];
        let out = render_tools_list_result(&tools, Some("fs.read"), None);
        let v: serde_json::Value = serde_json::from_str(&out).expect("detail mode must be JSON");
        assert_eq!(v["name"], "fs.read");
        assert_eq!(v["server_id"], "builtin");
        assert_eq!(v["input_schema"]["required"], json!(["path"]));
    }

    #[test]
    fn render_tools_list_result_accepts_server_id_slash_tool_name_shorthand() {
        // Two servers expose the same tool name; the slash form must
        // pick the right one rather than colliding.
        let mut a = tool("alpha", "search", "alpha search", false);
        a.schema.input_schema = json!({"type": "object", "properties": {"q": {"type": "string"}}});
        let mut b = tool("beta", "search", "beta search", false);
        b.schema.input_schema =
            json!({"type": "object", "properties": {"query": {"type": "string"}}});
        let tools = vec![a, b];
        let out = render_tools_list_result(&tools, Some("beta/search"), None);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["server_id"], "beta");
        assert_eq!(v["description"], "beta search");
    }

    #[test]
    fn render_tools_list_result_returns_no_match_message_for_unknown_filter() {
        let tools = vec![tool("builtin", "fs.list", "", false)];
        let out = render_tools_list_result(&tools, Some("does.not.exist"), None);
        assert!(out.to_lowercase().contains("no enabled tool"));
        assert!(out.contains("does.not.exist"));
    }

    #[test]
    fn render_tools_list_result_handles_empty_catalogue() {
        let out = render_tools_list_result(&[], None, None);
        assert!(out.to_lowercase().contains("no tools"));
    }
}
