//! Provider-agnostic LLM interface.
//!
//! Every backend (OVMS, ollama, llama.cpp) speaks an OpenAI-compatible
//! `/chat/completions` API, so the trait stays thin and the concrete clients
//! just point at a different base URL.

pub mod llamacpp;
pub mod ollama;
pub mod openai_compat;
pub mod ovms;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    /// Either a plain string (the common case) or an array of typed parts
    /// (the OpenAI Vision shape we use when the user attached images). Wire
    /// format follows the [OVMS chat/completions spec][1] verbatim so the
    /// same request body reaches OVMS, llama.cpp and ollama unchanged.
    ///
    /// Made optional so assistant messages whose sole purpose is to carry
    /// `tool_calls` (e.g. "the model asked for a tool, it produced no
    /// visible content") can omit `content` entirely instead of sending an
    /// empty string — some strict OpenAI-compatible servers reject that.
    ///
    /// [1]: https://docs.openvino.ai/2026/model-server/ovms_docs_rest_api_chat.html
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,

    /// Set on assistant messages that asked for one or more tool calls
    /// (OpenAI native function-calling protocol). Each call's `id` is
    /// later echoed back as `tool_call_id` on the matching `role: "tool"`
    /// follow-up so the model can correlate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Set on `role: "tool"` messages — echoes the assistant's tool-call id
    /// so the model can bind the result back to its request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    /// Set on `role: "tool"` messages — the function name that produced
    /// this result. Strictly speaking optional but several chat templates
    /// (Hermes, Llama 3, Qwen) lean on it for tool-result rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(MessageContent::Text(content.into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn parts(role: impl Into<String>, parts: Vec<ContentPart>) -> Self {
        Self {
            role: role.into(),
            content: Some(MessageContent::Parts(parts)),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Assistant turn that exists solely to ferry `tool_calls` to the
    /// model on the next round. `content` is included as an empty string
    /// when `text` is empty because some servers (including stricter
    /// OpenAI clones) reject an assistant message with neither `content`
    /// nor a non-empty `tool_calls` list rendered through their template.
    pub fn assistant_tool_calls(text: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        let text = text.into();
        Self {
            role: "assistant".into(),
            content: Some(MessageContent::Text(text)),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            name: None,
        }
    }

    /// `role: "tool"` message carrying a single tool's result, bound to
    /// the assistant's prior call via `tool_call_id`.
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".into(),
            content: Some(MessageContent::Text(content.into())),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
        }
    }
}

/// Content payload for a single chat message. `untagged` so the wire format
/// flips between `"hi"` and `[{type:"text",...}, {type:"image_url",...}]`
/// purely based on which variant the runner constructs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        MessageContent::Text(s)
    }
}
impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        MessageContent::Text(s.to_string())
    }
}

/// One element of a multimodal `content` array. Mirrors the OpenAI Vision
/// shape OVMS accepts (`type: "text"` / `type: "image_url"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    /// Either a `data:` URL (base64 inline) or a remote `http(s)://` URL.
    /// OVMS additionally accepts local filesystem paths when started with
    /// `--allowed_local_media_path`; we don't lean on that because data
    /// URLs work zero-config.
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    // Optional knobs are *omitted* (not serialised as `null`) when unset.
    // OVMS's LLM calculator validates types strictly and rejects `null`
    // for `max_tokens` with `"max_tokens is not an unsigned integer"`;
    // other OpenAI-compatible servers behave similarly, so skipping nulls
    // is the safe default for the whole transport.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Nucleus-sampling cutoff. Forwarded verbatim to the upstream
    /// when set; omitted when `None`. OpenAI-compat servers (OVMS,
    /// llama.cpp, ollama) all accept this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Top-k sampling cutoff. Not part of the original OpenAI spec but
    /// supported as an extension by OVMS / llama.cpp / ollama, and required
    /// for Gemma 4 to hit its documented sampling profile. Skipped on
    /// the wire when `None` so providers that ignore it aren't tripped
    /// up by a stray field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    pub stream: bool,
    /// OpenAI native function-calling tool catalogue. Models trained for
    /// the tools protocol (Llama 3.1+, Hermes 3, Qwen 2.5+, Mistral with
    /// the right parser, etc.) read this and emit `tool_calls` in their
    /// response instead of free-form fenced JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    /// `"auto"` (let the model decide), `"required"` (must call a tool),
    /// `"none"` (must not call a tool). Omitted means provider default,
    /// which is `"auto"` whenever `tools` is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    /// Template-specific kwargs passed to the chat template. Used by Qwen
    /// models to control thinking via `{"enable_thinking": false}`. Omitted
    /// when `None` so providers that don't support it aren't affected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<serde_json::Value>,
}

/// One entry in a [`ChatRequest::tools`] array — the OpenAI function spec.
/// We only support the `function` kind because that's all OVMS exposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionDef,
}

impl ToolDef {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            kind: "function".into(),
            function: FunctionDef {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// JSON-schema object describing the function's argument shape. OVMS
    /// passes this verbatim into the model's chat template.
    pub parameters: serde_json::Value,
}

/// A single tool invocation requested by the model. The wire shape is
/// fixed by the OpenAI spec: `arguments` is a JSON-encoded **string**
/// (not a JSON object), so callers must `serde_json::from_str` it before
/// dispatching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// JSON-encoded string of the argument object. Empty string is
    /// equivalent to `"{}"`.
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatChunk {
    /// Token delta. Empty `delta` + `done = true` marks end of stream.
    pub delta: String,
    pub thinking: bool,
    pub done: bool,
    /// Fully-assembled tool calls emitted alongside `done = true` when
    /// `finish_reason == "tool_calls"`. Empty for content-only rounds.
    /// The provider/transport layer accumulates the streamed deltas
    /// (which arrive in pieces, indexed by `tool_calls[].index`) so the
    /// runner sees complete entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Last `finish_reason` we saw on the stream (`"stop"`, `"length"`,
    /// `"tool_calls"`, ...). Set only on the terminating chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;

    /// Stream a chat completion. Implementations should write `ChatChunk`s
    /// until done (or drop the sender on error). Callers must drain.
    async fn chat_stream(
        &self,
        req: ChatRequest,
        sink: mpsc::Sender<anyhow::Result<ChatChunk>>,
    ) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OVMS's `LLMCalculator` rejects `null` for `max_tokens` (and other
    /// typed knobs). Make sure `None`s are dropped from the wire payload
    /// instead of being serialised as JSON nulls.
    #[test]
    fn omits_optional_fields_when_unset() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text("user", "hi")],
            temperature: None,
            max_tokens: None,
            top_p: None,
            top_k: None,
            stream: true,
            tools: None,
            tool_choice: None,
            chat_template_kwargs: None,
        };
        let v: serde_json::Value = serde_json::to_value(&req).unwrap();
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("temperature"), "got: {v}");
        assert!(!obj.contains_key("max_tokens"), "got: {v}");
        assert!(!obj.contains_key("top_p"), "got: {v}");
        assert!(!obj.contains_key("top_k"), "got: {v}");
        assert!(!obj.contains_key("tools"), "got: {v}");
        assert!(!obj.contains_key("tool_choice"), "got: {v}");
    }

    #[test]
    fn keeps_optional_fields_when_set() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![],
            temperature: Some(0.7),
            max_tokens: Some(128),
            top_p: Some(0.95),
            top_k: Some(64),
            stream: false,
            tools: None,
            tool_choice: None,
            chat_template_kwargs: None,
        };
        let v: serde_json::Value = serde_json::to_value(&req).unwrap();
        // Use approx-equality for the f32 → f64 widening that happens
        // when serde_json converts the `Option<f32>` to a JSON number.
        let temp = v["temperature"].as_f64().unwrap();
        assert!((temp - 0.7).abs() < 1e-5, "temperature was {temp}");
        assert_eq!(v["max_tokens"], serde_json::json!(128));
        let tp = v["top_p"].as_f64().unwrap();
        assert!((tp - 0.95).abs() < 1e-5, "top_p was {tp}");
        assert_eq!(v["top_k"], serde_json::json!(64));
    }

    /// Text-only messages must still serialise as `{"content": "..."}` (a
    /// bare string) — OVMS's LLM calculator and most OpenAI-compatible
    /// servers reject `content: ["..."]` strings-as-arrays.
    #[test]
    fn text_content_serialises_as_bare_string() {
        let msg = ChatMessage::text("user", "hi");
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["content"], serde_json::json!("hi"));
    }

    /// Multimodal messages must serialise as an array of typed parts per
    /// the OVMS vision request shape.
    #[test]
    fn parts_content_serialises_as_typed_array() {
        let msg = ChatMessage::parts(
            "user",
            vec![
                ContentPart::Text {
                    text: "describe".into(),
                },
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: "data:image/png;base64,iVBORw0KGgo".into(),
                    },
                },
            ],
        );
        let v = serde_json::to_value(&msg).unwrap();
        let arr = v["content"].as_array().expect("content array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "describe");
        assert_eq!(arr[1]["type"], "image_url");
        assert_eq!(
            arr[1]["image_url"]["url"],
            "data:image/png;base64,iVBORw0KGgo"
        );
    }

    /// Assistant messages that only carry `tool_calls` should serialise
    /// in the OpenAI shape: `role`, an optional empty `content`, and a
    /// `tool_calls` array where each call's `function.arguments` is a
    /// JSON-encoded string (not an object).
    #[test]
    fn assistant_tool_calls_message_matches_openai_shape() {
        let msg = ChatMessage::assistant_tool_calls(
            "",
            vec![ToolCall {
                id: "call_abc".into(),
                kind: "function".into(),
                function: FunctionCall {
                    name: "fs_list".into(),
                    arguments: "{\"path\":\"/tmp\"}".into(),
                },
            }],
        );
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["content"], "");
        let calls = v["tool_calls"].as_array().expect("tool_calls array");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_abc");
        assert_eq!(calls[0]["type"], "function");
        assert_eq!(calls[0]["function"]["name"], "fs_list");
        // arguments MUST be a JSON-encoded string per the OpenAI spec,
        // not a JSON object — strict servers (incl. OVMS) reject the
        // latter outright.
        assert_eq!(
            calls[0]["function"]["arguments"].as_str(),
            Some("{\"path\":\"/tmp\"}")
        );
    }

    /// `role: "tool"` follow-ups must carry `tool_call_id` and `name` so
    /// the model can correlate the result with its prior `tool_calls`
    /// entry. The body goes in `content` as a plain string.
    #[test]
    fn tool_result_message_matches_openai_shape() {
        let msg = ChatMessage::tool_result("call_abc", "fs_list", "entry-a\nentry-b");
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["role"], "tool");
        assert_eq!(v["tool_call_id"], "call_abc");
        assert_eq!(v["name"], "fs_list");
        assert_eq!(v["content"], "entry-a\nentry-b");
        // Tool messages must NOT carry a tool_calls field of their own.
        assert!(v.as_object().unwrap().get("tool_calls").is_none());
    }

    /// A `tools` array on a request must round-trip into the OpenAI
    /// `[{type:"function", function:{name, description, parameters}}]`
    /// shape so OVMS can hand it to the chat template.
    #[test]
    fn tool_def_serialises_in_openai_function_shape() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            top_p: None,
            top_k: None,
            stream: false,
            tools: Some(vec![ToolDef::function(
                "fs_list",
                "List a directory.",
                serde_json::json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            )]),
            tool_choice: Some("auto".into()),
            chat_template_kwargs: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["tool_choice"], "auto");
        let tools = v["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "fs_list");
        assert_eq!(tools[0]["function"]["description"], "List a directory.");
        assert_eq!(tools[0]["function"]["parameters"]["type"], "object");
    }
}
