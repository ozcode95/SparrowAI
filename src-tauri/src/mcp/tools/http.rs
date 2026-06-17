//! HTTP-fetch built-in tool.
//!
//! Lets the model issue an arbitrary HTTP request (GET / POST / PUT /
//! PATCH / DELETE / HEAD) and returns the status, response headers, and
//! response body. The shared [`reqwest::Client`] in [`crate::state::AppState`]
//! is reused so connection pooling and the project user-agent come along
//! for free.
//!
//! Safety / size considerations:
//!
//! - Response bodies are capped at [`MAX_BODY_BYTES`] and truncated with
//!   a `… [truncated]` marker if larger.
//! - Binary responses (anything that isn't valid UTF-8) are base64-encoded
//!   so the model still gets a deterministic representation it can echo
//!   back into a follow-up tool call.
//! - A `timeout_ms` (default [`DEFAULT_TIMEOUT_MS`]) bounds total request
//!   time so a slow server can't gate the chat.
//!
//! Not marked destructive: GET / HEAD requests are the common case and
//! adding a confirm prompt to every web fetch would train users to
//! disable the gate. POST / PUT / DELETE are technically observable
//! side-effects on remote systems but the destructive flag is meant for
//! *local* side-effects (filesystem, shell, scheduled tasks); remote
//! servers are out of the trust boundary either way.

use crate::mcp::{Tool, ToolResult, ToolSchema};
use crate::state::AppStateExt;
use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tauri::AppHandle;

/// Cap on response-body bytes returned to the model.
const MAX_BODY_BYTES: usize = 256 * 1024;

/// Default total request timeout.
const DEFAULT_TIMEOUT_MS: u64 = 20_000;

/// Hard upper bound to prevent the model from picking a huge timeout.
const MAX_TIMEOUT_MS: u64 = 2 * 60 * 1000;

#[derive(Debug)]
pub struct HttpFetch {
    http: Client,
}

impl HttpFetch {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            http: app.zero().http.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct FetchArgs {
    url: String,
    /// HTTP method. Defaults to GET. Case-insensitive.
    #[serde(default)]
    method: Option<String>,
    /// Optional request headers. String → string only; structured values
    /// must be serialised by the caller.
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
    /// Optional UTF-8 request body. For binary uploads, base64-encode it
    /// here and set a matching `Content-Encoding` / `Content-Type` header.
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[async_trait]
impl Tool for HttpFetch {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "http.fetch".into(),
            description: "Make an HTTP request and return the status, \
                 response headers, and response body. Supports GET, \
                 POST, PUT, PATCH, DELETE, HEAD. Bodies larger than \
                 ~256 KiB are truncated; non-UTF-8 bodies are base64 \
                 encoded. Uses the shared client (so http_proxy / \
                 user-agent settings apply)."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Absolute URL (http or https)."
                    },
                    "method": {
                        "type": "string",
                        "description": "HTTP method. Default GET.",
                        "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"]
                    },
                    "headers": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Request headers as a flat string map."
                    },
                    "body": {
                        "type": "string",
                        "description": "UTF-8 request body. Pair with a Content-Type header."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Total request timeout. Default 20000, max 120000."
                    }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: FetchArgs = serde_json::from_value(args).context("http.fetch: parse arguments")?;
        if a.url.trim().is_empty() {
            return Ok(ToolResult {
                content: "http.fetch: `url` is empty".into(),
                is_error: true,
            });
        }

        let method = parse_method(a.method.as_deref())?;
        let timeout = Duration::from_millis(
            a.timeout_ms
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .clamp(1, MAX_TIMEOUT_MS),
        );

        let mut header_map = HeaderMap::new();
        if let Some(h) = a.headers.as_ref() {
            for (k, v) in h {
                let name = HeaderName::from_bytes(k.as_bytes())
                    .with_context(|| format!("http.fetch: invalid header name `{k}`"))?;
                let value = HeaderValue::from_str(v)
                    .with_context(|| format!("http.fetch: invalid header value for `{k}`"))?;
                header_map.insert(name, value);
            }
        }

        let mut req = self
            .http
            .request(method.clone(), &a.url)
            .timeout(timeout)
            .headers(header_map);
        if let Some(body) = a.body {
            req = req.body(body);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult {
                    content: format!("http.fetch: request failed: {e}"),
                    is_error: true,
                });
            }
        };

        let status = resp.status();
        let status_line = format!(
            "{} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        );
        let mut header_lines = Vec::new();
        for (k, v) in resp.headers() {
            let v_str = v.to_str().unwrap_or("<binary>");
            header_lines.push(format!("{}: {}", k, v_str));
        }
        header_lines.sort();

        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return Ok(ToolResult {
                    content: format!(
                        "{status_line}\n{}\n--- body ---\n[read error: {e}]",
                        header_lines.join("\n")
                    ),
                    is_error: true,
                });
            }
        };

        let (body_text, truncated, encoding) = render_body(&bytes);
        let mut content = String::new();
        content.push_str(&format!("HTTP {} {}\n", method, status_line));
        content.push_str(&header_lines.join("\n"));
        if !header_lines.is_empty() {
            content.push('\n');
        }
        content.push_str(&format!("--- body ({encoding}) ---\n"));
        content.push_str(&body_text);
        if truncated {
            content.push_str("\n… [truncated]");
        }

        Ok(ToolResult {
            content,
            // Non-2xx still resolves to a successful tool call — the
            // model needs to see the status to react to it. Network /
            // parse failures above set `is_error = true`.
            is_error: false,
        })
    }
}

fn parse_method(raw: Option<&str>) -> Result<Method> {
    let s = raw.unwrap_or("GET").trim();
    if s.is_empty() {
        return Ok(Method::GET);
    }
    s.to_ascii_uppercase()
        .parse::<Method>()
        .with_context(|| format!("http.fetch: unsupported method `{s}`"))
}

/// Try to render `bytes` as UTF-8, truncating at [`MAX_BODY_BYTES`].
/// Falls back to base64 when the bytes aren't valid UTF-8. Returns
/// `(text, truncated, encoding)` where `encoding` is one of
/// `"utf-8"` / `"base64"`.
fn render_body(bytes: &[u8]) -> (String, bool, &'static str) {
    let truncated = bytes.len() > MAX_BODY_BYTES;
    let slice = if truncated {
        &bytes[..MAX_BODY_BYTES]
    } else {
        bytes
    };
    match std::str::from_utf8(slice) {
        Ok(s) => (s.to_string(), truncated, "utf-8"),
        Err(_) => (
            base64::engine::general_purpose::STANDARD.encode(slice),
            truncated,
            "base64",
        ),
    }
}

pub fn all(app: &AppHandle) -> Vec<Box<dyn Tool>> {
    vec![Box::new(HttpFetch::new(app))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_method_defaults_to_get_and_normalises_case() {
        assert_eq!(parse_method(None).unwrap(), Method::GET);
        assert_eq!(parse_method(Some("")).unwrap(), Method::GET);
        assert_eq!(parse_method(Some("post")).unwrap(), Method::POST);
        assert_eq!(parse_method(Some("DELETE")).unwrap(), Method::DELETE);
    }

    #[test]
    fn parse_method_rejects_garbage() {
        // `http::Method` accepts any valid HTTP token as an extension
        // method (e.g. PURGE), so we need a string with characters
        // outside the token grammar to provoke a parse failure.
        // (Pure-whitespace strings are normalised to GET above, so
        // they don't qualify as "garbage" here.)
        assert!(parse_method(Some("BAD METHOD")).is_err());
        assert!(parse_method(Some("GET\nHACK")).is_err());
    }

    #[test]
    fn render_body_returns_utf8_when_valid() {
        let (text, truncated, enc) = render_body(b"hello");
        assert_eq!(text, "hello");
        assert!(!truncated);
        assert_eq!(enc, "utf-8");
    }

    #[test]
    fn render_body_falls_back_to_base64_for_invalid_utf8() {
        let (text, truncated, enc) = render_body(&[0xff, 0xfe, 0x00]);
        assert_eq!(enc, "base64");
        assert!(!truncated);
        assert!(!text.is_empty());
    }

    #[test]
    fn render_body_truncates_oversized_input() {
        let big = vec![b'a'; MAX_BODY_BYTES + 10];
        let (text, truncated, enc) = render_body(&big);
        assert!(truncated);
        assert_eq!(enc, "utf-8");
        assert_eq!(text.len(), MAX_BODY_BYTES);
    }
}
