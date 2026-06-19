//! Task scheduler + queue.
//!
//! Tasks pair a trigger (cron / interval / manual / once) with an action.
//! Persistence lives in the `tasks` table; the action payload is stored as
//! JSON in `action_json` so we can grow new action variants without further
//! schema changes.
//!
//! The cron/interval ticker that actually fires due tasks lives in the
//! [`scheduler`] submodule and is spawned once from [`crate::state::AppState::init`].
//! The frontend's "Run" button drives [`run_action`] directly via the
//! `tasks_run_now` command for on-demand fires.

pub mod scheduler;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskTrigger {
    Cron {
        expr: String,
    },
    Interval {
        seconds: u64,
    },
    Manual,
    Once {
        at: String,
    },
    /// Fires once each time the app launches. The scheduler runs every
    /// enabled `Startup` task on init and then never again for that
    /// process. There's no de-duplication across launches — quitting
    /// and re-opening zero is the intended way to re-fire.
    Startup,
}

/// What a task actually *does* when it fires.
///
/// Serialised to JSON in the `action_json` column. The `kind` tag drives
/// the discriminator on both sides — keep the snake_case names in lockstep
/// with `src/stores/tasks.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskAction {
    /// Run an arbitrary executable. `program` is invoked directly (no shell);
    /// use [`TaskAction::Script`] if you need the OS shell to resolve the
    /// interpreter.
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    /// Run a script file. If `interpreter` is set we exec it as
    /// `<interpreter> <path>`; otherwise the script is launched directly
    /// and the OS decides what to do (shebang on unix, file association on
    /// Windows).
    Script {
        path: String,
        #[serde(default)]
        interpreter: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    /// Fire an OS notification (toast on Windows, banner on macOS, libnotify
    /// on Linux) via `tauri-plugin-notification`.
    Notify { title: String, body: String },
    /// Run an agent prompt. Hook into the chat runner is still TODO; for now
    /// this variant round-trips through storage but does nothing on
    /// [`run_action`]. `notify` indicates whether the eventual result should
    /// be surfaced as an OS notification when the runner lands.
    Prompt {
        prompt: String,
        #[serde(default)]
        notify: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub description: String,
    pub action: TaskAction,
    pub trigger: TaskTrigger,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTask {
    pub name: String,
    pub description: String,
    pub action: TaskAction,
    pub trigger: TaskTrigger,
    pub enabled: bool,
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Task>> {
    let rows = sqlx::query(
        "SELECT id, name, description, action_json, trigger_json, enabled,
                last_run_at, last_status, created_at
         FROM tasks ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let trigger_json: String = r.get("trigger_json");
        let trigger: TaskTrigger = serde_json::from_str(&trigger_json)?;
        let action_json: String = r.get("action_json");
        let action: TaskAction = serde_json::from_str(&action_json)?;
        let enabled: i64 = r.get("enabled");
        out.push(Task {
            id: r.get("id"),
            name: r.get("name"),
            description: r.get("description"),
            action,
            trigger,
            enabled: enabled != 0,
            last_run_at: nullable_text(&r, "last_run_at"),
            last_status: nullable_text(&r, "last_status"),
            created_at: r.get("created_at"),
        });
    }
    Ok(out)
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Task> {
    let r = sqlx::query(
        "SELECT id, name, description, action_json, trigger_json, enabled,
                last_run_at, last_status, created_at
         FROM tasks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("task {id} not found"))?;

    let trigger_json: String = r.get("trigger_json");
    let trigger: TaskTrigger = serde_json::from_str(&trigger_json)?;
    let action_json: String = r.get("action_json");
    let action: TaskAction = serde_json::from_str(&action_json)?;
    let enabled: i64 = r.get("enabled");
    Ok(Task {
        id: r.get("id"),
        name: r.get("name"),
        description: r.get("description"),
        action,
        trigger,
        enabled: enabled != 0,
        last_run_at: nullable_text(&r, "last_run_at"),
        last_status: nullable_text(&r, "last_status"),
        created_at: r.get("created_at"),
    })
}

pub async fn create(pool: &SqlitePool, t: NewTask) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let trigger_json = serde_json::to_string(&t.trigger)?;
    let action_json = serde_json::to_string(&t.action)?;
    sqlx::query(
        "INSERT INTO tasks
            (id, name, description, action_json, trigger_json, enabled, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&t.name)
    .bind(&t.description)
    .bind(&action_json)
    .bind(&trigger_json)
    .bind(if t.enabled { 1 } else { 0 })
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn update(pool: &SqlitePool, t: Task) -> Result<()> {
    let trigger_json = serde_json::to_string(&t.trigger)?;
    let action_json = serde_json::to_string(&t.action)?;
    sqlx::query(
        "UPDATE tasks SET name = ?, description = ?, action_json = ?,
                          trigger_json = ?, enabled = ?
         WHERE id = ?",
    )
    .bind(&t.name)
    .bind(&t.description)
    .bind(&action_json)
    .bind(&trigger_json)
    .bind(if t.enabled { 1 } else { 0 })
    .bind(&t.id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_enabled(pool: &SqlitePool, id: &str, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE tasks SET enabled = ? WHERE id = ?")
        .bind(if enabled { 1 } else { 0 })
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Stamp the `last_run_at` / `last_status` columns. Status is one of
/// `"ok"`, `"error"`, `"running"`.
pub async fn record_run(pool: &SqlitePool, id: &str, status: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE tasks SET last_run_at = ?, last_status = ? WHERE id = ?")
        .bind(&now)
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Execute a [`TaskAction`] right now and return a short human-readable
/// description of what happened — useful for logs and for surfacing
/// success/failure context to the UI later.
///
/// Notification delivery for the [`TaskAction::Prompt`] variant is gated on
/// the chat runner integration (still TODO); today it returns an error so
/// the UI marks the run as failed rather than silently no-op'ing.
pub async fn run_action(app: &AppHandle, action: &TaskAction) -> Result<String> {
    match action {
        TaskAction::Command { program, args, cwd } => {
            let mut cmd = tokio::process::Command::new(program);
            cmd.args(args);
            if let Some(dir) = cwd.as_deref().filter(|s| !s.is_empty()) {
                cmd.current_dir(dir);
            }
            let out = cmd
                .output()
                .await
                .map_err(|e| anyhow!("spawn `{program}` failed: {e}"))?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(anyhow!(
                    "`{program}` exited with {}: {}",
                    out.status,
                    stderr.trim()
                ));
            }
            Ok(format!("`{program}` exited 0"))
        }
        TaskAction::Script {
            path,
            interpreter,
            cwd,
        } => {
            let mut cmd = match interpreter.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(int) => {
                    let mut c = tokio::process::Command::new(int);
                    c.arg(path);
                    c
                }
                None => tokio::process::Command::new(path),
            };
            if let Some(dir) = cwd.as_deref().filter(|s| !s.is_empty()) {
                cmd.current_dir(dir);
            }
            let out = cmd
                .output()
                .await
                .map_err(|e| anyhow!("spawn script `{path}` failed: {e}"))?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(anyhow!(
                    "script `{path}` exited with {}: {}",
                    out.status,
                    stderr.trim()
                ));
            }
            Ok(format!("script `{path}` exited 0"))
        }
        TaskAction::Notify { title, body } => {
            app.notification()
                .builder()
                .title(title)
                .body(body)
                .show()
                .map_err(|e| anyhow!("notification failed: {e}"))?;
            Ok("notification shown".into())
        }
        TaskAction::Prompt { .. } => {
            // The chat runner integration is the next phase — see
            // `agent::run`. Until that lands we explicitly fail so the
            // UI shows a clear "this isn't wired up yet" state instead
            // of a false-positive success.
            Err(anyhow!(
                "prompt actions are not yet wired up to the agent runner"
            ))
        }
    }
}

/// Read a nullable TEXT column, normalising both NULL and the empty
/// string to `None`. SQLite's `sqlite3_column_text` returns an empty
/// pointer for NULL, which sqlx's `String` decoder surfaces as
/// `Ok("")` rather than an error — so a plain `.try_get::<String, _>`
/// silently produces `Some("")` on NULL. Using the `Option<String>`
/// decoder explicitly avoids that, and we additionally treat `""` as
/// missing so downstream code never has to parse an empty timestamp.
fn nullable_text(row: &sqlx::sqlite::SqliteRow, col: &str) -> Option<String> {
    row.try_get::<Option<String>, _>(col)
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}
