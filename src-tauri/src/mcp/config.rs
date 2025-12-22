use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    // For stdio transport
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    
    // For SSE and HTTP transports
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    
    // Auto-connect on startup
    #[serde(default)]
    pub auto_connect: bool,
}

impl McpServerConfig {
    /// Automatically detect transport type based on configuration
    pub fn get_transport_type(&self) -> TransportType {
        if let Some(_command) = &self.command {
            TransportType::Stdio
        } else if let Some(url) = &self.url {
            if url.ends_with("/sse") {
                TransportType::Sse
            } else if url.ends_with("/mcp") {
                TransportType::StreamableHttp
            } else {
                // Default to SSE for other URLs
                TransportType::Sse
            }
        } else {
            // If neither command nor url is specified, default to stdio
            TransportType::Stdio
        }
    }
    
    pub fn validate(&self) -> Result<(), String> {
        match self.get_transport_type() {
            TransportType::Stdio => {
                if self.command.is_none() {
                    return Err("Stdio transport requires 'command' field".to_string());
                }
            }
            TransportType::Sse | TransportType::StreamableHttp => {
                if self.url.is_none() {
                    return Err("URL-based transport requires 'url' field".to_string());
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransportType {
    Stdio,
    Sse,
    StreamableHttp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            mcp_servers: HashMap::new(),
        }
    }
}

impl McpConfig {
    pub fn load_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        if !path.exists() {
            return Ok(Self::default());
        }
        
        let content = fs::read_to_string(path)?;
        let config: McpConfig = serde_json::from_str(&content)?;
        Ok(config)
    }
    
    pub fn save_to_file(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
    
    pub fn get_config_path(_app_handle: &tauri::AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let home_dir = match std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
            Ok(home) => home,
            Err(_) => {
                return Err("Failed to get user home directory".into());
            }
        };
        let sparrow_dir = PathBuf::from(home_dir).join(".sparrow");
        Ok(sparrow_dir.join("mcp_config.json"))
    }
    
    pub fn add_server(&mut self, name: String, server: McpServerConfig) {
        self.mcp_servers.insert(name, server);
    }
    
    pub fn remove_server(&mut self, name: &str) -> Option<McpServerConfig> {
        self.mcp_servers.remove(name)
    }
    
    pub fn get_server(&self, name: &str) -> Option<&McpServerConfig> {
        self.mcp_servers.get(name)
    }
    
    pub fn list_servers(&self) -> Vec<(&String, &McpServerConfig)> {
        self.mcp_servers.iter().collect()
    }
}