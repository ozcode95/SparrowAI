use super::config::{ McpConfig, McpServerConfig, TransportType };
use tracing::{ info, warn, debug };
use rmcp::{
    ServiceExt,
    transport::{ TokioChildProcess, SseClientTransport, StreamableHttpClientTransport },
    service::RunningService,
    RoleClient,
};
use rmcp::model::CallToolRequestParam;
use serde::{ Deserialize, Serialize };
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use async_openai::types::chat::{ ChatCompletionTool, FunctionObjectArgs };
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub config: McpServerConfig,
    pub status: String, // "connected", "disconnected", "error"
    pub tools: Vec<String>,
}

pub struct McpManager {
    config: McpConfig,
    pub clients: HashMap<String, RunningService<RoleClient, ()>>,
}

impl McpManager {
    pub fn new(config: McpConfig) -> Self {
        Self {
            config,
            clients: HashMap::new(),
        }
    }

    pub async fn connect_to_server(
        &mut self,
        name: &str
    ) -> Result<(), Box<dyn std::error::Error>> {
        log_operation_start!("MCP server connection");
        tracing::debug!(server = %name, "Connecting to MCP server");
        
        let server_config = self.config
            .get_server(name)
            .ok_or_else(|| {
                log_operation_error!("MCP server connection", "Server not found", server = %name);
                format!("Server '{}' not found in configuration", name)
            })?;

        // Validate configuration
        server_config.validate().map_err(|e| {
            log_operation_error!("MCP server connection", &e, server = %name, note = "invalid configuration");
            format!("Invalid server configuration: {}", e)
        })?;

        let transport_type = server_config.get_transport_type();
        tracing::debug!(server = %name, transport_type = ?transport_type, "Detected transport type");

        let client = match transport_type {
            TransportType::Stdio => {
                let command = server_config.command.as_ref().unwrap();
                let args = server_config.args.as_deref().unwrap_or(&[]);

                tracing::debug!(command = %command, args = ?args, "Starting MCP server via stdio");

                // Create the command - on Windows, we might need to handle .cmd extensions
                let mut cmd = if
                    cfg!(target_os = "windows") &&
                    !command.ends_with(".exe") &&
                    !command.ends_with(".cmd")
                {
                    // Try to find the command with .cmd extension first
                    let cmd_with_extension = format!("{}.cmd", command);
                    let mut test_cmd = Command::new(&cmd_with_extension);
                    test_cmd.args(&["--version"]);

                    match test_cmd.output().await {
                        Ok(_) => {
                            tracing::trace!(original = %command, resolved = %cmd_with_extension, "Resolved Windows command with .cmd extension");
                            Command::new(cmd_with_extension)
                        }
                        Err(_) => {
                            tracing::trace!(command = %command, "Using original command");
                            Command::new(command)
                        }
                    }
                } else {
                    Command::new(command)
                };

                cmd.args(args);
                cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

                // Hide console window on Windows to prevent terminal popup
                #[cfg(target_os = "windows")]
                {
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    cmd.creation_flags(CREATE_NO_WINDOW);
                }

                // Ensure PATH is inherited from the current environment on Windows
                if cfg!(target_os = "windows") {
                    if let Ok(path_var) = std::env::var("PATH") {
                        cmd.env("PATH", path_var);
                    }
                }

                // Set environment variables if provided
                if let Some(env_vars) = &server_config.env {
                    for (key, value) in env_vars {
                        cmd.env(key, value);
                    }
                }

                // Create transport and connect
                let transport = TokioChildProcess::new(cmd).map_err(|e| {
                    log_operation_error!("MCP server start", &e, command = %command, args = ?args);
                    format!("Failed to start command '{}': {}", command, e)
                })?;
                ().serve(transport).await?
            }
            TransportType::Sse => {
                let url = server_config.url.as_ref().unwrap();
                tracing::debug!(url = %url, "Connecting to MCP server via SSE");

                let transport = SseClientTransport::start(url.clone()).await?;
                ().serve(transport).await?
            }
            TransportType::StreamableHttp => {
                let url = server_config.url.as_ref().unwrap();
                tracing::debug!(url = %url, "Connecting to MCP server via Streamable HTTP");

                let transport = StreamableHttpClientTransport::from_uri(url.clone());
                ().serve(transport).await?
            }
        };

        self.clients.insert(name.to_string(), client);
        log_operation_success!("MCP server connection");
        tracing::debug!(server = %name, "Successfully connected to MCP server");
        Ok(())
    }

    pub fn disconnect_from_server(&mut self, name: &str) {
        tracing::debug!(server = %name, "Disconnecting from MCP server");
        self.clients.remove(name);
    }

    pub async fn fetch_tools(
        &self,
        server_name: &str
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let client = self.clients
            .get(server_name)
            .ok_or_else(|| {
                log_operation_error!("Fetch MCP tools", "Server not connected", server = %server_name);
                format!("Server '{}' not connected", server_name)
            })?;

        tracing::debug!(server = %server_name, "Fetching tools from MCP server");
        let tools_response = client.list_tools(Default::default()).await?;

        let tool_names: Vec<String> = tools_response.tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();

        info!(server_name = %server_name, tool_count = tool_names.len(), tools = ?tool_names, "Found tools from MCP server");
        Ok(tool_names)
    }

    pub async fn fetch_tools_with_details(
        &self,
        server_name: &str
    ) -> Result<Vec<ToolInfo>, Box<dyn std::error::Error>> {
        let client = self.clients
            .get(server_name)
            .ok_or(format!("Server '{}' not connected", server_name))?;

        debug!(server_name = %server_name, "Fetching tools with details from MCP server");
        let tools_response = client.list_tools(Default::default()).await?;

        let tools: Vec<ToolInfo> = tools_response.tools
            .iter()
            .map(|tool| ToolInfo {
                name: tool.name.to_string(),
                description: tool.description.as_ref().map(|d| d.to_string()),
            })
            .collect();

        info!(server_name = %server_name, tool_count = tools.len(), "Found tools with details from MCP server");
        Ok(tools)
    }

    pub fn add_server(&mut self, name: String, config: McpServerConfig) {
        self.config.add_server(name, config);
    }

    pub fn remove_server(&mut self, name: &str) -> Option<McpServerConfig> {
        self.disconnect_from_server(name);
        self.config.remove_server(name)
    }

    pub fn get_config(&self) -> &McpConfig {
        &self.config
    }

    pub async fn get_all_tools_for_openai(
        &self
    ) -> Result<Vec<ChatCompletionTool>, Box<dyn std::error::Error>> {
        let mut all_tools = Vec::new();

        for (server_name, client) in &self.clients {
            debug!(server_name = %server_name, "Getting tools from server");

            // Get actual tools from the MCP server
            match client.list_tools(Default::default()).await {
                Ok(tools_response) => {
                    for tool in tools_response.tools {
                        // Convert MCP tool to OpenAI ChatCompletionTool format
                        let description = tool.description
                            .as_ref()
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| {
                                format!(
                                    "Tool '{}' from MCP server '{}'",
                                    tool.name,
                                    server_name
                                )
                            });
                        
                        let parameters = serde_json::Value::Object(tool.input_schema.as_ref().clone());
                        
                        let openai_tool = ChatCompletionTool {
                            function: FunctionObjectArgs::default()
                                .name(format!("{}_{}", server_name, tool.name))
                                .description(description)
                                .parameters(parameters)
                                .build()?,  
                        };
                        all_tools.push(openai_tool);
                    }
                }
                Err(e) => {
                    warn!(server_name = %server_name, error = %e, "Failed to get tools from server");
                    continue;
                }
            }
        }

        info!(tool_count = all_tools.len(), "Total MCP tools available");
        Ok(all_tools)
    }

    pub async fn call_mcp_tool(
        &self,
        tool_name: &str,
        arguments: Option<serde_json::Map<String, Value>>
    ) -> Result<String, Box<dyn std::error::Error>> {
        // Parse server name and actual tool name from the prefixed name
        let parts: Vec<&str> = tool_name.splitn(2, '_').collect();
        if parts.len() != 2 {
            return Err("Invalid tool name format. Expected: server_toolname".into());
        }

        let server_name = parts[0];
        let actual_tool_name = parts[1];

        let client = self.clients
            .get(server_name)
            .ok_or(format!("Server '{}' not connected", server_name))?;

        info!(tool_name = %actual_tool_name, server_name = %server_name, arguments = ?arguments, "Calling MCP tool");

        // Call the actual MCP tool
        let result = client.call_tool(CallToolRequestParam {
            name: actual_tool_name.to_string().into(),
            arguments,
        }).await?;

        // Convert MCP result to string
        let result_str = if let Some(content_vec) = result.content.as_ref() {
            if !content_vec.is_empty() {
                // Extract text content from MCP response (using debug format for now and parse)
                let debug_str = format!("{:#?}", content_vec);

                // Try to extract text field from the debug output
                let text_lines: Vec<&str> = debug_str
                    .lines()
                    .filter_map(|line| {
                        if line.trim_start().starts_with("text:") {
                            // Extract the text between quotes
                            let trimmed = line.trim();
                            if let Some(start) = trimmed.find('"') {
                                if let Some(end) = trimmed.rfind('"') {
                                    if end > start {
                                        return Some(&trimmed[start + 1..end]);
                                    }
                                }
                            }
                        }
                        None
                    })
                    .collect();

                if text_lines.is_empty() {
                    // Fallback to debug format if we can't parse
                    debug_str
                } else {
                    text_lines.join("\n")
                }
            } else {
                "Empty content returned from tool".to_string()
            }
        } else {
            "No content returned from tool".to_string()
        };

        debug!(tool_name = %actual_tool_name, result = %result_str, "MCP tool execution completed");
        Ok(result_str)
    }
}
