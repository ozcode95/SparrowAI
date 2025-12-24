use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc, Duration, NaiveTime, Datelike};
use tauri::{AppHandle, Emitter};
use tracing::{info, error, debug};
use std::path::PathBuf;
use tokio::time::sleep;

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub action_type: ActionType,
    pub action_params: serde_json::Value,
    pub trigger_time: TriggerTime,
    pub repeat_interval: Option<RepeatInterval>,
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub run_count: u32,
    pub auto_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ActionType {
    ShowNotification { title: String, message: String },
    RunMcpFunction { server_name: String, tool_name: String, arguments: serde_json::Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TriggerTime {
    /// Run at a specific date and time
    DateTime { datetime: DateTime<Utc> },
    /// Run daily at a specific time (HH:MM format)
    Daily { time: String },
    /// Run weekly on a specific day at a specific time (0=Sunday, 6=Saturday)
    Weekly { day_of_week: u8, time: String },
    /// Run monthly on a specific day at a specific time (1-31)
    Monthly { day_of_month: u8, time: String },
    /// Run every N minutes
    EveryNMinutes { minutes: u32 },
    /// Run every N hours
    EveryNHours { hours: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatInterval {
    pub value: u32,
    pub unit: TimeUnit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimeUnit {
    Minutes,
    Hours,
    Days,
    Weeks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStorage {
    pub tasks: HashMap<String, Task>,
}

impl TaskStorage {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskExecutionLog {
    pub task_id: String,
    pub executed_at: DateTime<Utc>,
    pub status: ExecutionStatus,
    pub message: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionStatus {
    Success,
    Failed,
    Skipped,
}

// Global task scheduler state
static TASK_SCHEDULER: std::sync::OnceLock<Arc<Mutex<TaskScheduler>>> = std::sync::OnceLock::new();

pub struct TaskScheduler {
    tasks: HashMap<String, Task>,
    execution_logs: Vec<TaskExecutionLog>,
    app_handle: Option<AppHandle>,
}

impl TaskScheduler {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            execution_logs: Vec::new(),
            app_handle: None,
        }
    }

    pub fn set_app_handle(&mut self, handle: AppHandle) {
        self.app_handle = Some(handle);
    }

    pub fn add_task(&mut self, task: Task) {
        info!("Adding task: {} ({})", task.name, task.id);
        self.tasks.insert(task.id.clone(), task);
    }

    pub fn remove_task(&mut self, task_id: &str) -> Option<Task> {
        info!("Removing task: {}", task_id);
        self.tasks.remove(task_id)
    }

    pub fn get_task(&self, task_id: &str) -> Option<&Task> {
        self.tasks.get(task_id)
    }

    pub fn get_all_tasks(&self) -> Vec<Task> {
        self.tasks.values().cloned().collect()
    }

    pub fn update_task(&mut self, task: Task) {
        info!("Updating task: {} ({})", task.name, task.id);
        self.tasks.insert(task.id.clone(), task);
    }

    pub fn add_execution_log(&mut self, log: TaskExecutionLog) {
        // Keep only last 100 logs per task
        let task_logs: Vec<_> = self.execution_logs
            .iter()
            .filter(|l| l.task_id == log.task_id)
            .collect();
        
        if task_logs.len() >= 100 {
            self.execution_logs.retain(|l| l.task_id != log.task_id);
        }
        
        self.execution_logs.push(log);
    }

    pub fn get_task_logs(&self, task_id: &str) -> Vec<TaskExecutionLog> {
        self.execution_logs
            .iter()
            .filter(|log| log.task_id == task_id)
            .cloned()
            .collect()
    }

    pub fn calculate_next_run(&self, task: &Task) -> Option<DateTime<Utc>> {
        if !task.enabled {
            return None;
        }

        let now = Utc::now();
        
        match &task.trigger_time {
            TriggerTime::DateTime { datetime } => {
                if *datetime > now {
                    Some(*datetime)
                } else if let Some(interval) = &task.repeat_interval {
                    Some(Self::add_interval(*datetime, interval, now))
                } else {
                    None // One-time task that already ran
                }
            },
            TriggerTime::Daily { time } => {
                if let Ok(naive_time) = NaiveTime::parse_from_str(time, "%H:%M") {
                    let today = now.date_naive().and_time(naive_time);
                    let today_utc = DateTime::<Utc>::from_naive_utc_and_offset(today, Utc);
                    
                    if today_utc > now {
                        Some(today_utc)
                    } else {
                        // Schedule for tomorrow
                        Some(today_utc + Duration::days(1))
                    }
                } else {
                    error!("Invalid time format: {}", time);
                    None
                }
            },
            TriggerTime::Weekly { day_of_week, time } => {
                if let Ok(naive_time) = NaiveTime::parse_from_str(time, "%H:%M") {
                    let current_weekday = now.weekday().num_days_from_sunday() as u8;
                    let days_until_target = if *day_of_week >= current_weekday {
                        (*day_of_week - current_weekday) as i64
                    } else {
                        (7 - current_weekday + *day_of_week) as i64
                    };
                    
                    let target_date = now.date_naive() + Duration::days(days_until_target);
                    let target_datetime = target_date.and_time(naive_time);
                    let target_utc = DateTime::<Utc>::from_naive_utc_and_offset(target_datetime, Utc);
                    
                    if target_utc > now {
                        Some(target_utc)
                    } else {
                        // Schedule for next week
                        Some(target_utc + Duration::weeks(1))
                    }
                } else {
                    error!("Invalid time format: {}", time);
                    None
                }
            },
            TriggerTime::Monthly { day_of_month, time } => {
                if let Ok(naive_time) = NaiveTime::parse_from_str(time, "%H:%M") {
                    let target_day = *day_of_month;
                    
                    // Calculate target date in current month
                    let current_month_date = now.date_naive()
                        .with_day(target_day as u32)
                        .and_then(|d| Some(d.and_time(naive_time)));
                    
                    if let Some(target_datetime) = current_month_date {
                        let target_utc = DateTime::<Utc>::from_naive_utc_and_offset(target_datetime, Utc);
                        
                        if target_utc > now {
                            return Some(target_utc);
                        }
                    }
                    
                    // Schedule for next month
                    let next_month = if now.month() == 12 {
                        now.date_naive()
                            .with_year(now.year() + 1)
                            .and_then(|d| d.with_month(1))
                    } else {
                        now.date_naive().with_month(now.month() + 1)
                    };
                    
                    if let Some(next_month_date) = next_month {
                        if let Some(target_date) = next_month_date.with_day(target_day as u32) {
                            let target_datetime = target_date.and_time(naive_time);
                            return Some(DateTime::<Utc>::from_naive_utc_and_offset(target_datetime, Utc));
                        }
                    }
                    
                    error!("Failed to calculate next monthly run");
                    None
                } else {
                    error!("Invalid time format: {}", time);
                    None
                }
            },
            TriggerTime::EveryNMinutes { minutes } => {
                // If task has never run, start from now + interval
                // Otherwise, add interval to last run time
                if let Some(last_run) = task.last_run {
                    let next = last_run + Duration::minutes(*minutes as i64);
                    Some(if next > now { next } else { now + Duration::minutes(*minutes as i64) })
                } else {
                    Some(now + Duration::minutes(*minutes as i64))
                }
            },
            TriggerTime::EveryNHours { hours } => {
                // If task has never run, start from now + interval
                // Otherwise, add interval to last run time
                if let Some(last_run) = task.last_run {
                    let next = last_run + Duration::hours(*hours as i64);
                    Some(if next > now { next } else { now + Duration::hours(*hours as i64) })
                } else {
                    Some(now + Duration::hours(*hours as i64))
                }
            }
        }
    }

    fn add_interval(start: DateTime<Utc>, interval: &RepeatInterval, now: DateTime<Utc>) -> DateTime<Utc> {
        let duration = match interval.unit {
            TimeUnit::Minutes => Duration::minutes(interval.value as i64),
            TimeUnit::Hours => Duration::hours(interval.value as i64),
            TimeUnit::Days => Duration::days(interval.value as i64),
            TimeUnit::Weeks => Duration::weeks(interval.value as i64),
        };

        let mut next = start;
        while next <= now {
            next = next + duration;
        }
        next
    }
}

// Task storage path
fn get_tasks_path() -> Result<PathBuf, String> {
    paths::get_sparrow_dir()
        .map(|p| p.join("tasks.json"))
        .map_err(|e| e.to_string())
}

fn load_tasks_from_file() -> Result<TaskStorage, String> {
    let path = get_tasks_path()?;
    
    if !path.exists() {
        info!("Tasks file doesn't exist, creating new storage");
        return Ok(TaskStorage::new());
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read tasks file: {}", e))?;
    
    let storage: TaskStorage = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse tasks file: {}", e))?;
    
    info!("Loaded {} tasks from file", storage.tasks.len());
    Ok(storage)
}

fn save_tasks_to_file(storage: &TaskStorage) -> Result<(), String> {
    let path = get_tasks_path()?;
    
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create tasks directory: {}", e))?;
    }

    let content = serde_json::to_string_pretty(storage)
        .map_err(|e| format!("Failed to serialize tasks: {}", e))?;
    
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write tasks file: {}", e))?;
    
    debug!("Saved {} tasks to file", storage.tasks.len());
    Ok(())
}

// Tauri commands
#[tauri::command]
pub async fn create_task(
    name: String,
    action_type: ActionType,
    action_params: serde_json::Value,
    trigger_time: TriggerTime,
    repeat_interval: Option<RepeatInterval>,
    auto_delete: Option<bool>,
) -> Result<Task, String> {
    let task_id = uuid::Uuid::new_v4().to_string();
    
    let task = Task {
        id: task_id,
        name,
        enabled: true,
        action_type,
        action_params,
        trigger_time: trigger_time.clone(),
        repeat_interval: repeat_interval.clone(),
        created_at: Utc::now(),
        last_run: None,
        next_run: None,
        run_count: 0,
        auto_delete: auto_delete.unwrap_or(false),
    };

    // Calculate next run
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    let next_run = {
        let sched = scheduler.lock().unwrap();
        sched.calculate_next_run(&task)
    };

    let mut task = task;
    task.next_run = next_run;

    // Add to scheduler and save
    {
        let mut scheduler = scheduler.lock().unwrap();
        scheduler.add_task(task.clone());
        
        let storage = TaskStorage {
            tasks: scheduler.get_all_tasks().into_iter().map(|t| (t.id.clone(), t)).collect(),
        };
        save_tasks_to_file(&storage)?;
    }

    info!("Created task: {} ({})", task.name, task.id);
    Ok(task)
}

#[tauri::command]
pub async fn get_tasks() -> Result<Vec<Task>, String> {
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    let scheduler = scheduler.lock().unwrap();
    Ok(scheduler.get_all_tasks())
}

#[tauri::command]
pub async fn get_task(task_id: String) -> Result<Task, String> {
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    let scheduler = scheduler.lock().unwrap();
    scheduler.get_task(&task_id)
        .cloned()
        .ok_or_else(|| format!("Task not found: {}", task_id))
}

#[tauri::command]
pub async fn update_task(task: Task) -> Result<Task, String> {
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    
    // Recalculate next run
    let next_run = {
        let sched = scheduler.lock().unwrap();
        sched.calculate_next_run(&task)
    };

    let mut task = task;
    task.next_run = next_run;

    {
        let mut scheduler = scheduler.lock().unwrap();
        scheduler.update_task(task.clone());
        
        let storage = TaskStorage {
            tasks: scheduler.get_all_tasks().into_iter().map(|t| (t.id.clone(), t)).collect(),
        };
        save_tasks_to_file(&storage)?;
    }

    info!("Updated task: {} ({})", task.name, task.id);
    Ok(task)
}

#[tauri::command]
pub async fn delete_task(task_id: String) -> Result<(), String> {
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    
    {
        let mut scheduler = scheduler.lock().unwrap();
        scheduler.remove_task(&task_id)
            .ok_or_else(|| format!("Task not found: {}", task_id))?;
        
        let storage = TaskStorage {
            tasks: scheduler.get_all_tasks().into_iter().map(|t| (t.id.clone(), t)).collect(),
        };
        save_tasks_to_file(&storage)?;
    }

    info!("Deleted task: {}", task_id);
    Ok(())
}

#[tauri::command]
pub async fn toggle_task(task_id: String) -> Result<Task, String> {
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    
    let task = {
        let mut scheduler = scheduler.lock().unwrap();
        let mut task = scheduler.get_task(&task_id)
            .cloned()
            .ok_or_else(|| format!("Task not found: {}", task_id))?;
        
        task.enabled = !task.enabled;
        
        // Recalculate next run
        task.next_run = scheduler.calculate_next_run(&task);
        
        scheduler.update_task(task.clone());
        
        let storage = TaskStorage {
            tasks: scheduler.get_all_tasks().into_iter().map(|t| (t.id.clone(), t)).collect(),
        };
        save_tasks_to_file(&storage)?;
        
        task
    };

    info!("Toggled task: {} (enabled: {})", task_id, task.enabled);
    Ok(task)
}

#[tauri::command]
pub async fn execute_task_manually(task_id: String, app_handle: AppHandle) -> Result<(), String> {
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    
    let task = {
        let scheduler = scheduler.lock().unwrap();
        scheduler.get_task(&task_id)
            .cloned()
            .ok_or_else(|| format!("Task not found: {}", task_id))?
    };

    info!("Manually executing task: {} ({})", task.name, task.id);
    
    tokio::spawn(async move {
        execute_task_action(&task, app_handle).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn get_task_logs(task_id: String) -> Result<Vec<TaskExecutionLog>, String> {
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    let scheduler = scheduler.lock().unwrap();
    Ok(scheduler.get_task_logs(&task_id))
}

// Task execution
async fn execute_task_action(task: &Task, app_handle: AppHandle) {
    info!("Executing task action: {} ({})", task.name, task.id);
    
    let result = match &task.action_type {
        ActionType::ShowNotification { title, message } => {
            execute_show_notification(title, message, &app_handle).await
        },
        ActionType::RunMcpFunction { server_name, tool_name, arguments } => {
            execute_mcp_function(server_name, tool_name, arguments, &app_handle).await
        },
    };

    let execution_success = result.is_ok();
    
    let log = match result {
        Ok(msg) => TaskExecutionLog {
            task_id: task.id.clone(),
            executed_at: Utc::now(),
            status: ExecutionStatus::Success,
            message: Some(msg),
            error: None,
        },
        Err(err) => TaskExecutionLog {
            task_id: task.id.clone(),
            executed_at: Utc::now(),
            status: ExecutionStatus::Failed,
            message: None,
            error: Some(err.clone()),
        },
    };

    // Update task and save log
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    let should_delete = {
        let mut scheduler = scheduler.lock().unwrap();
        
        if let Some(mut updated_task) = scheduler.get_task(&task.id).cloned() {
            updated_task.last_run = Some(Utc::now());
            updated_task.run_count += 1;
            updated_task.next_run = scheduler.calculate_next_run(&updated_task);
            
            // Check if task should be auto-deleted (one-time task with auto_delete enabled)
            let should_delete = updated_task.auto_delete && 
                updated_task.next_run.is_none() && 
                execution_success;
            
            if !should_delete {
                scheduler.update_task(updated_task.clone());
            }
            
            let storage = TaskStorage {
                tasks: scheduler.get_all_tasks().into_iter().map(|t| (t.id.clone(), t)).collect(),
            };
            let _ = save_tasks_to_file(&storage);
            
            should_delete
        } else {
            false
        }
    };
    
    // Delete task if needed (outside the lock to avoid deadlock)
    if should_delete {
        info!("Auto-deleting one-time task: {} ({})", task.name, task.id);
        let mut scheduler = scheduler.lock().unwrap();
        scheduler.remove_task(&task.id);
        let storage = TaskStorage {
            tasks: scheduler.get_all_tasks().into_iter().map(|t| (t.id.clone(), t)).collect(),
        };
        let _ = save_tasks_to_file(&storage);
    }
    
    {
        let mut scheduler = scheduler.lock().unwrap();
        scheduler.add_execution_log(log.clone());
    }

    // Emit event to UI
    let _ = app_handle.emit("task-executed", log);
}

async fn execute_show_notification(title: &str, message: &str, app_handle: &AppHandle) -> Result<String, String> {
    info!("Executing ShowNotification action: {} - {}", title, message);
    
    // Show native Windows notification using Tauri plugin
    use tauri_plugin_notification::NotificationExt;
    
    app_handle
        .notification()
        .builder()
        .title(title)
        .body(message)
        .show()
        .map_err(|e| format!("Failed to show notification: {}", e))?;
    
    Ok(format!("Notification shown: {}", title))
}

async fn execute_mcp_function(
    server_name: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    app_handle: &AppHandle,
) -> Result<String, String> {
    info!("Executing MCP function: {}:{}", server_name, tool_name);
    
    // Check if this is a builtin tool
    if server_name == "builtin" {
        info!("Executing builtin tool: {}", tool_name);
        
        // Execute builtin tool directly using the command
        match crate::mcp::execute_builtin_tool(tool_name.to_string(), arguments.clone()).await {
            Ok(_result) => {
                info!("Builtin tool executed successfully");
                Ok(format!("Built-in function {} executed successfully", tool_name))
            },
            Err(e) => {
                error!("Failed to execute builtin tool: {}", e);
                Err(format!("Failed to execute built-in function: {}", e))
            }
        }
    } else {
        // Convert arguments to Map if it's an object
        let args_map = match arguments {
            serde_json::Value::Object(map) => Some(map.clone()),
            _ => None,
        };
        
        // Build full tool name with server prefix for MCP server tools
        let full_tool_name = format!("{}:{}", server_name, tool_name);
        
        // Call the MCP tool directly
        match crate::mcp::call_mcp_tool(
            app_handle.clone(),
            full_tool_name.clone(),
            args_map,
        ).await {
            Ok(result) => {
                info!("MCP tool executed successfully: {}", result);
                Ok(format!("MCP function {} executed successfully", full_tool_name))
            },
            Err(e) => {
                error!("Failed to execute MCP tool: {}", e);
                Err(format!("Failed to execute MCP function: {}", e))
            }
        }
    }
}

// Task scheduler loop
pub async fn start_task_scheduler(app_handle: AppHandle) {
    info!("Starting task scheduler");
    
    // Initialize scheduler
    let scheduler = TASK_SCHEDULER.get_or_init(|| Arc::new(Mutex::new(TaskScheduler::new())));
    
    // Set app handle
    {
        let mut sched = scheduler.lock().unwrap();
        sched.set_app_handle(app_handle.clone());
    }

    // Load tasks from file
    match load_tasks_from_file() {
        Ok(storage) => {
            let mut sched = scheduler.lock().unwrap();
            for task in storage.tasks.values() {
                sched.add_task(task.clone());
            }
            info!("Loaded {} tasks", storage.tasks.len());
        },
        Err(e) => {
            error!("Failed to load tasks: {}", e);
        }
    }

    // Start scheduler loop
    tokio::spawn(async move {
        loop {
            sleep(std::time::Duration::from_secs(1)).await; // Check every second for accurate timing
            
            let tasks_to_execute = {
                let scheduler = scheduler.lock().unwrap();
                let now = Utc::now();
                
                scheduler.get_all_tasks()
                    .into_iter()
                    .filter(|task| {
                        task.enabled && 
                        task.next_run.is_some() && 
                        task.next_run.unwrap() <= now
                    })
                    .collect::<Vec<_>>()
            };

            for task in tasks_to_execute {
                info!("Triggering scheduled task: {} ({})", task.name, task.id);
                let app_handle = app_handle.clone();
                tokio::spawn(async move {
                    execute_task_action(&task, app_handle).await;
                });
            }
        }
    });
}
