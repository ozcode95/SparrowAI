use serde::{ Deserialize, Serialize };
use tracing::{ warn, error, debug, info }; // Add 'info' to the tracing import
use serde_json;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;
use async_openai::types::ChatCompletionRequestUserMessageArgs;
use async_openai::types::ChatCompletionRequestSystemMessageArgs;
use async_openai::types::ChatCompletionRequestAssistantMessageArgs;
// Removed unused tool choice imports since tools are now in system message
use async_openai::{ types::CreateChatCompletionRequestArgs, Client };
use async_openai::{ config::OpenAIConfig };
use futures::StreamExt;
use tauri::{ AppHandle, Emitter };
use crate::mcp;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String, // "user" or "assistant"
    pub content: String,
    pub timestamp: i64,
    pub tokens_per_second: Option<f64>,
    pub is_error: Option<bool>,
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
    let home_dir = std::env
        ::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "Failed to get user home directory".to_string())?;

    let sparrow_dir = PathBuf::from(home_dir).join(".sparrow");

    // Create .sparrow directory if it doesn't exist
    if !sparrow_dir.exists() {
        fs
            ::create_dir_all(&sparrow_dir)
            .map_err(|e| format!("Failed to create .sparrow directory: {}", e))?;
    }

    Ok(sparrow_dir.join("chat_sessions.json"))
}

fn load_chat_sessions() -> Result<ChatSessionsStorage, String> {
    debug!("Loading chat sessions");
    let path = get_chat_sessions_path()?;

    if !path.exists() {
        return Ok(ChatSessionsStorage::default());
    }

    let contents = fs
        ::read_to_string(&path)
        .map_err(|e| format!("Failed to read chat sessions file: {}", e))?;

    let result = serde_json
        ::from_str::<ChatSessionsStorage>(&contents)
        .map_err(|e| format!("Failed to parse chat sessions: {}", e));
    match &result {
        Ok(sessions) =>
            debug!(session_count = sessions.sessions.len(), "Chat sessions loaded successfully"),
        Err(e) => error!(error = %e, "Failed to load chat sessions"),
    }
    result
}

fn save_chat_sessions(storage: &ChatSessionsStorage) -> Result<(), String> {
    debug!(session_count = storage.sessions.len(), "Saving chat sessions");
    let path = get_chat_sessions_path()?;

    let contents = serde_json
        ::to_string_pretty(storage)
        .map_err(|e| format!("Failed to serialize chat sessions: {}", e))?;

    fs::write(&path, contents).map_err(|e| format!("Failed to write chat sessions file: {}", e))
}

fn generate_chat_title(content: &str) -> String {
    // Clean the content and create a meaningful title
    let cleaned = content.trim();

    // Remove common question words and make it more title-like
    let title = if cleaned.len() <= 60 {
        cleaned.to_string()
    } else {
        // Find a good break point near 60 characters
        let mut break_point = 60;
        if let Some(space_pos) = cleaned[..60].rfind(' ') {
            if space_pos > 40 {
                // Only use space if it's not too early
                break_point = space_pos;
            }
        }
        format!("{}...", &cleaned[..break_point])
    };

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

#[tauri::command]
pub async fn get_chat_sessions() -> Result<ChatSessionsStorage, String> {
    load_chat_sessions()
}

#[tauri::command]
pub async fn create_chat_session(title: Option<String>) -> Result<ChatSession, String> {
    let mut storage = load_chat_sessions()?;

    let session_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    let session = ChatSession {
        id: session_id.clone(),
        title: title.unwrap_or_else(|| "New Chat".to_string()),
        created_at: now,
        updated_at: now,
        model_id: None,
        messages: Vec::new(),
    };

    storage.sessions.insert(session_id.clone(), session.clone());
    storage.active_session_id = Some(session_id);

    save_chat_sessions(&storage)?;

    Ok(session)
}

#[tauri::command]
pub async fn create_temporary_chat_session(title: Option<String>) -> Result<ChatSession, String> {
    let session_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    let session = ChatSession {
        id: session_id.clone(),
        title: title.unwrap_or_else(|| "New Chat".to_string()),
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
    is_error: Option<bool>
) -> Result<ChatMessage, String> {
    let mut storage = load_chat_sessions()?;

    let session = storage.sessions
        .get_mut(&session_id)
        .ok_or_else(|| format!("Chat session not found: {}", session_id))?;

    let message_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    let message = ChatMessage {
        id: message_id,
        role,
        content: content.clone(),
        timestamp: now,
        tokens_per_second,
        is_error,
    };

    session.messages.push(message.clone());
    session.updated_at = now;

    // Auto-generate title from first user message if still "New Chat"
    if session.title == "New Chat" && message.role == "user" {
        let title = generate_chat_title(&content);
        session.title = title;
    }

    save_chat_sessions(&storage)?;

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
    is_error: Option<bool>
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
    max_completion_tokens: Option<u32>
) -> Result<String, String> {
    let config = OpenAIConfig::new()
        .with_api_key("unused")
        .with_api_base("http://localhost:1114/v1");
    let client = Client::with_config(config);

    // Get MCP tools info for system message
    let mcp_tools = match mcp::get_all_mcp_tools_for_chat(app.clone()).await {
        Ok(tools) => {
            debug!("Successfully loaded {} MCP tools for system message", tools.len());
            tools
        }
        Err(e) => {
            warn!("Failed to load MCP tools for system message: {}", e);
            Vec::new()
        }
    };

    let tools_info = if !mcp_tools.is_empty() {
        debug!("Processing MCP tools for system message...");

        // Generate tool descriptions in simple text format for the custom template
        let tool_descs: Vec<String> = mcp_tools
            .iter()
            .enumerate()
            .map(|(i, tool)| {
                debug!("Processing tool {}: {}", i, tool.function.name);
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
        let formatted_tools =
            format!(r#"

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

        debug!("Generated custom tool template: {} characters", formatted_tools.len());
        formatted_tools
    } else {
        debug!("No MCP tools available for system message");
        "".to_string()
    };

    let base_system_message = system_prompt.unwrap_or_else(|| {
        "You are a helpful AI assistant with access to various functions/tools. 
        You MUST use the available tools when they are relevant to answer the user's request.

        IMPORTANT RULES:
        1. NEVER make up or guess information that could be obtained from a function call
        2. If you have a tool that can answer the question, USE IT

        Available tools should be called whenever relevant to provide accurate, up-to-date information.".to_string()
    });

    // Always append tools info to system message (whether custom or default)
    let system_message = format!("{}{}", base_system_message, tools_info);

    debug!("Message: {}", system_message);
    // Log what we're including
    debug!("System message length: {} chars", system_message.len());
    debug!("Tools info length: {} chars", tools_info.len());
    if !tools_info.is_empty() {
        debug!("Including tools info in system message");
    } else {
        debug!("No tools info to include");
    }

    let mut messages = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content(system_message.clone())
            .build()
            .map_err(|e| format!("Failed to build system message: {}", e))?
            .into()
    ];

    // Include conversation history if requested and session_id is provided
    if include_history.unwrap_or(false) && session_id.is_some() {
        match get_conversation_history(session_id.clone().unwrap()).await {
            Ok(mut history) => {
                // Remove the last user message if it matches the current message
                // This prevents duplicate user messages
                if let Some(last_msg) = history.last() {
                    if last_msg.role == "user" && last_msg.content == message {
                        history.pop(); // Remove the last message
                    }
                }

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
                            messages.push(
                                ChatCompletionRequestAssistantMessageArgs::default()
                                    .content(msg.content.clone())
                                    .build()
                                    .map_err(|e|
                                        format!("Failed to build assistant message: {}", e)
                                    )?
                                    .into()
                            );
                        }
                        _ => {
                            warn!(role = %msg.role, "Skipping unknown role");
                            continue;
                        } // Skip unknown roles
                    };
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to get conversation history");
            }
        }
    }

    // Always add the current user message
    messages.push(
        ChatCompletionRequestUserMessageArgs::default()
            .content(message.clone())
            .build()
            .map_err(|e| format!("Failed to build user message: {}", e))?
            .into()
    );

    debug!("Starting chat request");

    // Create streaming chat completion
    let mut request_builder = CreateChatCompletionRequestArgs::default();
    request_builder
        .model(model_name.clone())
        .messages(messages.clone())
        .stream(true)
        .temperature(temperature.unwrap_or(0.7) as f32)
        .top_p(top_p.unwrap_or(1.0) as f32);

    // Only set these parameters if they have values
    if let Some(seed) = seed {
        request_builder.seed(seed);
    }

    // Set a reasonable max_tokens for function calls (override if too low)
    let effective_max_tokens = max_tokens.unwrap_or(1000).max(100); // Ensure at least 100 tokens
    request_builder.max_tokens(effective_max_tokens);

    if let Some(max_completion_tokens) = max_completion_tokens {
        request_builder.max_completion_tokens(max_completion_tokens);
    }

    // Commented out: Add MCP tools using modern tools format
    /*
    match mcp::get_all_mcp_tools_for_chat(app.clone()).await {
        Ok(mcp_tools) => {
            if !mcp_tools.is_empty() {
                let mcp_info = format!("Adding {} MCP tools to chat completion", mcp_tools.len());
                debug!("{}", mcp_info);

                // Log each tool for debugging
                for (i, tool) in mcp_tools.iter().enumerate() {
                    let tool_info = format!(
                        "Tool {}: {} - {}",
                        i,
                        &tool.function.name,
                        tool.function.description.as_ref().unwrap_or(&"No description".to_string())
                    );
                    debug!("{}", tool_info);
                }

                debug!("Using modern 'tools' format...");
                request_builder.tools(mcp_tools.clone());

                // Determine tool choice based on message content
                let message_lower = message.to_lowercase();
                let forced_tool = if message_lower.contains("time") || message_lower.contains("current") {
                    mcp_tools.iter().find(|tool| tool.function.name.contains("time_get_current_time"))
                } else if message_lower.contains("convert") && message_lower.contains("time") {
                    mcp_tools.iter().find(|tool| tool.function.name.contains("time_convert_time"))
                } else {
                    None
                };

                if let Some(tool) = forced_tool {
                    debug!("Forcing specific tool call: {}", tool.function.name);
                    
                    let specific_choice = ChatCompletionNamedToolChoice {
                        r#type: ChatCompletionToolType::Function,
                        function: FunctionName {
                            name: tool.function.name.clone(),
                        },
                    };
                    request_builder.tool_choice(ChatCompletionToolChoiceOption::Named(specific_choice));
                } else {
                    debug!("Using auto tool choice (no specific match found)");
                    request_builder.tool_choice(ChatCompletionToolChoiceOption::Auto);
                }
            } else {
                debug!("No MCP tools available");
            }
        }
        Err(e) => {
            let mcp_error = format!("Failed to get MCP tools: {}", e);
            warn!("{}", mcp_error);
            // Continue without tools
        }
    }
    */

    // Tools info is now in system message instead
    debug!("Tools info included in system message instead of request tools array");

    let request = request_builder
        .build()
        .map_err(|e| format!("Failed to build chat request: {}", e))?;

    // Request details logged to file only (verbose output disabled for console)

    // Check system message for tools info (since tools are now in system message)
    if let Ok(request_value) = serde_json::to_value(&request) {
        if let Some(messages) = request_value.get("messages") {
            if let Some(messages_array) = messages.as_array() {
                if let Some(system_msg) = messages_array.get(0) {
                    if let Some(content) = system_msg.get("content") {
                        if let Some(content_str) = content.as_str() {
                            if
                                content_str.contains("<tools>") ||
                                content_str.contains("Available functions:")
                            {
                                debug!("Tools info found in system message");
                            } else {
                                debug!("No tools info found in system message");
                            }
                        }
                    }
                }
            }
        }

        // Verify no tools array in request (should be commented out)
        if request_value.get("tools").is_some() {
            warn!("Tools array still present in request!");
        } else {
            debug!("Confirmed: No tools array in request (as expected)");
        }
    }
    // Request logging complete

    let mut stream = client
        .chat()
        .create_stream(request).await
        .map_err(|e| format!("Failed to create chat stream: {}", e))?;

    let mut full_response = String::new();
    let mut executed_tools = std::collections::HashSet::new();
    let mut needs_continuation = false;

    // Process streaming responses with function call support
    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                // Stream response chunk logging disabled for cleaner output

                for chat_choice in response.choices {
                    // Processing stream choice (verbose logging disabled)

                    // Handle content and look for <tool_call> XML tags
                    if let Some(content) = &chat_choice.delta.content {
                        full_response.push_str(content);

                        // Emit streaming content to frontend (including XML tags)
                        let _ = app.emit(
                            "chat-token",
                            serde_json::json!({
                                "token": content,
                                "finished": false
                            })
                        );

                        // Process any complete tool calls found in the response so far
                        let tool_calls = extract_all_tool_calls_from_xml(&full_response);

                        for (fn_name, fn_args) in tool_calls {
                            // Skip if we already executed this exact tool call
                            let tool_signature = format!("{}:{}", fn_name, fn_args);
                            if executed_tools.contains(&tool_signature) {
                                continue;
                            }

                            executed_tools.insert(tool_signature);

                            debug!("Found complete tool call: name={}, args={}", fn_name, fn_args);

                            // Parse arguments as JSON for MCP tool call
                            let args_map = match
                                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
                                    &fn_args
                                )
                            {
                                Ok(mut map) => {
                                    // Remove null values as MCP tools don't handle them well
                                    map.retain(|_k, v| !v.is_null());
                                    Some(map)
                                }
                                Err(e) => {
                                    let parse_error =
                                        format!("Failed to parse tool arguments: {}", e);
                                    warn!("{}", parse_error);
                                    None
                                }
                            };

                            // Call the MCP tool
                            match mcp::call_mcp_tool(app.clone(), fn_name.clone(), args_map).await {
                                Ok(tool_result) => {
                                    let tool_result_info = format!(
                                        "Tool {} returned: {}",
                                        fn_name,
                                        tool_result
                                    );
                                    debug!("{}", tool_result_info);

                                    // Emit function call result to frontend
                                    let _ = app.emit(
                                        "tool-call",
                                        serde_json::json!({
                                            "tool_name": fn_name,
                                            "arguments": fn_args,
                                            "result": tool_result
                                        })
                                    );

                                    // Add tool response in Qwen-Agent format and emit to frontend
                                    let tool_response_text =
                                        format!("\n<tool_response>\n{}\n</tool_response>", tool_result);
                                    full_response.push_str(&tool_response_text);

                                    // Emit tool response as streaming content (including XML tags)
                                    let _ = app.emit(
                                        "chat-token",
                                        serde_json::json!({
                                            "token": tool_response_text,
                                            "finished": false
                                        })
                                    );

                                    // Mark that we need to continue the conversation after tool execution
                                    needs_continuation = true;
                                }
                                Err(e) => {
                                    let tool_error = format!("Tool call failed: {}", e);
                                    error!("{}", tool_error);
                                    let error_response_text =
                                        format!("\n<tool_response>\nError: {}\n</tool_response>", e);
                                    full_response.push_str(&error_response_text);

                                    // Emit error response as streaming content (including XML tags)
                                    let _ = app.emit(
                                        "chat-token",
                                        serde_json::json!({
                                            "token": error_response_text,
                                            "finished": false
                                        })
                                    );

                                    // Mark that we need to continue the conversation even after tool error
                                    needs_continuation = true;
                                }
                            }
                        }
                    }

                    // Handle finish reason
                    if let Some(_finish_reason) = &chat_choice.finish_reason {
                        debug!("Stream finished with reason: {:?}", _finish_reason);

                        // Check for any remaining incomplete tool calls
                        if has_incomplete_tool_call(&full_response) {
                            warn!("Stream ended with incomplete tool call");
                        }
                    }
                }
            }
            Err(err) => {
                let error_info = format!("error: {err}");
                error!("{}", error_info);
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

    // Continue the conversation if we executed tools and got JSON responses
    if needs_continuation {
        debug!("Checking if continuation is needed after tool execution...");

        // Check if the tool responses contain JSON structures that need interpretation
        let should_continue = check_if_continuation_needed(&full_response);

        if should_continue {
            debug!("Tool response contains JSON - continuing conversation...");

            match
                continue_conversation_after_tools(
                    app.clone(),
                    &client,
                    &system_message,
                    &messages,
                    full_response.clone(),
                    &model_name,
                    temperature,
                    top_p,
                    seed,
                    max_tokens,
                    max_completion_tokens
                ).await
            {
                Ok(continued_response) => {
                    if !continued_response.trim().is_empty() {
                        // Append the continued response (streaming is already handled by continue_conversation_after_tools)
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
        } else {
            debug!("Tool response doesn't contain JSON - no continuation needed");
        }
    }

    // Emit completion signal
    let _ = app.emit(
        "chat-token",
        serde_json::json!({
        "token": "",
        "finished": true
    })
    );

    // Log the complete response before breaking
    debug!(
        message_length = full_response.len(),
        session_id = ?session_id,
        model = %model_name,
        "Chat completion finished"
    );

    // Log full response (truncated for readability)
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
    system_message: &str,
    previous_messages: &[async_openai::types::ChatCompletionRequestMessage],
    assistant_response_with_tools: String,
    model_name: &str,
    temperature: Option<f64>,
    top_p: Option<f64>,
    seed: Option<i64>,
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>
) -> Result<String, String> {
    debug!("Continuing conversation after tool execution");

    // Build new message list with the assistant's response containing tool calls and results
    let mut continuation_messages = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content(system_message.to_string())
            .build()
            .map_err(|e| format!("Failed to build system message: {}", e))?
            .into()
    ];

    // Add previous conversation messages (excluding system message)
    for msg in previous_messages.iter().skip(1) {
        continuation_messages.push(msg.clone());
    }

    // Add the assistant message with tool calls and responses
    continuation_messages.push(
        ChatCompletionRequestAssistantMessageArgs::default()
            .content(assistant_response_with_tools)
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
                        debug!("Continuation finished with reason: {:?}", finish_reason);
                        break;
                    }
                }
            }
            Err(err) => {
                let error_info = format!("Continuation stream error: {}", err);
                error!("{}", error_info);
                return Err(error_info);
            }
        }
    }

    debug!("Continuation response: {}", continued_response);
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
    rag_limit: Option<usize>
) -> Result<String, String> {
    let mut context_content = String::new();

    // RAG retrieval if enabled
    if use_rag.unwrap_or(false) {
        match perform_rag_retrieval(&message, rag_limit.unwrap_or(5)).await {
            Ok(context) => {
                context_content = context;
            }
            Err(e) => {
                error!(error = %e, "RAG retrieval failed");
                // Continue without RAG context rather than failing completely
            }
        }
    }

    // Enhanced system prompt with context
    let enhanced_system_prompt = if !context_content.is_empty() {
        format!(
            "{}\n\nRelevant context from documents:\n{}\n\nUse this context to answer the user's question when relevant. If the context doesn't contain relevant information, answer based on your general knowledge.",
            system_prompt.unwrap_or_else(||
                "You're an AI assistant that provides helpful responses.".to_string()
            ),
            context_content
        )
    } else {
        system_prompt.unwrap_or_else(||
            "You're an AI assistant that provides helpful responses.".to_string()
        )
    };

    // Use existing chat function with enhanced prompt
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
        max_completion_tokens
    ).await
}

async fn perform_rag_retrieval(query: &str, limit: usize) -> Result<String, String> {
    // Create query embedding
    let embedding_service = crate::rag::embeddings::EmbeddingService::new();
    let query_embedding = embedding_service.create_single_embedding(query.to_string()).await?;

    // Search similar documents
    let vector_store = crate::rag::vector_store::VectorStore::new()?;
    let search_results = vector_store.search_similar(&query_embedding, limit * 2)?; // Get more for reranking

    if search_results.is_empty() {
        return Ok(String::new());
    }

    // Rerank results
    let reranker = crate::rag::reranker::RerankerService::new();
    let reranked_results = reranker.rerank(query, search_results).await?;

    // Build context from top results
    let context_content = reranked_results
        .iter()
        .take(std::cmp::min(3, limit)) // Use top 3 results or limit, whichever is smaller
        .enumerate()
        .map(|(i, result)| {
            format!(
                "Source {}: {}\nContent: {}\nRelevance Score: {:.2}\n---",
                i + 1,
                result.document.title,
                truncate_content(&result.document.content, 500), // Limit content length
                result.rerank_score.unwrap_or(result.score)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

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
