//! Minimal JSON-RPC 2.0 client for external MCP servers.
//!
//! Implements just enough of the [Model Context Protocol][1] to:
//!
//! - probe a server's tool catalog (`tools/list`)
//! - invoke a single tool (`tools/call`)
//!
//! Two transports are supported:
//!
//! - **HTTP / SSE** — a one-shot POST per RPC against `cfg.url`. The
//!   streamable-HTTP transport answers either `application/json` or a
//!   `text/event-stream` body whose first `data:` frame carries the
//!   JSON-RPC envelope; we handle both.
//! - **stdio**       — we spawn `cfg.command` with `cfg.args` (merged
//!   env on top of the zero process env), perform the MCP handshake
//!   (`initialize` → `notifications/initialized`), issue the request,
//!   read the response, then drop the child so it exits. This is a
//!   one-shot session per RPC — wasteful, but keeps state out of
//!   `AppState` and avoids a stateful per-server actor in the MVP.
//!
//! [1]: https://modelcontextprotocol.io

use crate::mcp::{ToolResult, ToolSchema};
use crate::settings::McpServerConfig;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

static REQ_ID: AtomicU64 = AtomicU64::new(1);

/// MCP protocol version we advertise during stdio `initialize`. Servers
/// usually accept any reasonable date string here.
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Wall-clock cap on a single stdio RPC (spawn + initialize + request +
/// read + drain). Keeps a wedged server from gating the chat.
const STDIO_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Serialize)]
struct Request<'a> {
    jsonrpc: &'a str,
    id: u64,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

/// Dispatch one JSON-RPC call to the appropriate transport. Returns the
/// `result` field (errors are translated into `Err`).
async fn rpc(
    http: &reqwest::Client,
    cfg: &McpServerConfig,
    method: &str,
    params: Option<Value>,
) -> Result<Value> {
    match cfg.transport.as_str() {
        "stdio" => rpc_stdio(cfg, method, params).await,
        // Both http and sse use the same one-shot POST shape; the
        // response content-type discriminates how we parse the body.
        _ => rpc_http(http, cfg, method, params).await,
    }
}

/// HTTP / SSE transport.
async fn rpc_http(
    http: &reqwest::Client,
    cfg: &McpServerConfig,
    method: &str,
    params: Option<Value>,
) -> Result<Value> {
    let id = REQ_ID.fetch_add(1, Ordering::Relaxed);
    let req = Request {
        jsonrpc: "2.0",
        id,
        method,
        params,
    };

    let mut builder = http
        .post(&cfg.url)
        .timeout(Duration::from_secs(15))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    for (k, v) in &cfg.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    let resp = builder
        .json(&req)
        .send()
        .await
        .with_context(|| format!("POST {}", cfg.url))?;

    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("MCP HTTP {status}: {body}"));
    }

    // Strict streamable-HTTP transports answer JSON-RPC over SSE: response
    // is a `text/event-stream` payload whose `data:` lines carry the
    // JSON-RPC envelope. We accept both shapes so the same client works
    // against plain-JSON and SSE servers without configuration.
    let envelope: Response = if ct.contains("text/event-stream") {
        let json = extract_first_sse_data(&body)
            .ok_or_else(|| anyhow!("MCP SSE response had no `data:` frame: {body}"))?;
        serde_json::from_str(&json).with_context(|| format!("decode MCP SSE frame: {json}"))?
    } else {
        serde_json::from_str(&body).with_context(|| format!("decode MCP response: {body}"))?
    };

    unwrap_envelope(envelope)
}

/// stdio transport. One spawn per RPC — we run the standard `initialize`
/// handshake, send the real request, read the response, then drop the
/// child (which closes its stdin and triggers a clean exit on any
/// well-behaved MCP server).
async fn rpc_stdio(cfg: &McpServerConfig, method: &str, params: Option<Value>) -> Result<Value> {
    if cfg.command.trim().is_empty() {
        bail!("stdio MCP server `{}` has no `command` configured", cfg.id);
    }

    let inner = async {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn stdio MCP `{} {:?}`", cfg.command, cfg.args))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("stdio MCP `{}` exposed no stdin", cfg.id))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("stdio MCP `{}` exposed no stdout", cfg.id))?;
        let mut reader = BufReader::new(stdout).lines();

        // 1. initialize
        let init_id = REQ_ID.fetch_add(1, Ordering::Relaxed);
        let init = json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "clientInfo": { "name": "zero", "version": env!("CARGO_PKG_VERSION") },
            },
        });
        write_frame(&mut stdin, &init).await?;
        let _ = read_response_for(&mut reader, init_id).await?;

        // 2. notifications/initialized (no id, no response expected)
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        write_frame(&mut stdin, &notif).await?;

        // 3. the real request
        let rpc_id = REQ_ID.fetch_add(1, Ordering::Relaxed);
        let req = json!({
            "jsonrpc": "2.0",
            "id": rpc_id,
            "method": method,
            "params": params,
        });
        write_frame(&mut stdin, &req).await?;
        let envelope = read_response_for(&mut reader, rpc_id).await?;

        // Drop stdin so the child sees EOF and shuts down cleanly. The
        // `kill_on_drop` above is the belt-and-braces fallback for servers
        // that don't exit on EOF.
        drop(stdin);
        // Best-effort drain stderr for diagnostics; ignore failures.
        if let Some(mut err) = child.stderr.take() {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::with_capacity(2048);
            let _ = err.read_to_end(&mut buf).await;
            if !buf.is_empty() {
                tracing::debug!(
                    "stdio MCP `{}` stderr: {}",
                    cfg.id,
                    String::from_utf8_lossy(&buf).trim()
                );
            }
        }
        let _ = child.wait().await;

        unwrap_envelope(envelope)
    };

    match tokio::time::timeout(STDIO_TIMEOUT, inner).await {
        Ok(res) => res,
        Err(_) => bail!("stdio MCP `{}` timed out after {:?}", cfg.id, STDIO_TIMEOUT),
    }
}

async fn write_frame<W>(w: &mut W, value: &Value) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut line = serde_json::to_vec(value).context("encode JSON-RPC frame")?;
    line.push(b'\n');
    w.write_all(&line).await.context("write stdio frame")?;
    w.flush().await.context("flush stdio frame")?;
    Ok(())
}

/// Read newline-delimited JSON frames until one is the response to `wanted_id`.
/// Server-side notifications and unsolicited messages are logged and skipped.
async fn read_response_for<R>(reader: &mut tokio::io::Lines<R>, wanted_id: u64) -> Result<Response>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    loop {
        let line = reader
            .next_line()
            .await
            .context("read stdio frame")?
            .ok_or_else(|| anyhow!("stdio MCP closed before sending a response"))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let envelope: Response = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("stdio MCP: ignoring non-JSON line ({e}): {trimmed}");
                continue;
            }
        };
        match &envelope.id {
            Some(Value::Number(n)) if n.as_u64() == Some(wanted_id) => return Ok(envelope),
            None => {
                tracing::trace!("stdio MCP: ignoring server notification: {trimmed}");
                continue;
            }
            other => {
                tracing::trace!("stdio MCP: ignoring out-of-band response id={other:?}");
                continue;
            }
        }
    }
}

fn unwrap_envelope(envelope: Response) -> Result<Value> {
    if let Some(err) = envelope.error {
        return Err(anyhow!(
            "MCP error {} ({}): {:?}",
            err.code,
            err.message,
            err.data
        ));
    }
    envelope
        .result
        .ok_or_else(|| anyhow!("MCP response had neither `result` nor `error`"))
}

/// Concatenate `data:`-prefixed lines from a single SSE event and return
/// the assembled payload. We only care about the first event because
/// JSON-RPC requests are one-shot.
fn extract_first_sse_data(body: &str) -> Option<String> {
    let mut buf = String::new();
    for line in body.lines() {
        if line.is_empty() {
            if !buf.is_empty() {
                return Some(buf);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(rest.trim_start());
        }
    }
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

/// `tools/list` — enumerate the tool catalog. The MCP spec returns
/// `{ tools: [...] }`; we flatten that into a `Vec<ToolSchema>` for the UI.
pub async fn list_tools(http: &reqwest::Client, cfg: &McpServerConfig) -> Result<Vec<ToolSchema>> {
    let result = rpc(http, cfg, "tools/list", None).await?;
    let arr = result
        .get("tools")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("tools/list result missing `tools` array: {result}"))?;
    let mut out = Vec::with_capacity(arr.len());
    for raw in arr {
        let name = raw
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        let description = raw
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let input_schema = raw.get("inputSchema").cloned().unwrap_or_else(|| json!({}));
        let destructive = looks_destructive(&name);
        out.push(ToolSchema {
            name,
            description,
            input_schema,
            destructive,
        });
    }
    Ok(out)
}

/// `tools/call` — invoke a tool. The MCP result schema is
/// `{ content: [{ type: "text", text: "..." }, ...], isError: bool }`;
/// we coalesce all text parts into a single string for the simple UI.
pub async fn call_tool(
    http: &reqwest::Client,
    cfg: &McpServerConfig,
    name: &str,
    arguments: Value,
) -> Result<ToolResult> {
    let params = json!({
        "name": name,
        "arguments": arguments,
    });
    let result = rpc(http, cfg, "tools/call", Some(params)).await?;
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut content = String::new();
    if let Some(arr) = result.get("content").and_then(|v| v.as_array()) {
        for part in arr {
            if part.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(text);
                }
            }
        }
    }
    if content.is_empty() {
        // Fall back to the raw JSON when the server replied with a shape
        // we don't recognise (image / resource / structured-only).
        content = serde_json::to_string_pretty(&result).unwrap_or_default();
    }
    Ok(ToolResult { content, is_error })
}

/// Heuristic destructive-tool flag. We err on the side of *warning* —
/// the agent loop will gate execution on `Settings.destructive_tool_confirm`.
fn looks_destructive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "write", "delete", "remove", "exec", "shell", "kill", "drop", "rm_", "create_", "update_",
        "patch_", "post_",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_extractor_pulls_first_event() {
        let body = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[]}}\n\n";
        let extracted = extract_first_sse_data(body).unwrap();
        assert!(extracted.contains("\"result\""));
    }

    #[test]
    fn sse_extractor_handles_multiline_data() {
        let body = "data: {\"a\":1,\ndata: \"b\":2}\n\n";
        let extracted = extract_first_sse_data(body).unwrap();
        assert!(extracted.contains("\"a\":1"));
        assert!(extracted.contains("\"b\":2"));
    }

    #[test]
    fn destructive_flags_dangerous_names() {
        assert!(looks_destructive("fs.write"));
        assert!(looks_destructive("shell.exec"));
        assert!(!looks_destructive("fs.read"));
        assert!(!looks_destructive("search"));
    }
}
