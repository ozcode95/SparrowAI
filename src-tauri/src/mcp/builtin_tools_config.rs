use crate::errors::{Result, SparrowError};
use crate::paths::get_sparrow_dir;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinToolsConfig {
    /// Map of tool name to enabled status
    pub tools: HashMap<String, bool>,
}

impl BuiltinToolsConfig {
    /// Create config from actual builtin tools registry
    pub fn from_registry(registry: &super::builtin_tools::BuiltinToolRegistry) -> Self {
        let mut tools = HashMap::new();
        
        // Get all tools from registry (includes both base tools and skills)
        for tool in registry.list_tools() {
            tools.insert(tool.name.clone(), true);
        }
        
        Self { tools }
    }
}

impl BuiltinToolsConfig {
    /// Get the path to the builtin_tools_config.json file
    pub fn config_path() -> Result<PathBuf> {
        Ok(get_sparrow_dir()?.join("builtin_tools_config.json"))
    }

    /// Load the config from disk, or create default if it doesn't exist
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        
        if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|e| SparrowError::Io(e))?;
            let config: BuiltinToolsConfig = serde_json::from_str(&content)
                .map_err(|e| SparrowError::OperationFailed { message: e.to_string() })?;
            
            tracing::info!("Loaded builtin tools config from {:?}", path);
            Ok(config)
        } else {
            tracing::info!("No builtin tools config found, will create on first use");
            // Return empty config - it will be populated when tools are registered
            Ok(Self { tools: HashMap::new() })
        }
    }
    
    /// Load or create config, merging with current registry
    pub fn load_and_merge(registry: &super::builtin_tools::BuiltinToolRegistry) -> Result<Self> {
        let mut config = Self::load()?;
        
        // Merge with current tools from registry to ensure new tools are present
        for tool in registry.list_tools() {
            config.tools.entry(tool.name.clone()).or_insert(true);
        }
        
        // Remove tools that no longer exist in registry
        let registry_tools: std::collections::HashSet<_> = 
            registry.list_tools().iter().map(|t| t.name.clone()).collect();
        config.tools.retain(|name, _| registry_tools.contains(name));
        
        config.save()?;
        Ok(config)
    }

    /// Save the config to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| SparrowError::Io(e))?;
        }

        let content = serde_json::to_string_pretty(self)
            .map_err(|e| SparrowError::OperationFailed { message: e.to_string() })?;
        fs::write(&path, content)
            .map_err(|e| SparrowError::Io(e))?;
        
        tracing::info!("Saved builtin tools config to {:?}", path);
        Ok(())
    }

    /// Check if a tool is enabled
    pub fn is_tool_enabled(&self, tool_name: &str) -> bool {
        self.tools.get(tool_name).copied().unwrap_or(true)
    }

    /// Set a tool's enabled status
    pub fn set_tool_enabled(&mut self, tool_name: String, enabled: bool) -> Result<()> {
        self.tools.insert(tool_name, enabled);
        self.save()
    }

    /// Get all enabled tool names
    pub fn get_enabled_tools(&self) -> Vec<String> {
        self.tools
            .iter()
            .filter(|(_, &enabled)| enabled)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

// Tauri commands
#[tauri::command]
pub fn get_builtin_tools_config() -> std::result::Result<BuiltinToolsConfig, String> {
    let registry = super::builtin_tools::BuiltinToolRegistry::new();
    BuiltinToolsConfig::load_and_merge(&registry).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_builtin_tools_config(config: BuiltinToolsConfig) -> std::result::Result<(), String> {
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_builtin_tool_enabled(tool_name: String, enabled: bool) -> std::result::Result<(), String> {
    let mut config = BuiltinToolsConfig::load().map_err(|e| e.to_string())?;
    config.set_tool_enabled(tool_name, enabled).map_err(|e| e.to_string())
}
