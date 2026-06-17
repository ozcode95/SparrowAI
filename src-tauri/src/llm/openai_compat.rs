//! Generic OpenAI-compatible streaming client used by OVMS / ollama / vLLM.

use crate::llm::{ChatChunk, ChatRequest, ContentPart, FunctionCall, MessageContent, ToolCall};
use anyhow::Context;
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::mpsc;

/// Dedicated tracing target for the LLM request/response wire log. Kept
/// distinct from the module path so the file logger can dial just this
/// channel up/down without dragging the rest of the crate along.
const WIRE: &str = "llm::wire";

/// Typed view of the failures that escape [`stream_chat`] / [`open_stream`].
///
/// Returned via `anyhow::Error`, so it survives the trait object boundary in
/// [`crate::llm::LlmProvider::chat_stream`]; the chat runner downcasts to
/// this type to drive the structured `chat://error` event (hint, retry
/// suggestion, etc.).
#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    /// Initial POST never succeeded — every retry hit a transport error or
    /// retryable status. `last` is the most recent failure for context.
    #[error("upstream unreachable after {attempts} attempts: {last}")]
    Unreachable { attempts: u32, last: String },

    /// Initial POST returned a non-retryable HTTP status (e.g. 4xx other
    /// than 408/429). The body is included verbatim because OVMS / vLLM
    /// often pack the actionable detail in there.
    #[error("upstream {status}: {body}")]
    Http { status: StatusCode, body: String },
}

#[derive(Debug, Deserialize)]
struct StreamFrame {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    /// Some servers (notably OVMS on the terminating chunk) omit `delta`
    /// entirely once they have a `finish_reason` to report. Defaulting to
    /// an empty `Delta` keeps that frame valid instead of polluting the
    /// log with `bad SSE frame: missing field 'delta'` warnings.
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    /// Some servers expose chain-of-thought via a separate field.
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    /// Streamed tool-call fragments. Each entry carries an `index` that
    /// pins it to a slot in the assistant's final `tool_calls` array;
    /// fragments for the same index are concatenated. Typically the
    /// first fragment for an index supplies `id` + `function.name`, and
    /// subsequent fragments contribute `function.arguments` deltas.
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    /// Position in the assistant's final `tool_calls` array. The OpenAI
    /// spec guarantees this is stable for a given call across the
    /// streamed fragments — we use it as the accumulator key.
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Debug, Deserialize, Default)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Per-index accumulator for streamed tool-call fragments.
#[derive(Debug, Default)]
struct PendingToolCall {
    id: String,
    kind: String,
    name: String,
    arguments: String,
}

impl PendingToolCall {
    fn merge(&mut self, frag: ToolCallDelta) {
        if let Some(id) = frag.id {
            if !id.is_empty() {
                self.id = id;
            }
        }
        if let Some(kind) = frag.kind {
            if !kind.is_empty() {
                self.kind = kind;
            }
        }
        if let Some(fun) = frag.function {
            if let Some(name) = fun.name {
                if !name.is_empty() {
                    self.name = name;
                }
            }
            if let Some(args) = fun.arguments {
                self.arguments.push_str(&args);
            }
        }
    }

    fn into_tool_call(self) -> Option<ToolCall> {
        if self.name.is_empty() {
            return None;
        }
        Some(ToolCall {
            id: self.id,
            kind: if self.kind.is_empty() {
                "function".into()
            } else {
                self.kind
            },
            function: FunctionCall {
                name: self.name,
                // Empty argument string is equivalent to `"{}"` —
                // normalise here so dispatchers don't have to special
                // case it.
                arguments: if self.arguments.is_empty() {
                    "{}".into()
                } else {
                    self.arguments
                },
            },
        })
    }
}

/// Tunables for [`open_stream`]'s initial-POST retry loop. The default
/// matches production behaviour (4 attempts, 250ms initial backoff, 3s cap);
/// tests pass [`RetryConfig::no_wait`] to exercise the retry path without
/// burning real wall-clock time.
#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub cap_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            initial_delay: Duration::from_millis(250),
            cap_delay: Duration::from_secs(3),
        }
    }
}

impl RetryConfig {
    /// Test-only preset: 4 attempts, zero-length sleeps. Lets the retry
    /// behaviour be validated without inflating test runtime.
    #[cfg(test)]
    fn no_wait() -> Self {
        Self {
            max_attempts: 4,
            initial_delay: Duration::from_millis(0),
            cap_delay: Duration::from_millis(0),
        }
    }
}

pub async fn stream_chat(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    req: ChatRequest,
    sink: mpsc::Sender<anyhow::Result<ChatChunk>>,
) -> anyhow::Result<()> {
    stream_chat_with(
        client,
        base_url,
        api_key,
        req,
        sink,
        &RetryConfig::default(),
    )
    .await
}

/// Same as [`stream_chat`] but with a caller-supplied [`RetryConfig`]. Only
/// pub(crate) because no production caller needs to override the defaults —
/// it exists so tests can shrink the retry sleeps to zero.
pub(crate) async fn stream_chat_with(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    req: ChatRequest,
    sink: mpsc::Sender<anyhow::Result<ChatChunk>>,
    retry: &RetryConfig,
) -> anyhow::Result<()> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    // Log the outgoing request body. Inline base64 image payloads are
    // redacted because a single screenshot can balloon the line by 100s
    // of KB and bury the rest of the turn in the log file.
    let req_for_log = redact_request_for_log(&req);
    let req_pretty = serde_json::to_string_pretty(&req_for_log)
        .unwrap_or_else(|_| "<unserialisable request>".into());
    tracing::debug!(
        target: WIRE,
        url = %url,
        model = %req.model,
        stream = req.stream,
        messages = req.messages.len(),
        tools = req.tools.as_ref().map(|t| t.len()).unwrap_or(0),
        "llm request\n{}",
        req_pretty
    );

    let started = std::time::Instant::now();
    let resp = open_stream(client, &url, api_key, &req, retry)
        .await
        .with_context(|| format!("POST {url}"))?;

    let mut buf = String::new();
    let mut stream = resp.bytes_stream();
    // Tool calls arrive in pieces — first fragment has `id`+`name`, then
    // a long tail of `arguments` deltas — indexed by `tool_calls[].index`.
    // Accumulate per-index and flush as a single, fully-assembled batch
    // when the stream terminates.
    let mut pending_tool_calls: BTreeMap<u32, PendingToolCall> = BTreeMap::new();
    let mut last_finish_reason: Option<String> = None;
    // Accumulators for the wire log. Streaming the deltas verbatim to
    // the log would drown it; instead we buffer here and emit one entry
    // per turn once the stream terminates.
    let mut log_content = String::new();
    let mut log_thinking = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        buf.push_str(&String::from_utf8_lossy(&bytes));

        // SSE frames are separated by blank lines.
        while let Some(idx) = buf.find("\n\n") {
            let raw_frame = buf[..idx].to_string();
            buf.drain(..idx + 2);

            for line in raw_frame.lines() {
                let line = line.trim_start();
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    let tool_calls = drain_tool_calls(&mut pending_tool_calls);
                    log_response_complete(
                        &req,
                        &log_content,
                        &log_thinking,
                        &tool_calls,
                        last_finish_reason.as_deref(),
                        started.elapsed(),
                        "[DONE]",
                    );
                    let _ = sink
                        .send(Ok(ChatChunk {
                            delta: String::new(),
                            thinking: false,
                            done: true,
                            tool_calls,
                            finish_reason: last_finish_reason.take(),
                        }))
                        .await;
                    return Ok(());
                }
                match serde_json::from_str::<StreamFrame>(data) {
                    Ok(frame) => {
                        for c in frame.choices {
                            let think = c
                                .delta
                                .reasoning
                                .or(c.delta.reasoning_content)
                                .unwrap_or_default();
                            if !think.is_empty() {
                                log_thinking.push_str(&think);
                                let _ = sink
                                    .send(Ok(ChatChunk {
                                        delta: think,
                                        thinking: true,
                                        done: false,
                                        tool_calls: Vec::new(),
                                        finish_reason: None,
                                    }))
                                    .await;
                            }
                            if let Some(content) = c.delta.content {
                                if !content.is_empty() {
                                    log_content.push_str(&content);
                                    let _ = sink
                                        .send(Ok(ChatChunk {
                                            delta: content,
                                            thinking: false,
                                            done: false,
                                            tool_calls: Vec::new(),
                                            finish_reason: None,
                                        }))
                                        .await;
                                }
                            }
                            if let Some(frags) = c.delta.tool_calls {
                                for frag in frags {
                                    pending_tool_calls
                                        .entry(frag.index)
                                        .or_default()
                                        .merge(frag);
                                }
                            }
                            if let Some(reason) = c.finish_reason {
                                last_finish_reason = Some(reason);
                                let tool_calls = drain_tool_calls(&mut pending_tool_calls);
                                log_response_complete(
                                    &req,
                                    &log_content,
                                    &log_thinking,
                                    &tool_calls,
                                    last_finish_reason.as_deref(),
                                    started.elapsed(),
                                    "finish_reason",
                                );
                                let _ = sink
                                    .send(Ok(ChatChunk {
                                        delta: String::new(),
                                        thinking: false,
                                        done: true,
                                        tool_calls,
                                        finish_reason: last_finish_reason.take(),
                                    }))
                                    .await;
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("bad SSE frame: {e} :: {data}");
                    }
                }
            }
        }
    }

    // Stream ended without a [DONE] sentinel or finish_reason. Still emit
    // the wire log so the partial turn isn't silently dropped from the
    // record — useful when chasing truncated upstream responses.
    let tool_calls = drain_tool_calls(&mut pending_tool_calls);
    log_response_complete(
        &req,
        &log_content,
        &log_thinking,
        &tool_calls,
        last_finish_reason.as_deref(),
        started.elapsed(),
        "stream-closed",
    );
    Ok(())
}

/// Build a JSON view of a [`ChatRequest`] safe to dump into the log.
/// Image `data:` URLs are truncated to a small head + size marker so a
/// vision turn doesn't add hundreds of KB of base64 to every log line.
fn redact_request_for_log(req: &ChatRequest) -> serde_json::Value {
    let mut redacted = req.clone();
    for msg in &mut redacted.messages {
        if let Some(MessageContent::Parts(parts)) = msg.content.as_mut() {
            for part in parts {
                if let ContentPart::ImageUrl { image_url } = part {
                    let url = &image_url.url;
                    if url.starts_with("data:") && url.len() > 80 {
                        // Keep the MIME prefix (`data:image/png;base64,`)
                        // so the entry is still self-describing.
                        let head: String = url.chars().take(40).collect();
                        image_url.url =
                            format!("{head}...<{} bytes elided>", url.len() - head.len());
                    }
                }
            }
        }
    }
    serde_json::to_value(&redacted).unwrap_or(serde_json::Value::Null)
}

/// Emit a single debug-level wire-log entry summarising one completed
/// turn. Mirrors the request log's structural JSON shape (an OpenAI
/// `assistant` message with `tool_calls` + `thinking` + `finish_reason`)
/// so the request and response read as one continuous conversation when
/// you tail the log.
fn log_response_complete(
    req: &ChatRequest,
    content: &str,
    thinking: &str,
    tool_calls: &[ToolCall],
    finish_reason: Option<&str>,
    elapsed: Duration,
    terminator: &'static str,
) {
    let mut response = serde_json::Map::new();
    response.insert("role".into(), serde_json::Value::from("assistant"));
    response.insert("content".into(), serde_json::Value::from(content));
    if !thinking.is_empty() {
        response.insert("thinking".into(), serde_json::Value::from(thinking));
    }
    if !tool_calls.is_empty() {
        response.insert(
            "tool_calls".into(),
            serde_json::to_value(tool_calls).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(reason) = finish_reason {
        response.insert("finish_reason".into(), serde_json::Value::from(reason));
    }
    let response_pretty = serde_json::to_string_pretty(&serde_json::Value::Object(response))
        .unwrap_or_else(|_| "<unserialisable response>".into());
    tracing::debug!(
        target: WIRE,
        model = %req.model,
        finish_reason = finish_reason.unwrap_or("<none>"),
        terminator = terminator,
        elapsed_ms = elapsed.as_millis() as u64,
        content_len = content.len(),
        thinking_len = thinking.len(),
        tool_calls = tool_calls.len(),
        "llm response\n{}",
        response_pretty
    );
}

/// Drain the per-index pending tool-call map into a flat list in index
/// order. Empty / nameless entries are dropped — a stray fragment with no
/// `function.name` ever arriving means the upstream stream lied to us,
/// and we'd rather skip it than send a malformed call downstream.
fn drain_tool_calls(pending: &mut BTreeMap<u32, PendingToolCall>) -> Vec<ToolCall> {
    let drained = std::mem::take(pending);
    drained
        .into_values()
        .filter_map(PendingToolCall::into_tool_call)
        .collect()
}

/// Open a streaming chat-completions response, retrying transient failures
/// with capped exponential backoff. Only the *initial* POST is retried —
/// once we have a Response we're committed (re-running a partially-streamed
/// turn would produce duplicate text).
///
/// Retried:
///   - transport errors (connect refused, timeout, request build error)
///   - HTTP 408 / 429
///   - any 5xx
///
/// Not retried:
///   - 2xx (returned immediately)
///   - other 4xx (treated as caller bugs / configuration errors)
async fn open_stream(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    req: &ChatRequest,
    retry: &RetryConfig,
) -> anyhow::Result<reqwest::Response> {
    let mut delay = retry.initial_delay;
    let mut last_err: Option<String> = None;

    for attempt in 1..=retry.max_attempts {
        let mut rb = client.post(url).json(req);
        if let Some(k) = api_key {
            rb = rb.bearer_auth(k);
        }

        match rb.send().await {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if !is_retryable_status(status) {
                    return Err(UpstreamError::Http { status, body }.into());
                }
                last_err = Some(format!("{status}: {body}"));
            }
            Err(e) => {
                if !is_retryable_transport(&e) {
                    return Err(e.into());
                }
                last_err = Some(e.to_string());
            }
        }

        if attempt < retry.max_attempts {
            tracing::warn!(
                "chat upstream attempt {attempt}/{} failed: {} — retrying in {:?}",
                retry.max_attempts,
                last_err.as_deref().unwrap_or("<unknown>"),
                delay
            );
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            // Cap doubles — but never below the initial value, which keeps
            // `no_wait` (initial=0) at zero forever instead of growing.
            delay = (delay * 2).min(retry.cap_delay);
        }
    }

    Err(UpstreamError::Unreachable {
        attempts: retry.max_attempts,
        last: last_err.unwrap_or_else(|| "<no error captured>".into()),
    }
    .into())
}

fn is_retryable_status(status: StatusCode) -> bool {
    status.is_server_error()
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
}

fn is_retryable_transport(e: &reqwest::Error) -> bool {
    // `is_request` covers "could not build the request" which is usually a
    // permanent caller bug, but `is_connect` / `is_timeout` cover the
    // "OVMS isn't quite ready yet" case we actually want to retry.
    e.is_connect() || e.is_timeout()
}

#[cfg(test)]
mod tests {
    //! Integration tests for the OpenAI-compatible streaming client.
    //!
    //! We exercise the full `stream_chat` lifecycle against a tiny inline
    //! HTTP/1.1 mock that we drive at the TCP level — keeps the test
    //! deterministic without pulling in hyper/axum/wiremock as a dev
    //! dependency, and lets us simulate response bodies (multi-chunk SSE,
    //! malformed frames, partial writes) exactly as we want.

    use super::*;
    use crate::llm::{ChatMessage, ChatRequest};
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    /// A single canned response served by [`MockServer`] for one connection.
    /// `pre_body_delay_ms` lets us stall after writing the headers to
    /// exercise the SSE streaming path that buffers across chunks.
    #[derive(Clone)]
    struct Response {
        bytes: Vec<u8>,
        pre_body_delay_ms: u64,
    }

    impl Response {
        fn ok_sse(body: &str) -> Self {
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let mut bytes = headers.into_bytes();
            bytes.extend_from_slice(body.as_bytes());
            Self {
                bytes,
                pre_body_delay_ms: 0,
            }
        }

        fn status(code: u16, reason: &str, body: &str) -> Self {
            let headers = format!(
                "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let mut bytes = headers.into_bytes();
            bytes.extend_from_slice(body.as_bytes());
            Self {
                bytes,
                pre_body_delay_ms: 0,
            }
        }
    }

    /// Mock HTTP server bound to a random ephemeral port. Each incoming
    /// connection pops the next [`Response`] off `responses`; once the
    /// queue is empty the server hangs up immediately (so an unexpected
    /// extra request surfaces as a connection-closed error rather than
    /// hanging the test).
    struct MockServer {
        addr: SocketAddr,
        request_count: Arc<Mutex<u32>>,
    }

    impl MockServer {
        async fn start(responses: Vec<Response>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let request_count = Arc::new(Mutex::new(0u32));
            let counter = Arc::clone(&request_count);

            tokio::spawn(async move {
                let queue = Arc::new(Mutex::new(responses.into_iter()));
                loop {
                    let (mut sock, _) = match listener.accept().await {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    *counter.lock().await += 1;
                    let queue = Arc::clone(&queue);
                    tokio::spawn(async move {
                        // Drain request headers + body so reqwest doesn't see
                        // a RST while it's still flushing. We don't actually
                        // parse the request — the mock is response-driven.
                        let mut buf = [0u8; 4096];
                        let _ =
                            tokio::time::timeout(Duration::from_millis(200), sock.read(&mut buf))
                                .await;

                        let next = queue.lock().await.next();
                        let Some(resp) = next else {
                            // No more canned responses — hang up.
                            return;
                        };
                        if resp.pre_body_delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(resp.pre_body_delay_ms)).await;
                        }
                        let _ = sock.write_all(&resp.bytes).await;
                        let _ = sock.shutdown().await;
                    });
                }
            });

            Self {
                addr,
                request_count,
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        async fn request_count(&self) -> u32 {
            *self.request_count.lock().await
        }
    }

    fn dummy_req() -> ChatRequest {
        ChatRequest {
            model: "test-model".into(),
            messages: vec![ChatMessage::text("user", "hi")],
            temperature: None,
            max_tokens: None,
            top_p: None,
            top_k: None,
            stream: true,
            tools: None,
            tool_choice: None,
            chat_template_kwargs: None,
        }
    }

    /// Collect every chunk the runner emits into a flat vector, returning
    /// the overall `stream_chat` result alongside.
    async fn collect(base_url: &str) -> (anyhow::Result<()>, Vec<ChatChunk>) {
        let client = reqwest::Client::builder()
            // Short connect timeout so the `connection_refused` test doesn't
            // wait the full OS default on Windows (~2s per attempt). All
            // other tests connect to a live loopback listener and complete
            // well inside this budget.
            .connect_timeout(Duration::from_millis(200))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let (tx, mut rx) = mpsc::channel::<anyhow::Result<ChatChunk>>(64);
        let req = dummy_req();
        let drain = tokio::spawn(async move {
            let mut out = Vec::new();
            while let Some(item) = rx.recv().await {
                if let Ok(c) = item {
                    out.push(c);
                }
            }
            out
        });
        let res = stream_chat_with(&client, base_url, None, req, tx, &RetryConfig::no_wait()).await;
        let chunks = drain.await.unwrap();
        (res, chunks)
    }

    #[tokio::test]
    async fn happy_path_emits_content_and_thinking_then_done() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning\":\"think \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        );
        let server = MockServer::start(vec![Response::ok_sse(body)]).await;
        let (res, chunks) = collect(&server.base_url()).await;

        assert!(res.is_ok(), "stream_chat err: {res:?}");
        assert_eq!(chunks.len(), 4);
        assert!(chunks[0].thinking && chunks[0].delta == "think ");
        assert!(!chunks[1].thinking && chunks[1].delta == "hello ");
        assert!(!chunks[2].thinking && chunks[2].delta == "world");
        assert!(chunks[3].done && chunks[3].delta.is_empty());
        assert_eq!(server.request_count().await, 1);
    }

    #[tokio::test]
    async fn done_sentinel_terminates_stream() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let server = MockServer::start(vec![Response::ok_sse(body)]).await;
        let (res, chunks) = collect(&server.base_url()).await;

        assert!(res.is_ok());
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].delta, "hi");
        assert!(chunks[1].done);
    }

    #[tokio::test]
    async fn malformed_frames_are_skipped_not_fatal() {
        let body = concat!(
            "data: {not json}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let server = MockServer::start(vec![Response::ok_sse(body)]).await;
        let (res, chunks) = collect(&server.base_url()).await;

        assert!(res.is_ok());
        // Only the well-formed frame + the [DONE] sentinel produce chunks.
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].delta, "ok");
        assert!(chunks[1].done);
    }

    #[tokio::test]
    async fn http_401_surfaces_as_upstream_http_error_without_retry() {
        let server = MockServer::start(vec![Response::status(
            401,
            "Unauthorized",
            r#"{"error":"missing token"}"#,
        )])
        .await;
        let (res, chunks) = collect(&server.base_url()).await;

        let err = res.expect_err("401 should bubble up");
        let up = err
            .downcast_ref::<UpstreamError>()
            .expect("err should downcast to UpstreamError");
        match up {
            UpstreamError::Http { status, body } => {
                assert_eq!(status.as_u16(), 401);
                assert!(body.contains("missing token"), "body was {body:?}");
            }
            other => panic!("expected Http, got {other:?}"),
        }
        assert!(chunks.is_empty(), "no chunks should leak before the error");
        // 4xx (non-408/429) must NOT retry.
        assert_eq!(server.request_count().await, 1);
    }

    #[tokio::test]
    async fn http_503_retries_then_succeeds_on_recovery() {
        let good = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let server = MockServer::start(vec![
            Response::status(503, "Service Unavailable", "warming up"),
            Response::status(503, "Service Unavailable", "still warming"),
            Response::ok_sse(good),
        ])
        .await;
        let (res, chunks) = collect(&server.base_url()).await;

        assert!(res.is_ok(), "stream_chat err: {res:?}");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].delta, "ok");
        assert!(chunks[1].done);
        // Two 503s + one 200 → three connections total.
        assert_eq!(server.request_count().await, 3);
    }

    #[tokio::test]
    async fn http_503_exhausting_retries_returns_unreachable() {
        // Four 503s == MAX_ATTEMPTS in `open_stream`. We expect Unreachable
        // rather than Http, because the loop only returns Http for
        // *non-retryable* statuses; the retryable path that exhausts the
        // budget surfaces as the Unreachable variant.
        let server = MockServer::start(vec![
            Response::status(503, "Service Unavailable", "x"),
            Response::status(503, "Service Unavailable", "x"),
            Response::status(503, "Service Unavailable", "x"),
            Response::status(503, "Service Unavailable", "x"),
        ])
        .await;
        let (res, _chunks) = collect(&server.base_url()).await;

        let err = res.expect_err("exhausted retries should surface an error");
        let up = err
            .downcast_ref::<UpstreamError>()
            .expect("err should downcast to UpstreamError");
        match up {
            UpstreamError::Unreachable { attempts, last } => {
                assert_eq!(*attempts, 4);
                assert!(last.contains("503"), "last was {last:?}");
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
        assert_eq!(server.request_count().await, 4);
    }

    #[tokio::test]
    async fn connection_refused_surfaces_as_unreachable() {
        // Bind+drop to claim a port then immediately release it. The
        // subsequent connect attempts will be refused (or, on some
        // platforms, treated as a transport error reqwest classifies as
        // is_connect()); either way the retry budget should exhaust into
        // an Unreachable.
        let unused = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = l.local_addr().unwrap();
            drop(l);
            addr
        };
        let base = format!("http://{unused}");
        let (res, chunks) = collect(&base).await;

        assert!(chunks.is_empty());
        let err = res.expect_err("refused connect should error");
        let up = err
            .downcast_ref::<UpstreamError>()
            .expect("err should downcast to UpstreamError");
        assert!(
            matches!(up, UpstreamError::Unreachable { .. }),
            "got {up:?}"
        );
    }

    /// OpenAI streaming tool-call shape: first frame supplies
    /// `id`+`type`+`function.name`, subsequent frames append
    /// `function.arguments` deltas, finish_reason flips to `tool_calls`
    /// when the model is done. Verify per-index accumulation and a
    /// single fully-assembled batch on the terminating chunk.
    #[tokio::test]
    async fn tool_call_fragments_accumulate_and_flush_on_finish() {
        let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"fs_list\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\"}}]},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"/tmp\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
        let server = MockServer::start(vec![Response::ok_sse(body)]).await;
        let (res, chunks) = collect(&server.base_url()).await;
        res.expect("stream should succeed");

        let done = chunks.iter().find(|c| c.done).expect("terminating chunk");
        assert_eq!(done.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(done.tool_calls.len(), 1, "got: {:#?}", done.tool_calls);
        let call = &done.tool_calls[0];
        assert_eq!(call.id, "call_1");
        assert_eq!(call.kind, "function");
        assert_eq!(call.function.name, "fs_list");
        // Two `arguments` fragments concatenated into the final JSON.
        assert_eq!(call.function.arguments, "{\"path\":\"/tmp\"}");
        // Fragments must not leak as standalone calls on intermediate chunks.
        for c in chunks.iter().filter(|c| !c.done) {
            assert!(c.tool_calls.is_empty(), "leaked: {c:?}");
        }
    }

    /// A regular content-only stream must still produce an empty
    /// `tool_calls` list on the terminating chunk so downstream code can
    /// branch on `chunk.tool_calls.is_empty()` without special-casing.
    #[tokio::test]
    async fn content_only_stream_produces_no_tool_calls() {
        let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let server = MockServer::start(vec![Response::ok_sse(body)]).await;
        let (res, chunks) = collect(&server.base_url()).await;
        res.expect("stream should succeed");

        let done = chunks.iter().find(|c| c.done).expect("terminating chunk");
        assert!(done.tool_calls.is_empty());
        assert_eq!(done.finish_reason.as_deref(), Some("stop"));
    }
}
