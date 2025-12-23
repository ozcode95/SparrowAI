use super::config::{McpConfig, McpServerConfig};
use super::client::{McpManager, McpServerInfo};
use super::builtin_tools::{BuiltinToolRegistry, BuiltinTool, ToolResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use serde_json::Value;

// Global MCP manager instance
lazy_static::lazy_static! {
    static ref MCP_MANAGER: Arc<Mutex<Option<McpManager>>> = Arc::new(Mutex::new(None));
    static ref BUILTIN_TOOLS: BuiltinToolRegistry = BuiltinToolRegistry::new();
}

async fn get_or_init_manager(app_handle: &AppHandle) -> Result<(), String> {
    let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
    
    if manager_guard.is_none() {
        let config_path = McpConfig::get_config_path(app_handle)
            .map_err(|e| format!("Failed to get config path: {}", e))?;
            
        let config = McpConfig::load_from_file(&config_path)
            .map_err(|e| format!("Failed to load config: {}", e))?;
            
        *manager_guard = Some(McpManager::new(config));
    }
    
    Ok(())
}

#[tauri::command]
pub async fn get_mcp_servers(app_handle: AppHandle) -> Result<Vec<McpServerInfo>, String> {
    get_or_init_manager(&app_handle).await?;
    
    // We can't hold the lock across await, so we need to restructure this
    // For now, let's create the server info without async calls in the critical section
    let servers = {
        let manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        let manager = manager_guard.as_ref().ok_or("Manager not initialized")?;
        
        // Get basic server info without tools for now
        let mut servers = Vec::new();
        for (name, config) in manager.get_config().list_servers() {
            let status = if manager.clients.contains_key(name) {
                "connected"
            } else {
                "disconnected"
            };
            
            servers.push(McpServerInfo {
                name: name.clone(),
                config: config.clone(),
                status: status.to_string(),
                tools: vec![], // Will be populated separately
            });
        }
        servers
    };
    
    // TODO: Fetch tools for connected servers in a separate step
    
    Ok(servers)
}

#[derive(Serialize, Deserialize)]
pub struct AddServerRequest {
    pub name: String,
    // Stdio fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    
    // URL-based fields (SSE/HTTP)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    
    // Auto-connect on startup
    #[serde(default)]
    pub auto_connect: bool,
}

#[tauri::command]
pub async fn add_mcp_server(
    app_handle: AppHandle,
    request: AddServerRequest,
) -> Result<String, String> {
    get_or_init_manager(&app_handle).await?;
    
    let server_config = McpServerConfig {
        command: request.command,
        args: request.args,
        env: request.env,
        url: request.url,
        auto_connect: request.auto_connect,
    };
    
    // Validate the configuration
    server_config.validate().map_err(|e| format!("Invalid configuration: {}", e))?;
    
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        let manager = manager_guard.as_mut().ok_or("Manager not initialized")?;
        
        manager.add_server(request.name.clone(), server_config);
        
        // Save config to file
        let config_path = McpConfig::get_config_path(&app_handle)
            .map_err(|e| format!("Failed to get config path: {}", e))?;
        manager.get_config().save_to_file(&config_path)
            .map_err(|e| format!("Failed to save config: {}", e))?;
    }
    
    Ok(format!("MCP server '{}' added successfully", request.name))
}

#[tauri::command]
pub async fn edit_mcp_server(
    app_handle: AppHandle,
    request: AddServerRequest,
) -> Result<String, String> {
    get_or_init_manager(&app_handle).await?;
    
    let server_config = McpServerConfig {
        command: request.command,
        args: request.args,
        env: request.env,
        url: request.url,
        auto_connect: request.auto_connect,
    };
    
    // Validate the configuration
    server_config.validate().map_err(|e| format!("Invalid configuration: {}", e))?;
    
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        let manager = manager_guard.as_mut().ok_or("Manager not initialized")?;
        
        // Check if server exists
        if manager.get_config().get_server(&request.name).is_none() {
            return Err(format!("Server '{}' not found", request.name));
        }
        
        // Check if server is currently connected (if so, can't edit)
        if manager.clients.contains_key(&request.name) {
            return Err(format!("Cannot edit server '{}' while it is connected. Please disconnect first.", request.name));
        }
        
        // Update the server configuration
        manager.add_server(request.name.clone(), server_config);
        
        // Save config to file
        let config_path = McpConfig::get_config_path(&app_handle)
            .map_err(|e| format!("Failed to get config path: {}", e))?;
        manager.get_config().save_to_file(&config_path)
            .map_err(|e| format!("Failed to save config: {}", e))?;
    }
    
    Ok(format!("MCP server '{}' updated successfully", request.name))
}

#[tauri::command]
pub async fn remove_mcp_server(
    app_handle: AppHandle,
    server_name: String,
) -> Result<String, String> {
    get_or_init_manager(&app_handle).await?;
    
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        let manager = manager_guard.as_mut().ok_or("Manager not initialized")?;
        
        manager.remove_server(&server_name)
            .ok_or_else(|| format!("Server '{}' not found", server_name))?;
        
        // Save config to file
        let config_path = McpConfig::get_config_path(&app_handle)
            .map_err(|e| format!("Failed to get config path: {}", e))?;
        manager.get_config().save_to_file(&config_path)
            .map_err(|e| format!("Failed to save config: {}", e))?;
    }
    
    Ok(format!("MCP server '{}' removed successfully", server_name))
}

#[tauri::command]
pub async fn connect_mcp_server(
    app_handle: AppHandle,
    server_name: String,
) -> Result<String, String> {
    log_operation_start!("Connect MCP server");
    tracing::debug!(server = %server_name, "Connecting to MCP server");
    
    get_or_init_manager(&app_handle).await?;
    
    // We need to extract the manager temporarily to call async methods
    let mut temp_manager = {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| {
            log_operation_error!("Connect MCP server", &e, note = "lock error");
            format!("Lock error: {}", e)
        })?;
        manager_guard.take().ok_or_else(|| {
            log_operation_error!("Connect MCP server", "Manager not initialized");
            "Manager not initialized".to_string()
        })?
    };
    
    // Connect to server (this is async)
    let connection_result = temp_manager.connect_to_server(&server_name).await;
    
    // Put the manager back
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        *manager_guard = Some(temp_manager);
    }
    
    // Handle connection result
    connection_result.map_err(|e| {
        log_operation_error!("Connect MCP server", &e, server = %server_name);
        format!("Failed to connect to server '{}': {}", server_name, e)
    })?;
    
    log_operation_success!("Connect MCP server");
    Ok(format!("Connected to MCP server '{}'", server_name))
}

#[tauri::command]
pub async fn disconnect_mcp_server(
    app_handle: AppHandle,
    server_name: String,
) -> Result<String, String> {
    log_operation_start!("Disconnect MCP server");
    tracing::debug!(server = %server_name, "Disconnecting from MCP server");
    
    get_or_init_manager(&app_handle).await?;
    
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| {
            log_operation_error!("Disconnect MCP server", &e, note = "lock error");
            format!("Lock error: {}", e)
        })?;
        let manager = manager_guard.as_mut().ok_or_else(|| {
            log_operation_error!("Disconnect MCP server", "Manager not initialized");
            "Manager not initialized".to_string()
        })?;
        
        manager.disconnect_from_server(&server_name);
    }
    
    log_operation_success!("Disconnect MCP server");
    Ok(format!("Disconnected from MCP server '{}'", server_name))
}

#[tauri::command]
pub async fn get_mcp_server_info(
    app_handle: AppHandle,
    server_name: String,
) -> Result<Option<McpServerInfo>, String> {
    get_or_init_manager(&app_handle).await?;
    
    // Get basic info first
    let basic_info = {
        let manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        let manager = manager_guard.as_ref().ok_or("Manager not initialized")?;
        
        if let Some(config) = manager.get_config().get_server(&server_name) {
            let status = if manager.clients.contains_key(&server_name) {
                "connected"
            } else {
                "disconnected"
            };
            
            Some(McpServerInfo {
                name: server_name.clone(),
                config: config.clone(),
                status: status.to_string(),
                tools: vec![], // Will be populated below if connected
            })
        } else {
            None
        }
    };
    
    // If server is connected, try to fetch tools
    if let Some(info) = basic_info {
        if info.status == "connected" {
            // TODO: Implement tool fetching without holding the lock
            // This requires restructuring to avoid async in lock
        }
        Ok(Some(info))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn fetch_mcp_server_tools(
    app_handle: AppHandle,
    server_name: String,
) -> Result<Vec<String>, String> {
    get_or_init_manager(&app_handle).await?;
    
    // Similar pattern - extract manager temporarily
    let temp_manager = {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        manager_guard.take().ok_or("Manager not initialized")?
    };
    
    // Fetch tools (this is async)
    let tools_result = temp_manager.fetch_tools(&server_name).await;
    
    // Put the manager back
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        *manager_guard = Some(temp_manager);
    }
    
    // Handle result
    tools_result.map_err(|e| format!("Failed to fetch tools: {}", e))
}

#[tauri::command]
pub async fn fetch_mcp_server_tools_details(
    app_handle: AppHandle,
    server_name: String,
) -> Result<Vec<super::client::ToolInfo>, String> {
    get_or_init_manager(&app_handle).await?;
    
    // Similar pattern - extract manager temporarily
    let temp_manager = {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        manager_guard.take().ok_or("Manager not initialized")?
    };
    
    // Fetch tools with details (this is async)
    let tools_result = temp_manager.fetch_tools_with_details(&server_name).await;
    
    // Put the manager back
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        *manager_guard = Some(temp_manager);
    }
    
    // Handle result
    tools_result.map_err(|e| format!("Failed to fetch tools details: {}", e))
}

#[tauri::command]
pub async fn get_all_mcp_tools_for_chat(
    app_handle: AppHandle,
) -> Result<Vec<async_openai::types::ChatCompletionTool>, String> {
    // Get built-in tools first
    let mut all_tools = BUILTIN_TOOLS.to_openai_tools();
    
    tracing::debug!(builtin_count = all_tools.len(), "Added built-in tools for chat");
    
    // Get external MCP tools
    get_or_init_manager(&app_handle).await?;
    
    // Extract manager temporarily
    let temp_manager = {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        manager_guard.take().ok_or("Manager not initialized")?
    };
    
    // Get all external MCP tools (this is async)
    let external_tools_result = temp_manager.get_all_tools_for_openai().await;
    
    // Put the manager back
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        *manager_guard = Some(temp_manager);
    }
    
    // Add external tools to the list
    match external_tools_result {
        Ok(mut external_tools) => {
            tracing::debug!(external_count = external_tools.len(), "Added external MCP tools for chat");
            all_tools.append(&mut external_tools);
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to get external MCP tools, continuing with built-in tools only");
        }
    }
    
    tracing::info!(total_tools = all_tools.len(), "Total tools available for chat (built-in + external)");
    Ok(all_tools)
}

#[tauri::command]
pub async fn call_mcp_tool(
    app_handle: AppHandle,
    tool_name: String,
    arguments: Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<String, String> {
    log_operation_start!("Call tool");
    tracing::debug!(tool = %tool_name, has_args = arguments.is_some(), "Calling tool");
    
    // Check if this is a built-in tool (prefixed with "builtin_")
    if tool_name.starts_with("builtin_") {
        let actual_tool_name = &tool_name[8..]; // Remove "builtin_" prefix
        tracing::debug!(builtin_tool = %actual_tool_name, "Executing built-in tool");
        
        // Convert arguments to Value
        let args_value = match arguments {
            Some(map) => Value::Object(map),
            None => Value::Object(serde_json::Map::new()),
        };
        
        // Execute built-in tool
        let result = BUILTIN_TOOLS.execute_tool(actual_tool_name, args_value).await?;
        
        // Extract text from ToolResult
        let result_text = result.content.iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        
        log_operation_success!("Built-in tool executed");
        tracing::debug!(tool = %actual_tool_name, result_length = result_text.len(), "Built-in tool executed");
        
        return Ok(result_text);
    }
    
    // Otherwise, handle as external MCP tool
    get_or_init_manager(&app_handle).await?;
    
    // Extract manager temporarily
    let temp_manager = {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| {
            log_operation_error!("Call MCP tool", &e, note = "lock error");
            format!("Lock error: {}", e)
        })?;
        manager_guard.take().ok_or_else(|| {
            log_operation_error!("Call MCP tool", "Manager not initialized");
            "Manager not initialized".to_string()
        })?
    };
    
    // Call tool (this is async)
    let call_result = temp_manager.call_mcp_tool(&tool_name, arguments).await;
    
    // Put the manager back
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        *manager_guard = Some(temp_manager);
    }
    
    // Handle result
    let result = call_result.map_err(|e| {
        log_operation_error!("Call MCP tool", &e, tool = %tool_name);
        format!("Failed to call MCP tool: {}", e)
    })?;
    
    log_operation_success!("MCP tool executed");
    tracing::debug!(tool = %tool_name, result_length = result.len(), "External MCP tool executed");
    
    Ok(result)
}

#[tauri::command]
pub async fn toggle_mcp_server_auto_connect(
    app_handle: AppHandle,
    server_name: String,
    auto_connect: bool,
) -> Result<String, String> {
    get_or_init_manager(&app_handle).await?;
    
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        let manager = manager_guard.as_mut().ok_or("Manager not initialized")?;
        
        // Get the current server config
        let mut config = manager.get_config()
            .get_server(&server_name)
            .ok_or_else(|| format!("Server '{}' not found", server_name))?
            .clone();
        
        // Update auto_connect
        config.auto_connect = auto_connect;
        
        // Save the updated config
        manager.add_server(server_name.clone(), config);
        
        // Save to file
        let config_path = McpConfig::get_config_path(&app_handle)
            .map_err(|e| format!("Failed to get config path: {}", e))?;
        manager.get_config().save_to_file(&config_path)
            .map_err(|e| format!("Failed to save config: {}", e))?;
    }
    
    Ok(format!("Auto-connect {} for '{}'", if auto_connect { "enabled" } else { "disabled" }, server_name))
}

#[tauri::command]
pub async fn enable_all_auto_connect(
    app_handle: AppHandle,
) -> Result<String, String> {
    get_or_init_manager(&app_handle).await?;
    
    {
        let mut manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        let manager = manager_guard.as_mut().ok_or("Manager not initialized")?;
        
        // Get all server names
        let server_names: Vec<String> = manager.get_config()
            .list_servers()
            .into_iter()
            .map(|(name, _)| name.clone())
            .collect();
        
        // Enable auto_connect for all servers
        for server_name in server_names {
            if let Some(mut config) = manager.get_config().get_server(&server_name).cloned() {
                config.auto_connect = true;
                manager.add_server(server_name.clone(), config);
            }
        }
        
        // Save to file
        let config_path = McpConfig::get_config_path(&app_handle)
            .map_err(|e| format!("Failed to get config path: {}", e))?;
        manager.get_config().save_to_file(&config_path)
            .map_err(|e| format!("Failed to save config: {}", e))?;
    }
    
    Ok("Auto-connect enabled for all servers".to_string())
}

#[tauri::command]
pub async fn auto_connect_mcp_servers(
    app_handle: AppHandle,
) -> Result<Vec<String>, String> {
    get_or_init_manager(&app_handle).await?;
    
    // Get list of servers to auto-connect
    let servers_to_connect: Vec<String> = {
        let manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        let manager = manager_guard.as_ref().ok_or("Manager not initialized")?;
        
        manager.get_config()
            .list_servers()
            .into_iter()
            .filter_map(|(name, config)| {
                if config.auto_connect {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect()
    };
    
    let mut connected = Vec::new();
    
    // Connect to each server
    for server_name in servers_to_connect {
        match connect_mcp_server(app_handle.clone(), server_name.clone()).await {
            Ok(_) => connected.push(server_name),
            Err(e) => {
                tracing::warn!("Failed to auto-connect to server '{}': {}", server_name, e);
            }
        }
    }
    
    Ok(connected)
}

// ============================================================================
// Built-in MCP Tools Commands
// ============================================================================

/// Get all built-in tools
#[tauri::command]
pub async fn get_builtin_tools() -> Result<Vec<BuiltinTool>, String> {
    Ok(BUILTIN_TOOLS.list_tools())
}

/// Execute a built-in tool
#[tauri::command]
pub async fn execute_builtin_tool(
    tool_name: String,
    arguments: Value,
) -> Result<ToolResult, String> {
    tracing::debug!(tool = %tool_name, args = ?arguments, "Executing built-in tool");
    
    BUILTIN_TOOLS.execute_tool(&tool_name, arguments).await
}

/// Get all available tools (both built-in and external MCP servers)
#[tauri::command]
pub async fn get_all_available_tools(
    app_handle: AppHandle,
) -> Result<AllToolsResponse, String> {
    // Get built-in tools
    let builtin_tools = BUILTIN_TOOLS.list_tools();
    
    // Get external MCP server tools
    let mut external_tools = HashMap::new();
    
    // Try to get MCP servers (they might not be initialized)
    if let Ok(()) = get_or_init_manager(&app_handle).await {
        let manager_guard = MCP_MANAGER.lock().map_err(|e| format!("Lock error: {}", e))?;
        if let Some(manager) = manager_guard.as_ref() {
            for (server_name, _) in manager.get_config().list_servers() {
                if manager.clients.contains_key(server_name) {
                    // Server is connected, try to get its tools
                    // Note: This is a simplified version. In production, you'd want to
                    // fetch tools asynchronously without holding the lock
                    external_tools.insert(server_name.clone(), Vec::new());
                }
            }
        }
    }
    
    Ok(AllToolsResponse {
        builtin_tools,
        external_servers: external_tools,
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AllToolsResponse {
    pub builtin_tools: Vec<BuiltinTool>,
    pub external_servers: HashMap<String, Vec<String>>, // server_name -> tool_names
}
