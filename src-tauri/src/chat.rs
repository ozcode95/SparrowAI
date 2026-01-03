use serde::{ Deserialize, Serialize };
use tracing::{ error, debug, info };
use serde_json;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;
use std::sync::{ Arc, Mutex };
use tokio::sync::broadcast;
use async_openai::{Client, config::OpenAIConfig};
use async_openai::types::chat::{
    CreateChatCompletionRequestArgs,
    ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestUserMessageArgs,
    ChatCompletionRequestAssistantMessageArgs,
    ChatCompletionRequestUserMessageContent,
    ChatCompletionRequestUserMessageContentPart,
    ChatCompletionRequestMessageContentPartText,
    ChatCompletionRequestMessageContentPartImage,
    ChatCompletionStreamOptions,
    ChatCompletionTool,
    ChatCompletionTools,
    ChatCompletionMessageToolCall,
    ChatCompletionMessageToolCalls,
    ChatCompletionRequestToolMessageArgs,
    CreateChatCompletionStreamResponse,
    ImageUrl,
    ImageDetail,
};
use futures::StreamExt;
use tauri::{ AppHandle, Emitter };
use base64::Engine;

use crate::{ mcp, paths, constants };

// Global state for managing streaming cancellation
lazy_static::lazy_static! {
    static ref ACTIVE_STREAMS: Arc<Mutex<HashMap<String, broadcast::Sender<()>>>> = Arc::new(Mutex::new(HashMap::new()));
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInfo {
    pub file_path: String,
    pub file_name: String,
    pub file_type: String,
    #[serde(default)]
    pub is_image: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String, // "user" or "assistant"
    pub content: String,
    pub timestamp: i64,
    pub tokens_per_second: Option<f64>,
    pub is_error: Option<bool>,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub attachments: Option<Vec<AttachmentInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub model_id: Option<String>,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatSessionsStorage {
    pub sessions: HashMap<String, ChatSession>,
    pub active_session_id: Option<String>,
}

impl Default for ChatSessionsStorage {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            active_session_id: None,
        }
    }
}

fn get_chat_sessions_path() -> Result<PathBuf, String> {
    paths::get_chat_sessions_path().map_err(|e| e.to_string())
}

fn load_chat_sessions() -> Result<ChatSessionsStorage, String> {
    debug!("Loading chat sessions");
    let path = get_chat_sessions_path()?;
    
    info!(path = %path.display(), exists = path.exists(), "Checking chat sessions file");

    if !path.exists() {
        info!("Chat sessions file does not exist, returning empty storage");
        return Ok(ChatSessionsStorage::default());
    }

    let contents = fs
        ::read_to_string(&path)
        .map_err(|e| {
            error!(path = %path.display(), error = %e, "Failed to read chat sessions file");
            format!("Failed to read chat sessions file: {}", e)
        })?;

    info!(path = %path.display(), size = contents.len(), "Chat sessions file read successfully");

    let result = serde_json
        ::from_str::<ChatSessionsStorage>(&contents)
        .map_err(|e| {
            error!(error = %e, content_preview = %&contents[..contents.len().min(200)], "Failed to parse chat sessions");
            format!("Failed to parse chat sessions: {}", e)
        });
    
    match &result {
        Ok(sessions) => {
            info!(
                session_count = sessions.sessions.len(),
                active_session = ?sessions.active_session_id,
                "Chat sessions loaded successfully"
            );
            // Log all session IDs for debugging
            for (id, session) in &sessions.sessions {
                debug!(session_id = %id, title = %session.title, message_count = session.messages.len(), "Loaded session");
            }
        }
        Err(e) => error!(error = %e, "Failed to load chat sessions"),
    }
    result
}

fn save_chat_sessions(storage: &ChatSessionsStorage) -> Result<(), String> {
    debug!(session_count = storage.sessions.len(), "Saving chat sessions");
    let path = get_chat_sessions_path()?;

    info!(path = %path.display(), "Saving chat sessions to file");

    let contents = serde_json
        ::to_string_pretty(storage)
        .map_err(|e| {
            error!(error = %e, "Failed to serialize chat sessions");
            format!("Failed to serialize chat sessions: {}", e)
        })?;

    fs::write(&path, &contents).map_err(|e| {
        error!(path = %path.display(), error = %e, "Failed to write chat sessions file");
        format!("Failed to write chat sessions file: {}", e)
    })?;

    info!(
        path = %path.display(),
        session_count = storage.sessions.len(),
        active_session = ?storage.active_session_id,
        file_size = contents.len(),
        "Chat sessions saved successfully"
    );

    Ok(())
}

fn generate_chat_title(content: &str) -> String {
    // Clean the content and create a meaningful title
    let cleaned = content.trim();

    // Remove common question words and make it more title-like
    let title = if cleaned.len() <= constants::MAX_CHAT_TITLE_LENGTH {
        cleaned.to_string()
    } else {
        // Find a good break point near max length
        let mut break_point = constants::MAX_CHAT_TITLE_LENGTH;
        if let Some(space_pos) = cleaned[..constants::MAX_CHAT_TITLE_LENGTH].rfind(' ') {
            if space_pos > constants::MIN_CHAT_TITLE_LENGTH {
                // Only use space if it's not too early
                break_point = space_pos;
            }
        }
        format!("{}...", &cleaned[..break_point])
    };

    // Capitalize first letter and ensure it doesn't end with punctuation before ellipsis
    // Capitalize first letter and ensure it doesn't end with punctuation before ellipsis
    let mut chars: Vec<char> = title.chars().collect();
    if !chars.is_empty() {
        chars[0] = chars[0].to_uppercase().next().unwrap_or(chars[0]);
    }

    let result: String = chars.into_iter().collect();

    // Clean up any trailing punctuation before "..."
    if result.ends_with("...") {
        let without_ellipsis = &result[..result.len() - 3];
        let trimmed = without_ellipsis.trim_end_matches(['.', ',', '!', '?', ';', ':']);
        format!("{}...", trimmed)
    } else {
        result
    }
}

// Helper function to determine if model should use modern tool format
fn should_use_modern_tool_format(model_name: &str) -> bool {
    // let model_lower = model_name.to_lowercase();
    // model_lower.contains("qwen3") ||
    // model_lower.contains("llama-3.") ||
    // model_lower.contains("hermes-3") ||
    // model_lower.contains("mistral-7b") ||
    // model_lower.contains("phi-4") ||
    // model_lower.contains("gpt-oss")
    false // Placeholder until more models are supported
}

// Helper function to build system message with or without tool info
async fn build_system_message(
    app: AppHandle,
    system_prompt: Option<String>,
    use_modern_format: bool
) -> Result<(String, Vec<ChatCompletionTool>), String> {
    // Get MCP tools
    let mcp_tools = match mcp::get_all_mcp_tools_for_chat(app.clone()).await {
        Ok(tools) => {
            tracing::debug!(count = tools.len(), "Loaded MCP tools for chat");
            if tools.is_empty() {
                log_warning!("No MCP tools available", note = "LLM will not have access to any tools");
            } else {
                tracing::debug!(tools = ?tools.iter().map(|t| &t.function.name).collect::<Vec<_>>(), "Available MCP tools");
            }
            tools
        }
        Err(e) => {
            log_warning!("Failed to load MCP tools", error = %e);
            Vec::new()
        }
    };

    let base_system_message = system_prompt.unwrap_or_else(|| {
        "You are a helpful AI assistant with access to various functions/tools.

        Tool Usage Guidelines:
        - Use tools ONLY when they are necessary to answer the user's question
        - For simple greetings, general questions, or conversations, respond naturally WITHOUT using tools
        - Only call a tool if the user's request specifically requires information or actions that the tool provides
        - Examples of when NOT to use tools: greetings (hello, hi), general chat, opinions, explanations
        - Examples of when to use tools: getting current time, converting units, fetching specific data
        
        When a tool would be helpful, use it. Otherwise, respond conversationally.".to_string()
    });

    if use_modern_format {
        // Modern format: tools go in the request, minimal system message
        tracing::debug!("Using modern tool format (tools in request)");
        Ok((base_system_message, mcp_tools))
    } else {
        // Legacy format: tools in system message as XML template
        let tools_info = if !mcp_tools.is_empty() {
            tracing::debug!("Processing MCP tools for system message...");

            let tool_descs: Vec<String> = mcp_tools
                .iter()
                .enumerate()
                .map(|(i, tool)| {
                    tracing::trace!(index = i, name = %tool.function.name, "Processing tool");
                    let params_str = match &tool.function.parameters {
                        Some(params) => serde_json::to_string_pretty(params).unwrap_or_default(),
                        None => "{}".to_string(),
                    };

                    format!(
                        "{}({}) - {}",
                        tool.function.name,
                        params_str,
                        tool.function.description.as_ref().unwrap_or(&"".to_string())
                    )
                })
                .collect();

            let tool_descs_text = tool_descs.join("\n");
            let formatted_tools = format!(r#"

# Tools

You may call one or more functions to assist with the user query.

You are provided with function signatures within <tools></tools> XML tags:
<tools>
{}
</tools>

For each function call, return a json object with function name and arguments within <tool_call></tool_call> XML tags:
<tool_call>
{{"name": <function-name>, "arguments": <args-json-object>}}
</tool_call>"#, tool_descs_text);

            tracing::debug!(length = formatted_tools.len(), "Generated custom tool template");
            formatted_tools
        } else {
            tracing::trace!("No MCP tools available for system message");
            "".to_string()
        };

        let system_message = format!("{}{}", base_system_message, tools_info);
        tracing::debug!(
            length = system_message.len(),
            has_tools = !tools_info.is_empty(),
            tools_count = mcp_tools.len(),
            "System message prepared with XML tool format"
        );
        
        Ok((system_message, Vec::new())) // Return empty tools vec for legacy format
    }
}

// Helper function to build messages array including history
async fn build_messages_array(
    session_id: Option<String>,
    include_history: Option<bool>,
    system_message: String,
    user_message: String,
    attachments: Option<Vec<AttachmentInfo>>
) -> Result<Vec<ChatCompletionRequestMessage>, String> {
    let mut messages = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content(system_message.clone())
            .build()
            .map_err(|e| format!("Failed to build system message: {}", e))?
            .into()
    ];

    // Include conversation history if requested
    if include_history.unwrap_or(false) && session_id.is_some() {
        match get_conversation_history(session_id.clone().unwrap()).await {
            Ok(mut history) => {
                if let Some(last_msg) = history.last() {
                    if last_msg.role == "user" && last_msg.content == user_message {
                        history.pop();
                    }
                }

                tracing::debug!(
                    history_count = history.len(),
                    session_id = session_id.clone().unwrap(),
                    "Including conversation history"
                );

                for msg in history {
                    match msg.role.as_str() {
                        "user" => {
                            messages.push(
                                ChatCompletionRequestUserMessageArgs::default()
                                    .content(msg.content.clone())
                                    .build()
                                    .map_err(|e| format!("Failed to build user message: {}", e))?
                                    .into()
                            );
                        }
                        "assistant" => {
                            let cleaned_content = strip_tool_xml_tags(&msg.content);
                            
                            if !cleaned_content.is_empty() {
                                messages.push(
                                    ChatCompletionRequestAssistantMessageArgs::default()
                                        .content(cleaned_content)
                                        .build()
                                        .map_err(|e|
                                            format!("Failed to build assistant message: {}", e)
                                        )?
                                        .into()
                                );
                            }
                        }
                        _ => {
                            log_warning!("Skipping message with unknown role", role = %msg.role);
                            continue;
                        }
                    };
                }
            }
            Err(e) => {
                log_operation_error!("Load conversation history", &e);
            }
        }
    }

    // Build current user message with potential attachments
    let user_message_content = build_user_message_content(&user_message, attachments)?;
    messages.push(
        ChatCompletionRequestUserMessageArgs::default()
            .content(user_message_content)
            .build()
            .map_err(|e| format!("Failed to build user message: {}", e))?
            .into()
    );

    tracing::debug!(
        total_messages = messages.len(),
        "Messages array prepared"
    );

    Ok(messages)
}

// Helper function to build user message content with attachments
fn build_user_message_content(
    message: &str,
    attachments: Option<Vec<AttachmentInfo>>
) -> Result<ChatCompletionRequestUserMessageContent, String> {
    if let Some(ref attachment_list) = attachments {
        let image_attachments: Vec<&AttachmentInfo> = attachment_list
            .iter()
            .filter(|a| a.is_image)
            .collect();

        if !image_attachments.is_empty() {
            tracing::debug!(
                image_count = image_attachments.len(),
                "Building multimodal message with images"
            );

            let mut content_parts: Vec<ChatCompletionRequestUserMessageContentPart> = vec![
                ChatCompletionRequestMessageContentPartText {
                    text: message.to_string(),
                }.into()
            ];

            for img_attachment in image_attachments {
                match fs::read(&img_attachment.file_path) {
                    Ok(image_data) => {
                        let base64_data = base64::engine::general_purpose::STANDARD.encode(&image_data);
                        
                        let mime_type = match img_attachment.file_type.to_lowercase().as_str() {
                            "png" => "image/png",
                            "jpg" | "jpeg" => "image/jpeg",
                            "gif" => "image/gif",
                            "webp" => "image/webp",
                            _ => "image/jpeg",
                        };
                        
                        let data_url = format!("data:{};base64,{}", mime_type, base64_data);
                        
                        tracing::debug!(
                            file_name = %img_attachment.file_name,
                            data_url_length = data_url.len(),
                            "Added image to message"
                        );

                        content_parts.push(
                            ChatCompletionRequestMessageContentPartImage {
                                image_url: ImageUrl {
                                    url: data_url,
                                    detail: Some(ImageDetail::Auto),
                                }
                            }.into()
                        );
                    }
                    Err(e) => {
                        log_warning!(
                            "Failed to read image file",
                            error = %e,
                            file_path = %img_attachment.file_path
                        );
                    }
                }
            }

            return Ok(ChatCompletionRequestUserMessageContent::Array(content_parts));
        }
    }

    Ok(ChatCompletionRequestUserMessageContent::Text(message.to_string()))
}

// Process streaming response and handle tool calls (both modern and XML format)
async fn process_stream_with_tool_calls(
    app: AppHandle,
    client: &Client<OpenAIConfig>,
    mut stream: impl futures::Stream<Item = Result<CreateChatCompletionStreamResponse, async_openai::error::OpenAIError>> + Unpin,
    stream_id: String,
    cancel_rx: &mut broadcast::Receiver<()>,
    use_modern_format: bool,
    system_message: &str,
    previous_messages: &[ChatCompletionRequestMessage],
    model_name: &str,
    temperature: Option<f64>,
    top_p: Option<f64>,
    seed: Option<i64>,
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>
) -> Result<(String, Option<(u32, u32, u32)>, bool), String> {
    let mut full_response = String::new();
    let mut executed_tools = std::collections::HashSet::new();
    let mut needs_continuation = false;
    let mut usage_data: Option<(u32, u32, u32)> = None;
    let mut was_cancelled = false;
    
    // For modern tool format
    let mut modern_tool_calls: Vec<ChatCompletionMessageToolCall> = Vec::new();

    loop {
        tokio::select! {
            _ = cancel_rx.recv() => {
                info!(stream_id = %stream_id, "Stream cancelled by user");
                was_cancelled = true;
                break;
            }
            result = stream.next() => {
                match result {
                    None => break,
                    Some(stream_result) => match stream_result {
                        Ok(response) => {
                            if let Some(usage) = response.usage {
                                let prompt_tokens = usage.prompt_tokens;
                                let completion_tokens = usage.completion_tokens;
                                let total_tokens = usage.total_tokens;
                                
                                usage_data = Some((prompt_tokens, completion_tokens, total_tokens));
                                
                                info!(
                                    prompt_tokens = prompt_tokens,
                                    completion_tokens = completion_tokens,
                                    total_tokens = total_tokens,
                                    "✅ Captured usage statistics from final stream chunk"
                                );
                            }

                            for chat_choice in response.choices {
                                // Handle modern tool calls
                                if use_modern_format {
                                    if let Some(tool_call_chunks) = chat_choice.delta.tool_calls {
                                        for chunk in tool_call_chunks {
                                            let index = chunk.index as usize;
                                            
                                            while modern_tool_calls.len() <= index {
                                                modern_tool_calls.push(ChatCompletionMessageToolCall {
                                                    id: String::new(),
                                                    function: Default::default(),
                                                });
                                            }
                                            
                                            let tool_call = &mut modern_tool_calls[index];
                                            if let Some(id) = chunk.id {
                                                tool_call.id = id;
                                            }
                                            if let Some(function_chunk) = chunk.function {
                                                if let Some(name) = function_chunk.name {
                                                    tool_call.function.name = name;
                                                }
                                                if let Some(args) = function_chunk.arguments {
                                                    tool_call.function.arguments.push_str(&args);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Handle content (for both formats)
                                if let Some(content) = &chat_choice.delta.content {
                                    full_response.push_str(content);

                                    let _ = app.emit(
                                        "chat-token",
                                        serde_json::json!({
                                            "token": content,
                                            "finished": false
                                        })
                                    );

                                    // For XML format, process tool calls
                                    if !use_modern_format {
                                        let tool_calls = extract_all_tool_calls_from_xml(&full_response);

                                        for (fn_name, fn_args) in tool_calls {
                                            let tool_signature = format!("{}:{}", fn_name, fn_args);
                                            if executed_tools.contains(&tool_signature) {
                                                continue;
                                            }

                                            executed_tools.insert(tool_signature);
                                            tracing::debug!(name = %fn_name, args = %fn_args, "Found XML tool call");

                                            let tool_result = execute_tool_call(
                                                app.clone(),
                                                &fn_name,
                                                &fn_args
                                            ).await;

                                            handle_tool_result(
                                                app.clone(),
                                                &fn_name,
                                                &fn_args,
                                                tool_result,
                                                &mut full_response,
                                                &mut needs_continuation
                                            )?;
                                        }
                                    }
                                }

                                if let Some(_finish_reason) = &chat_choice.finish_reason {
                                    tracing::debug!(reason = ?_finish_reason, "Stream finished");

                                    if !use_modern_format && has_incomplete_tool_call(&full_response) {
                                        log_warning!("Stream ended with incomplete tool call", note = "response may be truncated");
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            log_operation_error!("Chat stream", &err);
                            let _ = app.emit(
                                "chat-error",
                                serde_json::json!({
                                    "error": format!("Stream error: {}", err)
                                })
                            );
                            break;
                        }
                    }
                }
            }
        }
    }

    // Execute modern tool calls after stream completes
    if use_modern_format && !modern_tool_calls.is_empty() {
        tracing::debug!(count = modern_tool_calls.len(), "Processing modern tool calls");
        
        let continuation_result = execute_modern_tool_calls_and_continue(
            app.clone(),
            client,
            modern_tool_calls,
            previous_messages,
            model_name,
            temperature,
            top_p,
            seed,
            max_tokens,
            max_completion_tokens
        ).await?;
        
        full_response.push_str(&continuation_result);
    } else if !use_modern_format && needs_continuation {
        // Handle XML format continuation
        let should_continue = check_if_continuation_needed(&full_response);

        if should_continue {
            tracing::debug!("Tool response contains JSON - continuing conversation...");

            match
                continue_conversation_after_tools(
                    app.clone(),
                    client,
                    system_message,
                    previous_messages,
                    full_response.clone(),
                    model_name,
                    temperature,
                    top_p,
                    seed,
                    max_tokens,
                    max_completion_tokens
                ).await
            {
                Ok(continued_response) => {
                    if !continued_response.trim().is_empty() {
                        full_response.push_str(&continued_response);
                    }
                }
                Err(e) => {
                    error!("Failed to continue conversation: {}", e);
                    let error_msg = format!("\n\n[Continuation Error: {}]", e);
                    full_response.push_str(&error_msg);

                    let _ = app.emit(
                        "chat-token",
                        serde_json::json!({
                            "token": error_msg,
                            "finished": false
                        })
                    );
                }
            }
        }
    }

    Ok((full_response, usage_data, was_cancelled))
}

// Execute a single tool call
async fn execute_tool_call(
    app: AppHandle,
    fn_name: &str,
    fn_args: &str
) -> Result<String, String> {
    let args_map = match serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(fn_args) {
        Ok(mut map) => {
            map.retain(|_k, v| !v.is_null());
            Some(map)
        }
        Err(e) => {
            log_warning!("Failed to parse tool arguments", error = %e, args = %fn_args);
            None
        }
    };

    mcp::call_mcp_tool(app, fn_name.to_string(), args_map).await
}

// Handle tool execution result
fn handle_tool_result(
    app: AppHandle,
    fn_name: &str,
    fn_args: &str,
    tool_result: Result<String, String>,
    full_response: &mut String,
    needs_continuation: &mut bool
) -> Result<(), String> {
    match tool_result {
        Ok(result) => {
            tracing::debug!(tool = %fn_name, result_length = result.len(), "Tool execution completed");
            tracing::trace!(result = %result, "Tool result content");

            let _ = app.emit(
                "tool-call",
                serde_json::json!({
                    "tool_name": fn_name,
                    "arguments": fn_args,
                    "result": result
                })
            );

            let tool_response_text = format!("\n<tool_response>\n{}\n</tool_response>", result);
            full_response.push_str(&tool_response_text);

            let _ = app.emit(
                "chat-token",
                serde_json::json!({
                    "token": tool_response_text,
                    "finished": false
                })
            );

            *needs_continuation = true;
        }
        Err(e) => {
            log_operation_error!("Tool execution", &e, tool = %fn_name);
            let error_response_text = format!("\n<tool_response>\nError: {}\n</tool_response>", e);
            full_response.push_str(&error_response_text);

            let _ = app.emit(
                "chat-token",
                serde_json::json!({
                    "token": error_response_text,
                    "finished": false
                })
            );

            *needs_continuation = true;
        }
    }
    
    Ok(())
}

// Execute modern tool calls and continue conversation
async fn execute_modern_tool_calls_and_continue(
    app: AppHandle,
    client: &Client<OpenAIConfig>,
    tool_calls: Vec<ChatCompletionMessageToolCall>,
    previous_messages: &[ChatCompletionRequestMessage],
    model_name: &str,
    temperature: Option<f64>,
    top_p: Option<f64>,
    seed: Option<i64>,
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>
) -> Result<String, String> {
    let mut messages = previous_messages.to_vec();
    let mut tool_responses = Vec::new();

    // Execute each tool call
    for tool_call in &tool_calls {
        tracing::debug!(
            tool_name = %tool_call.function.name,
            tool_id = %tool_call.id,
            "Executing modern tool call"
        );

        let result = execute_tool_call(
            app.clone(),
            &tool_call.function.name,
            &tool_call.function.arguments
        ).await;

        let response_content = match result {
            Ok(content) => content,
            Err(e) => format!("Error: {}", e),
        };

        // Emit tool call event
        let _ = app.emit(
            "tool-call",
            serde_json::json!({
                "tool_name": tool_call.function.name,
                "arguments": tool_call.function.arguments,
                "result": response_content
            })
        );

        tool_responses.push((tool_call.id.clone(), response_content));
    }

    // Add assistant message with tool calls
    let tool_calls_converted: Vec<ChatCompletionMessageToolCalls> = tool_calls
        .iter()
        .map(|tc| tc.clone().into())
        .collect();
    
    messages.push(
        ChatCompletionRequestAssistantMessageArgs::default()
            .tool_calls(tool_calls_converted)
            .build()
            .map_err(|e| format!("Failed to build assistant message: {}", e))?
            .into()
    );

    // Add tool response messages
    for (tool_call_id, response) in tool_responses {
        messages.push(
            ChatCompletionRequestToolMessageArgs::default()
                .content(response)
                .tool_call_id(tool_call_id)
                .build()
                .map_err(|e| format!("Failed to build tool message: {}", e))?
                .into()
        );
    }

    // Create follow-up request
    let mut request_builder = CreateChatCompletionRequestArgs::default();
    request_builder
        .model(model_name.to_string())
        .messages(messages)
        .stream(true)
        .temperature(temperature.unwrap_or(0.7) as f32)
        .top_p(top_p.unwrap_or(1.0) as f32);

    if let Some(seed) = seed {
        request_builder.seed(seed);
    }

    let effective_max_tokens = max_tokens.unwrap_or(1000).max(100);
    request_builder.max_tokens(effective_max_tokens);

    if let Some(max_completion_tokens) = max_completion_tokens {
        request_builder.max_completion_tokens(max_completion_tokens);
    }

    let request = request_builder
        .build()
        .map_err(|e| format!("Failed to build follow-up request: {}", e))?;

    let mut stream = client
        .chat()
        .create_stream(request).await
        .map_err(|e| format!("Failed to create follow-up stream: {}", e))?;

    let mut continued_response = String::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                for chat_choice in response.choices {
                    if let Some(content) = &chat_choice.delta.content {
                        continued_response.push_str(content);

                        let _ = app.emit(
                            "chat-token",
                            serde_json::json!({
                                "token": content,
                                "finished": false
                            })
                        );
                    }
                }
            }
            Err(err) => {
                log_operation_error!("Follow-up stream", &err);
                return Err(format!("Follow-up stream error: {}", err));
            }
        }
    }

    Ok(continued_response)
}

#[tauri::command]
pub async fn get_chat_sessions() -> Result<ChatSessionsStorage, String> {
    load_chat_sessions()
}

#[tauri::command]
pub async fn create_chat_session(title: Option<String>) -> Result<ChatSession, String> {
    let session_title = title.clone().unwrap_or_else(|| constants::DEFAULT_CHAT_TITLE.to_string());
    log_operation_start!("Creating chat session", title = %session_title);
    
    let mut storage = load_chat_sessions()?;

    let session_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    let session = ChatSession {
        id: session_id.clone(),
        title: session_title,
        created_at: now,
        updated_at: now,
        model_id: None,
        messages: Vec::new(),
    };

    log_debug_details!(
        session_id = %session_id,
        title = %session.title,
        "Chat session created"
    );
    
    storage.sessions.insert(session_id.clone(), session.clone());
    storage.active_session_id = Some(session_id.clone());

    save_chat_sessions(&storage)?;
    log_operation_success!("Chat session created", session_id = %session_id);

    Ok(session)
}

#[tauri::command]
pub async fn create_temporary_chat_session(title: Option<String>) -> Result<ChatSession, String> {
    let session_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    let session = ChatSession {
        id: session_id.clone(),
        title: title.unwrap_or_else(|| constants::DEFAULT_CHAT_TITLE.to_string()),
        created_at: now,
        updated_at: now,
        model_id: None,
        messages: Vec::new(),
    };

    // Don't save to storage yet - this is a temporary session
    Ok(session)
}

#[tauri::command]
pub async fn update_chat_session(
    session_id: String,
    title: Option<String>,
    model_id: Option<String>
) -> Result<ChatSession, String> {
    let mut storage = load_chat_sessions()?;

    let session = storage.sessions
        .get_mut(&session_id)
        .ok_or_else(|| format!("Chat session not found: {}", session_id))?;

    if let Some(new_title) = title {
        session.title = new_title;
    }

    if let Some(new_model_id) = model_id {
        session.model_id = Some(new_model_id);
    }

    session.updated_at = chrono::Utc::now().timestamp_millis();

    let updated_session = session.clone();
    save_chat_sessions(&storage)?;

    Ok(updated_session)
}

#[tauri::command]
pub async fn delete_chat_session(session_id: String) -> Result<String, String> {
    let mut storage = load_chat_sessions()?;

    if !storage.sessions.contains_key(&session_id) {
        return Err(format!("Chat session not found: {}", session_id));
    }

    storage.sessions.remove(&session_id);

    // If this was the active session, clear it
    if storage.active_session_id.as_ref() == Some(&session_id) {
        storage.active_session_id = None;
    }

    save_chat_sessions(&storage)?;

    Ok(format!("Chat session deleted: {}", session_id))
}

#[tauri::command]
pub async fn set_active_chat_session(session_id: String) -> Result<String, String> {
    let mut storage = load_chat_sessions()?;

    if !storage.sessions.contains_key(&session_id) {
        return Err(format!("Chat session not found: {}", session_id));
    }

    storage.active_session_id = Some(session_id.clone());
    save_chat_sessions(&storage)?;

    Ok(session_id)
}

#[tauri::command]
pub async fn add_message_to_session(
    session_id: String,
    role: String,
    content: String,
    tokens_per_second: Option<f64>,
    is_error: Option<bool>,
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
    attachments: Option<Vec<AttachmentInfo>>
) -> Result<ChatMessage, String> {
    tracing::debug!(
        session_id = %session_id,
        role = %role,
        content_length = content.len(),
        tokens_per_second = ?tokens_per_second,
        has_attachments = attachments.is_some(),
        "Adding message to session"
    );
    
    let mut storage = load_chat_sessions()?;

    let session = storage.sessions
        .get_mut(&session_id)
        .ok_or_else(|| {
            log_operation_error!("Add message to session", "Session not found", session_id = %session_id);
            format!("Chat session not found: {}", session_id)
        })?;

    let message_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    let message = ChatMessage {
        id: message_id.clone(),
        role: role.clone(),
        content: content.clone(),
        timestamp: now,
        tokens_per_second,
        is_error,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        attachments,
    };

    session.messages.push(message.clone());
    session.updated_at = now;

    // Auto-generate title from first user message if still "New Chat"
    let auto_generated_title = if session.title == "New Chat" && role == "user" {
        let title = generate_chat_title(&content);
        tracing::debug!(
            session_id = %session_id,
            old_title = "New Chat",
            new_title = %title,
            "Auto-generated session title"
        );
        session.title = title.clone();
        Some(title)
    } else {
        None
    };

    let message_count = session.messages.len();

    save_chat_sessions(&storage)?;
    info!(
        session_id = %session_id,
        message_id = %message_id,
        role = %role,
        message_count = message_count,
        auto_title = ?auto_generated_title,
        "Message added and session saved"
    );

    Ok(message)
}

#[tauri::command]
pub async fn persist_temporary_session(session: ChatSession) -> Result<ChatSession, String> {
    let mut storage = load_chat_sessions()?;

    storage.sessions.insert(session.id.clone(), session.clone());
    storage.active_session_id = Some(session.id.clone());

    save_chat_sessions(&storage)?;

    Ok(session)
}

#[tauri::command]
pub async fn add_message_to_temporary_session(
    mut session: ChatSession,
    role: String,
    content: String,
    tokens_per_second: Option<f64>,
    is_error: Option<bool>,
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
    attachments: Option<Vec<AttachmentInfo>>
) -> Result<(ChatSession, ChatMessage), String> {
    let message_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    let message = ChatMessage {
        id: message_id,
        role: role.clone(),
        content: content.clone(),
        timestamp: now,
        tokens_per_second,
        is_error,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        attachments,
    };

    session.messages.push(message.clone());
    session.updated_at = now;

    // Auto-generate title from first user message if still "New Chat"
    if session.title == "New Chat" && role == "user" {
        let title = generate_chat_title(&content);
        session.title = title;
    }

    Ok((session, message))
}

#[tauri::command]
pub async fn get_session_messages(session_id: String) -> Result<Vec<ChatMessage>, String> {
    let storage = load_chat_sessions()?;

    let session = storage.sessions
        .get(&session_id)
        .ok_or_else(|| format!("Chat session not found: {}", session_id))?;

    Ok(session.messages.clone())
}

#[tauri::command]
pub async fn stop_chat_streaming(session_id: String) -> Result<String, String> {
    info!(session_id = %session_id, "Attempting to stop chat streaming");
    
    let mut streams = ACTIVE_STREAMS.lock().map_err(|e| format!("Failed to acquire lock: {}", e))?;
    
    if let Some(sender) = streams.remove(&session_id) {
        // Send cancellation signal
        let _ = sender.send(());
        info!(session_id = %session_id, "Streaming cancellation signal sent");
        Ok(format!("Streaming stopped for session: {}", session_id))
    } else {
        log_warning!("No active stream found", session_id = %session_id);
        Err(format!("No active streaming session found: {}", session_id))
    }
}

#[tauri::command]
pub async fn get_conversation_history(session_id: String) -> Result<Vec<ChatMessage>, String> {
    let storage = load_chat_sessions()?;

    let session = storage.sessions
        .get(&session_id)
        .ok_or_else(|| format!("Chat session not found: {}", session_id))?;

    // Return all messages except any currently streaming ones
    let messages: Vec<ChatMessage> = session.messages
        .iter()
        .filter(|msg| msg.role == "user" || msg.role == "assistant")
        .cloned()
        .collect();

    Ok(messages)
}

// Chat with the currently loaded model using streaming
#[tauri::command]
pub async fn chat_with_loaded_model_streaming(
    app: AppHandle,
    model_name: String,
    message: String,
    session_id: Option<String>,
    include_history: Option<bool>,
    system_prompt: Option<String>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    seed: Option<i64>,
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>,
    attachments: Option<Vec<AttachmentInfo>>
) -> Result<String, String> {
    log_operation_start!("Chat request");
    tracing::debug!(model = %model_name, message_length = message.len(), "Chat parameters");

    let config = OpenAIConfig::new()
        .with_api_key("unused")
        .with_api_base("http://localhost:1114/v3");
    let client = Client::with_config(config);

    // Determine tool format based on model
    let use_modern_format = should_use_modern_tool_format(&model_name);
    tracing::debug!(
        model = %model_name,
        use_modern_format = use_modern_format,
        "Determined tool format"
    );

    // Build system message and get tools
    let (system_message, tools) = build_system_message(
        app.clone(),
        system_prompt,
        use_modern_format
    ).await?;

    // Build messages array
    let messages = build_messages_array(
        session_id.clone(),
        include_history,
        system_message.clone(),
        message.clone(),
        attachments
    ).await?;

    // Build request
    let mut request_builder = CreateChatCompletionRequestArgs::default();
    request_builder
        .model(model_name.clone())
        .messages(messages.clone())
        .stream(true)
        .stream_options(ChatCompletionStreamOptions {
            include_usage: Some(true),
            include_obfuscation: None,
        })
        .temperature(temperature.unwrap_or(0.7) as f32)
        .top_p(top_p.unwrap_or(1.0) as f32);

    if let Some(seed) = seed {
        request_builder.seed(seed);
    }

    let effective_max_tokens = max_tokens.unwrap_or(1000).max(100);
    request_builder.max_tokens(effective_max_tokens);

    if let Some(max_completion_tokens) = max_completion_tokens {
        request_builder.max_completion_tokens(max_completion_tokens);
    }

    // Add tools if using modern format
    if use_modern_format && !tools.is_empty() {
        tracing::debug!(tool_count = tools.len(), "Adding tools to request (modern format)");
        let tools_converted: Vec<ChatCompletionTools> = tools
            .into_iter()
            .map(|t| ChatCompletionTools::Function(t))
            .collect();
        request_builder.tools(tools_converted);
    }

    let request = request_builder
        .build()
        .map_err(|e| {
            log_operation_error!("Build chat request", &e);
            format!("Failed to build chat request: {}", e)
        })?;

    let stream = client
        .chat()
        .create_stream(request).await
        .map_err(|e| {
            log_operation_error!("Create chat stream", &e);
            format!("Failed to create chat stream: {}", e)
        })?;

    // Setup cancellation channel
    let (cancel_tx, mut cancel_rx) = broadcast::channel::<()>(1);
    let stream_id = session_id.clone().unwrap_or_else(|| "temp".to_string());
    
    {
        let mut streams = ACTIVE_STREAMS.lock().map_err(|e| format!("Failed to acquire lock: {}", e))?;
        streams.insert(stream_id.clone(), cancel_tx);
    }

    // Process stream with tool calls
    let (full_response, usage_data, was_cancelled) = process_stream_with_tool_calls(
        app.clone(),
        &client,
        stream,
        stream_id.clone(),
        &mut cancel_rx,
        use_modern_format,
        &system_message,
        &messages,
        &model_name,
        temperature,
        top_p,
        seed,
        max_tokens,
        max_completion_tokens
    ).await?;

    // Cleanup
    {
        let mut streams = ACTIVE_STREAMS.lock().map_err(|e| format!("Failed to acquire lock: {}", e))?;
        streams.remove(&stream_id);
    }

    // Emit completion signal
    let _ = app.emit(
        "chat-token",
        serde_json::json!({
            "token": "",
            "finished": true,
            "cancelled": was_cancelled,
            "usage": usage_data.map(|(prompt, completion, total)| {
                serde_json::json!({
                    "prompt_tokens": prompt,
                    "completion_tokens": completion,
                    "total_tokens": total
                })
            })
        })
    );

    if let Some((prompt_tokens, completion_tokens, total_tokens)) = usage_data {
        let _ = app.emit(
            "chat-usage",
            serde_json::json!({
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": total_tokens
            })
        );
    }

    debug!(
        message_length = full_response.len(),
        session_id = ?session_id,
        model = %model_name,
        "Chat completion finished"
    );

    if full_response.len() > 500 {
        info!(
            response_preview = %&full_response[..500],
            total_length = full_response.len(),
            "Full chat response (truncated)"
        );
    } else {
        info!(
            full_response = %full_response,
            "Complete chat response"
        );
    }

    Ok(full_response)
}

async fn continue_conversation_after_tools(
    app: AppHandle,
    client: &Client<OpenAIConfig>,
    _system_message: &str,
    previous_messages: &[ChatCompletionRequestMessage],
    assistant_response_with_tools: String,
    model_name: &str,
    temperature: Option<f64>,
    top_p: Option<f64>,
    seed: Option<i64>,
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>
) -> Result<String, String> {
    debug!("Continuing conversation after tool execution");

    // Create a clean system message for continuation - completely remove tool formatting
    let continuation_system_message = 
        "You are a helpful AI assistant. A tool has just been executed and returned results. \
        Your task is to interpret the JSON results and provide a clear, natural language response to the user.\n\n\
        CRITICAL: Respond ONLY with plain text. Do NOT use any XML tags like <tool_call> or <tool_response>. \
        Do NOT wrap your response in any special formatting. Just provide a natural, conversational answer.".to_string();

    debug!("Continuation system message (length: {})", continuation_system_message.len());

    // Extract just the tool response content (the JSON data) without XML tags
    // This prevents the LLM from seeing the XML format and copying it
    let clean_tool_response = extract_tool_response_content(&assistant_response_with_tools);
    
    tracing::trace!(length = clean_tool_response.len(), "Clean tool response for continuation");

    // Build a minimal message list for continuation - ONLY system, user question, and tool result
    // Do NOT include conversation history to avoid the LLM seeing XML patterns
    let mut continuation_messages = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content(continuation_system_message)
            .build()
            .map_err(|e| format!("Failed to build system message: {}", e))?
            .into()
    ];

    // Find the last user message (the current question)
    if let Some(last_user_msg) = previous_messages.iter().rev().find(|msg| {
        // Check if this is a user message by inspecting the message structure
        if let Ok(value) = serde_json::to_value(msg) {
            if let Some(role) = value.get("role") {
                return role.as_str() == Some("user");
            }
        }
        false
    }) {
        // Add only the current user's question
        continuation_messages.push(last_user_msg.clone());
    } else {
        log_warning!("Could not find user message in history", note = "continuation may lack context");
    }

    // Add a simplified assistant message showing the tool result without XML formatting
    continuation_messages.push(
        ChatCompletionRequestAssistantMessageArgs::default()
            .content(format!("I called a tool and received this result:\n{}", clean_tool_response))
            .build()
            .map_err(|e| format!("Failed to build assistant message with tools: {}", e))?
            .into()
    );

    // Create a new streaming request to continue the conversation
    let mut request_builder = CreateChatCompletionRequestArgs::default();
    request_builder
        .model(model_name.to_string())
        .messages(continuation_messages)
        .stream(true)
        .temperature(temperature.unwrap_or(0.7) as f32)
        .top_p(top_p.unwrap_or(1.0) as f32);

    if let Some(seed) = seed {
        request_builder.seed(seed);
    }

    let effective_max_tokens = max_tokens.unwrap_or(1000).max(100);
    request_builder.max_tokens(effective_max_tokens);

    if let Some(max_completion_tokens) = max_completion_tokens {
        request_builder.max_completion_tokens(max_completion_tokens);
    }

    let request = request_builder
        .build()
        .map_err(|e| format!("Failed to build continuation request: {}", e))?;

    debug!("Sending continuation request...");

    let mut stream = client
        .chat()
        .create_stream(request).await
        .map_err(|e| format!("Failed to create continuation stream: {}", e))?;

    let mut continued_response = String::new();

    // Process the continuation stream
    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                for chat_choice in response.choices {
                    if let Some(content) = &chat_choice.delta.content {
                        continued_response.push_str(content);

                        // Emit streaming content for continuation
                        let _ = app.emit(
                            "chat-token",
                            serde_json::json!({
                                "token": content,
                                "finished": false
                            })
                        );
                    }

                    if let Some(finish_reason) = &chat_choice.finish_reason {
                        tracing::debug!(reason = ?finish_reason, "Continuation finished");
                        break;
                    }
                }
            }
            Err(err) => {
                log_operation_error!("Continuation stream", &err);
                return Err(format!("Continuation stream error: {}", err));
            }
        }
    }

    tracing::debug!(length = continued_response.len(), "Continuation response completed");
    Ok(continued_response)
}

// RAG-enhanced chat with streaming
#[tauri::command]
pub async fn chat_with_rag_streaming(
    app: AppHandle,
    model_name: String,
    message: String,
    session_id: Option<String>,
    include_history: Option<bool>,
    system_prompt: Option<String>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    seed: Option<i64>,
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>,
    use_rag: Option<bool>,
    rag_limit: Option<usize>,
    attachments: Option<Vec<AttachmentInfo>>
) -> Result<String, String> {
    let mut context_content = String::new();

    // Separate images from documents
    let (_image_attachments, document_attachments): (Vec<AttachmentInfo>, Vec<AttachmentInfo>) = 
        if let Some(attachment_list) = attachments.as_ref() {
            let mut images = Vec::new();
            let mut docs = Vec::new();
            for attachment in attachment_list {
                if attachment.is_image {
                    images.push(attachment.clone());
                } else {
                    docs.push(attachment.clone());
                }
            }
            (images, docs)
        } else {
            (Vec::new(), Vec::new())
        };

    // Extract document file paths for RAG (excluding images)
    let doc_file_paths: Option<Vec<String>> = if !document_attachments.is_empty() {
        Some(document_attachments.iter().map(|a| a.file_path.clone()).collect())
    } else {
        None
    };

    // RAG retrieval if enabled OR if there are document attachments (not images)
    let should_use_rag = use_rag.unwrap_or(false) || doc_file_paths.is_some();
    
    if should_use_rag {
        tracing::info!(
            has_attached_files = doc_file_paths.is_some(), 
            attached_count = doc_file_paths.as_ref().map(|f| f.len()),
            "RAG is enabled, performing document retrieval"
        );
        match perform_rag_retrieval(&message, rag_limit, doc_file_paths.as_ref()).await {
            Ok(context) => {
                if !context.is_empty() {
                    tracing::info!(context_length = context.len(), "RAG context retrieved successfully");
                    context_content = context;
                } else {
                    tracing::warn!("RAG retrieval returned empty context - no relevant documents found");
                }
            }
            Err(e) => {
                error!(error = %e, "RAG retrieval failed");
                // Continue without RAG context rather than failing completely
            }
        }
    } else {
        tracing::debug!("RAG is disabled for this request");
    }

    // Enhanced system prompt with context
    let enhanced_system_prompt = if !context_content.is_empty() {
        let prompt = format!(
            "You are a helpful AI assistant with access to document content. CRITICAL INSTRUCTIONS:\n\
            - You MUST use the document excerpts provided below to answer questions\n\
            - Quote specific details from the documents when relevant\n\
            - If information is in the documents, cite it explicitly (e.g., \"According to Source 1...\")\n\
            - If the answer isn't in the provided excerpts, say so clearly\n\
            - DO NOT claim you cannot analyze documents - the content is right here\n\
            - Synthesize information across multiple sources when needed\n\n\
            DOCUMENT EXCERPTS:\n\
            {}\n\n\
            Answer the user's question using the above document content. Be specific and cite your sources.",
            context_content
        );
        tracing::info!(prompt_length = prompt.len(), has_context = true, "Enhanced system prompt with RAG context");
        prompt
    } else {
        let prompt = system_prompt.unwrap_or_else(||
            "You're an AI assistant that provides helpful responses.".to_string()
        );
        tracing::debug!(has_context = false, "Using standard system prompt without RAG");
        prompt
    };

    // Use existing chat function with enhanced prompt
    // Pass the full attachments list (including images) to the base chat function
    chat_with_loaded_model_streaming(
        app,
        model_name,
        message,
        session_id,
        include_history,
        Some(enhanced_system_prompt),
        temperature,
        top_p,
        seed,
        max_tokens,
        max_completion_tokens,
        attachments // Pass all attachments, images will be handled separately
    ).await
}

async fn perform_rag_retrieval(
    query: &str, 
    limit: Option<usize>,
    attached_file_paths: Option<&Vec<String>>
) -> Result<String, String> {
    tracing::info!(
        query_length = query.len(), 
        limit = ?limit,
        has_attached_files = attached_file_paths.is_some(),
        attached_count = attached_file_paths.map(|f| f.len()),
        "Starting RAG retrieval"
    );
    
    // Create query embedding
    let embedding_service = crate::rag::embeddings::EmbeddingService::new();
    let query_embedding = embedding_service.create_single_embedding(query.to_string()).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to create query embedding");
            e
        })?;
    tracing::debug!(embedding_dim = query_embedding.len(), "Query embedding created");

    // Search similar documents
    let vector_store = crate::rag::vector_store::VectorStore::new()?;
    
    // If attached files are specified, search only in those files
    let search_results = if let Some(file_paths) = attached_file_paths {
        tracing::info!(file_count = file_paths.len(), "Searching only in attached files");
        vector_store.search_similar_in_files(&query_embedding, file_paths, limit.unwrap_or(100))?
    } else {
        // Otherwise, search all documents with the specified limit
        let search_limit = limit.unwrap_or(5) * 2; // Get more for reranking
        vector_store.search_similar(&query_embedding, search_limit)?
    };
    
    tracing::info!(results_found = search_results.len(), "Vector search completed");

    if search_results.is_empty() {
        tracing::warn!("No similar documents found in vector store");
        return Ok(String::new());
    }

    // Rerank results
    let reranker = crate::rag::reranker::RerankerService::new();
    let reranked_results = reranker.rerank(query, search_results).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to rerank results");
            e
        })?;
    
    tracing::info!(reranked_count = reranked_results.len(), "Results reranked");

    // Build context from top results
    // Use a higher count if filtering by specific files
    let default_top_results = if attached_file_paths.is_some() { 10 } else { 5 };
    let top_results_count = std::cmp::min(
        default_top_results, 
        limit.unwrap_or(default_top_results)
    );
    let context_content = reranked_results
        .iter()
        .take(top_results_count)
        .enumerate()
        .map(|(i, result)| {
            tracing::debug!(
                chunk_index = i,
                score = result.score,
                rerank_score = ?result.rerank_score,
                content_length = result.document.content.len(),
                file_path = %result.document.file_path,
                "Including document chunk in context"
            );
            format!(
                "Source {}: {}\nContent: {}\nRelevance Score: {:.2}\n---",
                i + 1,
                result.document.title,
                &result.document.content, // Use full content instead of truncating
                result.rerank_score.unwrap_or(result.score)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    tracing::info!(
        context_length = context_content.len(),
        chunks_included = top_results_count,
        "RAG context built successfully"
    );

    Ok(context_content)
}

fn extract_all_tool_calls_from_xml(text: &str) -> Vec<(String, String)> {
    let mut tool_calls = Vec::new();
    let mut search_start = 0;

    while let Some(start) = text[search_start..].find("<tool_call>") {
        let actual_start = search_start + start;
        if let Some(end) = text[actual_start..].find("</tool_call>") {
            let actual_end = actual_start + end;
            let tool_call_content = &text[actual_start + 11..actual_end]; // 11 is length of "<tool_call>"

            // Parse JSON inside the tool_call tags
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(tool_call_content) {
                if let (Some(name), Some(args)) = (parsed.get("name"), parsed.get("arguments")) {
                    if let (Some(name_str), Some(args_obj)) = (name.as_str(), args.as_object()) {
                        tool_calls.push((
                            name_str.to_string(),
                            serde_json::to_string(args_obj).unwrap_or_default(),
                        ));
                    }
                }
            }
            search_start = actual_end + 12; // 12 is length of "</tool_call>"
        } else {
            break;
        }
    }

    tool_calls
}

fn extract_tool_response_content(text: &str) -> String {
    // Extract content from <tool_response> tags without the XML formatting
    if let Some(start) = text.find("<tool_response>") {
        if let Some(end) = text[start..].find("</tool_response>") {
            let content_start = start + 15; // Length of "<tool_response>"
            let content_end = start + end;
            let raw_content = text[content_start..content_end].trim();
            
            // Try to parse as JSON and pretty-print it for better LLM readability
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw_content) {
                // Successfully parsed - return pretty-printed JSON
                return serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| raw_content.to_string());
            }
            
            // If not valid JSON, return as-is
            return raw_content.to_string();
        }
    }
    // If no tool_response found, return the whole text
    text.to_string()
}

fn has_incomplete_tool_call(text: &str) -> bool {
    if let Some(start) = text.rfind("<tool_call>") {
        if let Some(_end) = text[start..].find("</tool_call>") {
            return false; // Complete tool call found
        }
        return true; // Incomplete tool call
    }
    false
}

fn check_if_continuation_needed(text: &str) -> bool {
    // Always continue conversation when any tool_response is found, regardless of content
    text.contains("<tool_response>")
}

#[allow(dead_code)]
fn truncate_content(content: &str, max_length: usize) -> String {
    if content.len() <= max_length {
        content.to_string()
    } else {
        let truncated = &content[..max_length];
        // Try to find the last complete word
        if let Some(last_space) = truncated.rfind(' ') {
            format!("{}...", &truncated[..last_space])
        } else {
            format!("{}...", truncated)
        }
    }
}

fn strip_tool_xml_tags(content: &str) -> String {
    let mut result = String::new();
    let mut current_pos = 0;
    
    while current_pos < content.len() {
        // Look for tool_call start
        if let Some(tool_call_start) = content[current_pos..].find("<tool_call>") {
            let actual_start = current_pos + tool_call_start;
            
            // Add any text before the tool_call
            result.push_str(&content[current_pos..actual_start]);
            
            // Find the end of tool_call
            if let Some(tool_call_end_offset) = content[actual_start..].find("</tool_call>") {
                let tool_call_end = actual_start + tool_call_end_offset + 12; // 12 = "</tool_call>".len()
                current_pos = tool_call_end;
                continue;
            } else {
                // Incomplete tool_call, skip to end
                break;
            }
        }
        // Look for tool_response start
        else if let Some(tool_response_start) = content[current_pos..].find("<tool_response>") {
            let actual_start = current_pos + tool_response_start;
            
            // Add any text before the tool_response
            result.push_str(&content[current_pos..actual_start]);
            
            // Find the end of tool_response
            if let Some(tool_response_end_offset) = content[actual_start..].find("</tool_response>") {
                let tool_response_end = actual_start + tool_response_end_offset + 16; // 16 = "</tool_response>".len()
                current_pos = tool_response_end;
                continue;
            } else {
                // Incomplete tool_response, skip to end
                break;
            }
        }
        else {
            // No more XML tags, add remaining content
            result.push_str(&content[current_pos..]);
            break;
        }
    }
    
    result.trim().to_string()
}
