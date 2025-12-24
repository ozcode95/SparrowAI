use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use sysinfo::System;
use chrono::Local;
use std::fs;
use std::path::Path;

/// Represents a built-in MCP tool with its metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub hidden_from_task_creation: bool,
}

/// Result of executing a tool
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<ToolContent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

impl ToolResult {
    pub fn text(text: String) -> Self {
        Self {
            content: vec![ToolContent {
                content_type: "text".to_string(),
                text,
            }],
        }
    }
}

/// Registry of all built-in tools
pub struct BuiltinToolRegistry {
    tools: HashMap<String, BuiltinTool>,
}

impl BuiltinToolRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
        };
        registry.register_all_tools();
        registry
    }

    /// Convert built-in tools to OpenAI ChatCompletionTool format
    pub fn to_openai_tools(&self) -> Vec<async_openai::types::ChatCompletionTool> {
        use async_openai::types::{ChatCompletionTool, ChatCompletionToolType, FunctionObject};
        
        self.tools.values().map(|tool| {
            let tool_name = format!("builtin_{}", tool.name);
            tracing::debug!("Registering builtin tool for chat: {} (hidden_from_task_creation: {})", 
                tool_name, tool.hidden_from_task_creation);
            
            ChatCompletionTool {
                r#type: ChatCompletionToolType::Function,
                function: FunctionObject {
                    name: tool_name, // Prefix with "builtin_" to identify source
                    description: Some(tool.description.clone()),
                    parameters: Some(tool.input_schema.clone()),
                    strict: None,
                },
            }
        }).collect()
    }

    fn register_all_tools(&mut self) {
        // Tool 1: Get system information
        self.tools.insert(
            "get_system_info".to_string(),
            BuiltinTool {
                name: "get_system_info".to_string(),
                description: "Get detailed system information including CPU, GPU, RAM, and OS details".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
                hidden_from_task_creation: false,
            },
        );

        // Tool 2: Get current time
        self.tools.insert(
            "get_current_time".to_string(),
            BuiltinTool {
                name: "get_current_time".to_string(),
                description: "Get the current date and time in various formats".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "format": {
                            "type": "string",
                            "description": "Time format: 'iso8601' (default), 'unix', 'readable'",
                            "enum": ["iso8601", "unix", "readable"]
                        }
                    },
                    "required": []
                }),
                hidden_from_task_creation: false,
            },
        );

        // Tool 3: List directory contents
        self.tools.insert(
            "list_directory".to_string(),
            BuiltinTool {
                name: "list_directory".to_string(),
                description: "List files and subdirectories in a directory. Returns names, types (file/dir), and sizes".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The directory path to list"
                        },
                        "recursive": {
                            "type": "boolean",
                            "description": "Whether to list recursively (default: false)"
                        }
                    },
                    "required": ["path"]
                }),
                hidden_from_task_creation: false,
            },
        );

        // Tool 4: Create task from natural language prompt
        self.tools.insert(
            "create_task".to_string(),
            BuiltinTool {
                name: "create_task".to_string(),
                description: "Create a scheduled task with structured parameters. Supports notifications and MCP function execution on various schedules.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name of the task"
                        },
                        "action_type": {
                            "type": "object",
                            "description": "Action to perform. For ShowNotification: must include 'type', 'title', 'message'. For RunMcpFunction: must include 'type', 'server_name', 'tool_name', 'arguments'",
                            "oneOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "const": "ShowNotification"
                                        },
                                        "title": {
                                            "type": "string",
                                            "description": "Notification title"
                                        },
                                        "message": {
                                            "type": "string",
                                            "description": "Notification message"
                                        }
                                    },
                                    "required": ["type", "title", "message"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "const": "RunMcpFunction"
                                        },
                                        "server_name": {
                                            "type": "string",
                                            "description": "MCP server name (e.g., 'builtin' for builtin tools)"
                                        },
                                        "tool_name": {
                                            "type": "string",
                                            "description": "Tool/function name to execute"
                                        },
                                        "arguments": {
                                            "type": "object",
                                            "description": "Arguments to pass to the function"
                                        }
                                    },
                                    "required": ["type", "server_name", "tool_name"]
                                }
                            ]
                        },
                        "trigger_time": {
                            "type": "object",
                            "description": "When to run the task",
                            "oneOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "const": "DateTime"
                                        },
                                        "datetime": {
                                            "type": "string",
                                            "description": "ISO 8601 datetime string (e.g., '2023-10-10T15:25:00Z' or '2023-10-10T15:25:00')"
                                        }
                                    },
                                    "required": ["type", "datetime"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "const": "Daily"
                                        },
                                        "time": {
                                            "type": "string",
                                            "description": "Time in HH:MM format (24-hour)"
                                        }
                                    },
                                    "required": ["type", "time"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "const": "Weekly"
                                        },
                                        "day_of_week": {
                                            "type": "integer",
                                            "description": "Day of week (0=Sunday, 1=Monday, ..., 6=Saturday)",
                                            "minimum": 0,
                                            "maximum": 6
                                        },
                                        "time": {
                                            "type": "string",
                                            "description": "Time in HH:MM format (24-hour)"
                                        }
                                    },
                                    "required": ["type", "day_of_week", "time"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "const": "Monthly"
                                        },
                                        "day_of_month": {
                                            "type": "integer",
                                            "description": "Day of month (1-31)",
                                            "minimum": 1,
                                            "maximum": 31
                                        },
                                        "time": {
                                            "type": "string",
                                            "description": "Time in HH:MM format (24-hour)"
                                        }
                                    },
                                    "required": ["type", "day_of_month", "time"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "const": "EveryNMinutes"
                                        },
                                        "minutes": {
                                            "type": "integer",
                                            "description": "Number of minutes between executions",
                                            "minimum": 1
                                        }
                                    },
                                    "required": ["type", "minutes"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {
                                            "type": "string",
                                            "const": "EveryNHours"
                                        },
                                        "hours": {
                                            "type": "integer",
                                            "description": "Number of hours between executions",
                                            "minimum": 1
                                        }
                                    },
                                    "required": ["type", "hours"]
                                }
                            ]
                        },
                        "auto_delete": {
                            "type": "boolean",
                            "description": "Auto-delete task after one-time execution (default: false)"
                        }
                    },
                    "required": ["name", "action_type", "trigger_time"]
                }),
                hidden_from_task_creation: true,
            },
        );
    }

    pub fn list_tools(&self) -> Vec<BuiltinTool> {
        self.tools.values().cloned().collect()
    }

    #[allow(dead_code)]
    pub fn get_tool(&self, name: &str) -> Option<&BuiltinTool> {
        self.tools.get(name)
    }

    pub async fn execute_tool(&self, name: &str, arguments: Value) -> Result<ToolResult, String> {
        match name {
            "get_system_info" => execute_get_system_info().await,
            "get_current_time" => execute_get_current_time(arguments).await,
            "list_directory" => execute_list_directory(arguments).await,
            "create_task" => execute_create_task(arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        }
    }
}

// Tool implementations

async fn execute_get_system_info() -> Result<ToolResult, String> {
    let mut sys = System::new_all();
    sys.refresh_all();

    // CPU information
    let cpu_count = sys.cpus().len();
    let cpu_brand = sys.cpus().first()
        .map(|cpu| cpu.brand())
        .unwrap_or("Unknown");
    let cpu_frequency = sys.cpus().first()
        .map(|cpu| cpu.frequency())
        .unwrap_or(0);
    
    // Calculate average CPU usage
    let cpu_usage: f32 = sys.cpus().iter()
        .map(|cpu| cpu.cpu_usage())
        .sum::<f32>() / cpu_count as f32;

    // Memory information
    let total_memory = sys.total_memory();
    let used_memory = sys.used_memory();
    let available_memory = sys.available_memory();
    let memory_usage_percent = (used_memory as f64 / total_memory as f64) * 100.0;

    // System information
    let os_name = System::name().unwrap_or_else(|| "Unknown".to_string());
    let os_version = System::os_version().unwrap_or_else(|| "Unknown".to_string());
    let kernel_version = System::kernel_version().unwrap_or_else(|| "Unknown".to_string());
    let hostname = System::host_name().unwrap_or_else(|| "Unknown".to_string());

    // GPU information (basic - just detect if GPU libraries are available)
    let gpu_info = get_gpu_info();

    let info = json!({
        "system": {
            "hostname": hostname,
            "os": os_name,
            "os_version": os_version,
            "kernel_version": kernel_version,
            "architecture": std::env::consts::ARCH,
        },
        "cpu": {
            "brand": cpu_brand,
            "cores": cpu_count,
            "frequency_mhz": cpu_frequency,
            "usage_percent": format!("{:.1}", cpu_usage),
        },
        "memory": {
            "total_bytes": total_memory,
            "total_gb": format!("{:.2}", total_memory as f64 / 1024.0 / 1024.0 / 1024.0),
            "used_bytes": used_memory,
            "used_gb": format!("{:.2}", used_memory as f64 / 1024.0 / 1024.0 / 1024.0),
            "available_bytes": available_memory,
            "available_gb": format!("{:.2}", available_memory as f64 / 1024.0 / 1024.0 / 1024.0),
            "usage_percent": format!("{:.1}", memory_usage_percent),
        },
        "gpu": gpu_info,
    });

    Ok(ToolResult::text(serde_json::to_string_pretty(&info).unwrap()))
}

fn get_gpu_info() -> Value {
    // Basic GPU detection - can be enhanced with specific GPU libraries
    #[cfg(target_os = "windows")]
    {
        // On Windows, try to detect GPU via system info
        json!({
            "status": "detection_limited",
            "note": "GPU detection requires additional system queries. Using basic system info.",
        })
    }
    
    #[cfg(not(target_os = "windows"))]
    {
        json!({
            "status": "not_available",
            "note": "GPU detection not implemented for this platform"
        })
    }
}

async fn execute_get_current_time(arguments: Value) -> Result<ToolResult, String> {
    let format = arguments.get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("iso8601");

    let now = Local::now();

    let time_info = match format {
        "unix" => {
            json!({
                "timestamp": now.timestamp(),
                "timestamp_millis": now.timestamp_millis(),
                "format": "unix"
            })
        }
        "readable" => {
            json!({
                "datetime": now.format("%A, %B %e, %Y at %I:%M:%S %p %Z").to_string(),
                "date": now.format("%Y-%m-%d").to_string(),
                "time": now.format("%H:%M:%S").to_string(),
                "format": "readable"
            })
        }
        _ => { // iso8601 or default
            json!({
                "datetime": now.to_rfc3339(),
                "date": now.format("%Y-%m-%d").to_string(),
                "time": now.format("%H:%M:%S").to_string(),
                "timezone": now.format("%:z").to_string(),
                "format": "iso8601"
            })
        }
    };

    Ok(ToolResult::text(serde_json::to_string_pretty(&time_info).unwrap()))
}

async fn execute_list_directory(arguments: Value) -> Result<ToolResult, String> {
    let path_str = arguments.get("path")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'path' parameter")?;
    
    let recursive = arguments.get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let path = Path::new(path_str);
    
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path_str));
    }

    if !path.is_dir() {
        return Err(format!("Path is not a directory: {}", path_str));
    }

    let mut entries = Vec::new();

    if recursive {
        list_directory_recursive(path, "", &mut entries)?;
    } else {
        list_directory_simple(path, &mut entries)?;
    }

    let result = json!({
        "path": path_str,
        "recursive": recursive,
        "entry_count": entries.len(),
        "entries": entries,
    });

    Ok(ToolResult::text(serde_json::to_string_pretty(&result).unwrap()))
}

fn list_directory_simple(path: &Path, entries: &mut Vec<Value>) -> Result<(), String> {
    let dir_entries = fs::read_dir(path)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in dir_entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let metadata = entry.metadata().map_err(|e| format!("Failed to read metadata: {}", e))?;
        
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = metadata.is_dir();
        let size = if is_dir { None } else { Some(metadata.len()) };

        entries.push(json!({
            "name": name,
            "type": if is_dir { "directory" } else { "file" },
            "size_bytes": size,
        }));
    }

    // Sort by type (directories first) then by name
    entries.sort_by(|a, b| {
        let a_type = a.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let b_type = b.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let a_name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let b_name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
        
        b_type.cmp(a_type).then(a_name.cmp(b_name))
    });

    Ok(())
}

fn list_directory_recursive(path: &Path, prefix: &str, entries: &mut Vec<Value>) -> Result<(), String> {
    let dir_entries = fs::read_dir(path)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    let mut dir_items: Vec<_> = dir_entries.collect();
    dir_items.sort_by_key(|entry| {
        entry.as_ref().ok().and_then(|e| e.file_name().into_string().ok())
    });

    for entry in dir_items {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let metadata = entry.metadata().map_err(|e| format!("Failed to read metadata: {}", e))?;
        
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = metadata.is_dir();
        let relative_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };

        let size = if is_dir { None } else { Some(metadata.len()) };

        entries.push(json!({
            "name": name,
            "path": relative_path,
            "type": if is_dir { "directory" } else { "file" },
            "size_bytes": size,
        }));

        if is_dir {
            // Recurse into subdirectory
            let subpath = entry.path();
            list_directory_recursive(&subpath, &relative_path, entries)?;
        }
    }

    Ok(())
}

async fn execute_create_task(arguments: Value) -> Result<ToolResult, String> {
    use crate::tasks::{ActionType, TriggerTime};
    
    // Extract name
    let name = arguments.get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'name' parameter")?
        .to_string();
    
    // Extract and parse action_type
    let action_obj = arguments.get("action_type")
        .ok_or("Missing 'action_type' parameter")?;
    
    let action_type_str = action_obj.get("type")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'action_type.type' field")?;
    
    let action_type = match action_type_str {
        "ShowNotification" => {
            let title = action_obj.get("title")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'action_type.title' for ShowNotification")?
                .to_string();
            let message = action_obj.get("message")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'action_type.message' for ShowNotification")?
                .to_string();
            ActionType::ShowNotification { title, message }
        },
        "RunMcpFunction" => {
            let server_name = action_obj.get("server_name")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'action_type.server_name' for RunMcpFunction")?
                .to_string();
            let tool_name = action_obj.get("tool_name")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'action_type.tool_name' for RunMcpFunction")?
                .to_string();
            let arguments = action_obj.get("arguments")
                .cloned()
                .unwrap_or(json!({}));
            ActionType::RunMcpFunction { server_name, tool_name, arguments }
        },
        _ => return Err(format!("Invalid action_type: {}", action_type_str)),
    };
    
    // Extract and parse trigger_time
    let trigger_obj = arguments.get("trigger_time")
        .ok_or("Missing 'trigger_time' parameter")?;
    
    let trigger_type_str = trigger_obj.get("type")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'trigger_time.type' field")?;
    
    let trigger_time = match trigger_type_str {
        "DateTime" => {
            let datetime_str = trigger_obj.get("datetime")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'trigger_time.datetime' for DateTime trigger")?;
            
            // Try parsing as RFC3339 first (with timezone)
            let datetime = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(datetime_str) {
                dt.with_timezone(&chrono::Utc)
            } else {
                // Try parsing as naive datetime (without timezone), assume local timezone
                use chrono::{NaiveDateTime, TimeZone};
                let naive_dt = NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%dT%H:%M:%S")
                    .or_else(|_| NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%d %H:%M:%S"))
                    .map_err(|e| format!("Invalid datetime format (expected ISO 8601 with or without timezone): {}", e))?;
                chrono::Local.from_local_datetime(&naive_dt)
                    .single()
                    .ok_or("Ambiguous or invalid local datetime")?
                    .with_timezone(&chrono::Utc)
            };
            
            TriggerTime::DateTime { datetime }
        },
        "Daily" => {
            let time = trigger_obj.get("time")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'trigger_time.time' for Daily trigger")?
                .to_string();
            TriggerTime::Daily { time }
        },
        "Weekly" => {
            let day_of_week = trigger_obj.get("day_of_week")
                .and_then(|v| v.as_u64())
                .ok_or("Missing 'trigger_time.day_of_week' for Weekly trigger")? as u8;
            let time = trigger_obj.get("time")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'trigger_time.time' for Weekly trigger")?
                .to_string();
            TriggerTime::Weekly { day_of_week, time }
        },
        "Monthly" => {
            let day_of_month = trigger_obj.get("day_of_month")
                .and_then(|v| v.as_u64())
                .ok_or("Missing 'trigger_time.day_of_month' for Monthly trigger")? as u8;
            let time = trigger_obj.get("time")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'trigger_time.time' for Monthly trigger")?
                .to_string();
            TriggerTime::Monthly { day_of_month, time }
        },
        "EveryNMinutes" => {
            let minutes = trigger_obj.get("minutes")
                .and_then(|v| v.as_u64())
                .ok_or("Missing 'trigger_time.minutes' for EveryNMinutes trigger")? as u32;
            TriggerTime::EveryNMinutes { minutes }
        },
        "EveryNHours" => {
            let hours = trigger_obj.get("hours")
                .and_then(|v| v.as_u64())
                .ok_or("Missing 'trigger_time.hours' for EveryNHours trigger")? as u32;
            TriggerTime::EveryNHours { hours }
        },
        _ => return Err(format!("Invalid trigger_time.type: {}", trigger_type_str)),
    };
    
    // Extract auto_delete (optional)
    let auto_delete = arguments.get("auto_delete")
        .and_then(|v| v.as_bool());
    
    // Create the task using the tasks module
    let task = crate::tasks::create_task(
        name,
        action_type,
        json!({}),
        trigger_time,
        None,
        auto_delete,
    ).await?;

    let result = json!({
        "success": true,
        "task_id": task.id,
        "task_name": task.name,
        "message": format!("Task '{}' created successfully", task.name),
        "next_run": task.next_run,
    });

    Ok(ToolResult::text(serde_json::to_string_pretty(&result).unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_system_info() {
        let result = execute_get_system_info().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_get_current_time() {
        let result = execute_get_current_time(json!({})).await;
        assert!(result.is_ok());

        let result = execute_get_current_time(json!({"format": "unix"})).await;
        assert!(result.is_ok());
    }
}
