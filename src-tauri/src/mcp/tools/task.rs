//! Task-creation built-in tool.
//!
//! Lets the agent persist a new entry in the [`crate::tasks`] table —
//! the same storage the Tasks page in the UI reads from. Triggers map
//! one-to-one to [`crate::tasks::TaskTrigger`] and actions to
//! [`crate::tasks::TaskAction`] so the model can ask for cron / interval
//! / manual / once schedules paired with a command / script / notify /
//! prompt action without us inventing a new vocabulary.
//!
//! The tool is marked **destructive** because a recurring task is a
//! durable side-effect that runs commands or prompts on a schedule once
//! the scheduler lands (phase 5). Today the row is just persisted; even
//! so, the confirm gate keeps the model from filling the user's queue
//! with unintended jobs.

use crate::mcp::{Tool, ToolResult, ToolSchema};
use crate::state::AppStateExt;
use crate::tasks::{self, NewTask, TaskAction, TaskTrigger};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tauri::AppHandle;

#[derive(Debug)]
pub struct TaskCreate {
    db: SqlitePool,
}

impl TaskCreate {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            db: app.zero().db.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CreateArgs {
    name: String,
    #[serde(default)]
    description: Option<String>,
    /// Preferred shape: tagged `TaskAction` JSON. When omitted we fall
    /// back to wrapping the legacy `prompt` string as an agent prompt
    /// action so older callers (and the prior tool schema) keep working.
    #[serde(default)]
    action: Option<TaskAction>,
    #[serde(default)]
    prompt: Option<String>,
    trigger: TaskTrigger,
    /// Defaults to `true` so a freshly-created task is live as soon as
    /// the scheduler picks it up; pass `false` to stage one for later
    /// manual enabling.
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[async_trait]
impl Tool for TaskCreate {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "task.create".into(),
            description: "Persist a new scheduled task. `trigger` is a \
                 tagged object: \
                 {\"kind\":\"cron\",\"expr\":\"0 9 * * *\"}, \
                 {\"kind\":\"interval\",\"seconds\":3600}, \
                 {\"kind\":\"manual\"}, or \
                 {\"kind\":\"once\",\"at\":\"2025-01-01T09:00:00Z\"}. \
                 `action` is also tagged: \
                 {\"kind\":\"command\",\"program\":\"...\",\"args\":[...]}, \
                 {\"kind\":\"script\",\"path\":\"...\",\"interpreter\":\"...\"}, \
                 {\"kind\":\"notify\",\"title\":\"...\",\"body\":\"...\"}, or \
                 {\"kind\":\"prompt\",\"prompt\":\"...\",\"notify\":true}. \
                 For backwards compatibility you may pass `prompt` instead \
                 of `action` — it is wrapped as a prompt action. \
                 The returned id can be passed to other task tools. \
                 This is destructive — the user is prompted before each \
                 call unless the global confirm gate is disabled."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["name", "trigger"],
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Short human-readable task name."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional longer description shown in the UI."
                    },
                    "action": {
                        "type": "object",
                        "description": "Tagged action object (`kind` field discriminates). \
                                        Pick one of: command, script, notify, prompt.",
                        "oneOf": [
                            {
                                "type": "object",
                                "required": ["kind", "program"],
                                "properties": {
                                    "kind": { "const": "command" },
                                    "program": { "type": "string" },
                                    "args": {
                                        "type": "array",
                                        "items": { "type": "string" }
                                    },
                                    "cwd": { "type": ["string", "null"] }
                                }
                            },
                            {
                                "type": "object",
                                "required": ["kind", "path"],
                                "properties": {
                                    "kind": { "const": "script" },
                                    "path": { "type": "string" },
                                    "interpreter": { "type": ["string", "null"] },
                                    "cwd": { "type": ["string", "null"] }
                                }
                            },
                            {
                                "type": "object",
                                "required": ["kind", "title", "body"],
                                "properties": {
                                    "kind": { "const": "notify" },
                                    "title": { "type": "string" },
                                    "body": { "type": "string" }
                                }
                            },
                            {
                                "type": "object",
                                "required": ["kind", "prompt"],
                                "properties": {
                                    "kind": { "const": "prompt" },
                                    "prompt": { "type": "string" },
                                    "notify": { "type": "boolean" }
                                }
                            }
                        ]
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Legacy shortcut: wrapped as a prompt action when \
                                        `action` is not provided."
                    },
                    "trigger": {
                        "type": "object",
                        "description": "Tagged trigger object (`kind` field discriminates).",
                        "oneOf": [
                            {
                                "type": "object",
                                "required": ["kind", "expr"],
                                "properties": {
                                    "kind": { "const": "cron" },
                                    "expr": { "type": "string" }
                                }
                            },
                            {
                                "type": "object",
                                "required": ["kind", "seconds"],
                                "properties": {
                                    "kind": { "const": "interval" },
                                    "seconds": { "type": "integer", "minimum": 1 }
                                }
                            },
                            {
                                "type": "object",
                                "required": ["kind"],
                                "properties": {
                                    "kind": { "const": "manual" }
                                }
                            },
                            {
                                "type": "object",
                                "required": ["kind", "at"],
                                "properties": {
                                    "kind": { "const": "once" },
                                    "at": { "type": "string", "description": "RFC3339 timestamp." }
                                }
                            }
                        ]
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Whether the task is active immediately. Default true."
                    }
                }
            }),
            destructive: true,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: CreateArgs = serde_json::from_value(args).context("task.create: parse arguments")?;
        if a.name.trim().is_empty() {
            return Ok(ToolResult {
                content: "task.create: `name` is empty".into(),
                is_error: true,
            });
        }
        let action = match (a.action, a.prompt) {
            (Some(act), _) => act,
            (None, Some(p)) if !p.trim().is_empty() => TaskAction::Prompt {
                prompt: p,
                notify: false,
            },
            _ => {
                return Ok(ToolResult {
                    content: "task.create: provide either `action` or `prompt`".into(),
                    is_error: true,
                });
            }
        };

        let new = NewTask {
            name: a.name,
            description: a.description.unwrap_or_default(),
            action,
            trigger: a.trigger,
            enabled: a.enabled,
        };
        let trigger_summary = describe_trigger(&new.trigger);
        let action_summary = describe_action(&new.action);
        match tasks::create(&self.db, new).await {
            Ok(id) => Ok(ToolResult {
                content: format!("created task `{id}` ({action_summary}, {trigger_summary})"),
                is_error: false,
            }),
            Err(e) => Ok(ToolResult {
                content: format!("task.create: failed to insert row: {e}"),
                is_error: true,
            }),
        }
    }
}

/// One-line description of a trigger for the success message. Keeping
/// this in sync with the JSON shape is enough — the UI renders the
/// canonical form from `trigger_json` itself.
fn describe_trigger(t: &TaskTrigger) -> String {
    match t {
        TaskTrigger::Cron { expr } => format!("cron `{expr}`"),
        TaskTrigger::Interval { seconds } => format!("every {seconds}s"),
        TaskTrigger::Manual => "manual".to_string(),
        TaskTrigger::Once { at } => format!("once at {at}"),
        TaskTrigger::Startup => "on app startup".to_string(),
    }
}

fn describe_action(a: &TaskAction) -> String {
    match a {
        TaskAction::Command { program, .. } => format!("run `{program}`"),
        TaskAction::Script { path, .. } => format!("run script `{path}`"),
        TaskAction::Notify { title, .. } => format!("notify `{title}`"),
        TaskAction::Prompt { .. } => "agent prompt".to_string(),
    }
}

pub fn all(app: &AppHandle) -> Vec<Box<dyn Tool>> {
    vec![Box::new(TaskCreate::new(app))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_trigger_covers_every_variant() {
        assert_eq!(
            describe_trigger(&TaskTrigger::Cron {
                expr: "0 9 * * *".into()
            }),
            "cron `0 9 * * *`"
        );
        assert_eq!(
            describe_trigger(&TaskTrigger::Interval { seconds: 60 }),
            "every 60s"
        );
        assert_eq!(describe_trigger(&TaskTrigger::Manual), "manual");
        assert_eq!(
            describe_trigger(&TaskTrigger::Once {
                at: "2025-01-01T09:00:00Z".into()
            }),
            "once at 2025-01-01T09:00:00Z"
        );
        assert_eq!(describe_trigger(&TaskTrigger::Startup), "on app startup");
    }

    #[test]
    fn create_args_accept_every_trigger_shape() {
        let cron = serde_json::from_value::<CreateArgs>(json!({
            "name": "n", "prompt": "p",
            "trigger": {"kind": "cron", "expr": "* * * * *"}
        }));
        assert!(cron.is_ok());

        let interval = serde_json::from_value::<CreateArgs>(json!({
            "name": "n", "prompt": "p",
            "trigger": {"kind": "interval", "seconds": 30}
        }));
        assert!(interval.is_ok());

        let manual = serde_json::from_value::<CreateArgs>(json!({
            "name": "n", "prompt": "p",
            "trigger": {"kind": "manual"}
        }));
        assert!(manual.is_ok());

        let once = serde_json::from_value::<CreateArgs>(json!({
            "name": "n", "prompt": "p",
            "trigger": {"kind": "once", "at": "2025-01-01T00:00:00Z"}
        }));
        assert!(once.is_ok());

        let startup = serde_json::from_value::<CreateArgs>(json!({
            "name": "n", "prompt": "p",
            "trigger": {"kind": "startup"}
        }));
        assert!(startup.is_ok());
    }

    #[test]
    fn create_args_default_enabled_to_true() {
        let a: CreateArgs = serde_json::from_value(json!({
            "name": "n", "prompt": "p",
            "trigger": {"kind": "manual"}
        }))
        .unwrap();
        assert!(a.enabled);
    }

    #[test]
    fn create_args_accept_tagged_action_shapes() {
        let cmd = serde_json::from_value::<CreateArgs>(json!({
            "name": "n",
            "action": {"kind": "command", "program": "git", "args": ["status"]},
            "trigger": {"kind": "manual"}
        }))
        .unwrap();
        assert!(matches!(cmd.action, Some(TaskAction::Command { .. })));

        let notify = serde_json::from_value::<CreateArgs>(json!({
            "name": "n",
            "action": {"kind": "notify", "title": "hi", "body": "there"},
            "trigger": {"kind": "manual"}
        }))
        .unwrap();
        assert!(matches!(notify.action, Some(TaskAction::Notify { .. })));
    }
}
