//! Background ticker for scheduled tasks.
//!
//! A single tokio task wakes up every [`TICK_INTERVAL`], scans the
//! `tasks` table, and fires anything whose trigger is due. Each fire
//! runs on its own spawned task so a slow action (long-running script,
//! flaky network) can't hold up the rest of the schedule.
//!
//! The scheduler is intentionally simple — it relies on the DB as its
//! source of truth and never holds a long-lived in-memory job list.
//! New tasks created from the UI or the `task.create` MCP tool are
//! picked up automatically on the next tick. The only in-memory state
//! is a [`HashSet`] of currently-running task ids, used to avoid
//! double-firing if a previous run is still in-flight when its next
//! schedule slot comes around.
//!
//! Cron expressions are evaluated in the **local** timezone — users
//! mean "every weekday 8am" the way their wall clock reads, not in
//! UTC. The `cron` crate's [`Schedule`] is timezone-generic so we feed
//! it `DateTime<Local>` derived from the stored RFC3339 timestamps.

use crate::events;
use crate::state::AppStateExt;
use crate::tasks::{self, Task, TaskTrigger};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
use cron::Schedule;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

/// How often the scheduler wakes up to look for due tasks. 30 s is
/// fine-grained enough for minute-precision cron expressions while
/// keeping the wakeup rate negligible.
const TICK_INTERVAL: Duration = Duration::from_secs(30);

/// Spawn the scheduler on the Tauri async runtime. Returns immediately
/// — the loop runs in the background for the lifetime of the app.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let running: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        tracing::info!(
            "tasks scheduler started (tick = {}s)",
            TICK_INTERVAL.as_secs()
        );
        // Fire every enabled `Startup` task once before the periodic
        // loop kicks in. Done up front so the user sees their startup
        // actions take effect immediately rather than after the first
        // sleep, and so the periodic ticker doesn't have to special-case
        // a one-shot trigger.
        if let Err(e) = run_startup_pass(&app, &running).await {
            tracing::warn!("tasks startup pass failed: {e:#}");
        }
        // Wait one tick before the first periodic scan so the rest of
        // `init` (OVMS hydration, etc.) finishes settling first. Not
        // strictly required, but it keeps startup logs tidy.
        tokio::time::sleep(TICK_INTERVAL).await;
        loop {
            if let Err(e) = tick(&app, &running).await {
                tracing::warn!("tasks scheduler tick failed: {e:#}");
            }
            tokio::time::sleep(TICK_INTERVAL).await;
        }
    });
}

/// One-shot "launch" pass: spawn every enabled `Startup` task. Unlike
/// the periodic tick this never marks a task as due via `is_due`
/// (`Startup` always returns `Ok(false)` there — we explicitly do not
/// want it firing again until the next process launch).
async fn run_startup_pass(app: &AppHandle, running: &Arc<Mutex<HashSet<String>>>) -> Result<()> {
    let state = app.zero();
    let tasks = tasks::list(&state.db).await?;
    let mut fired = 0usize;
    for task in tasks {
        if !task.enabled {
            continue;
        }
        if !matches!(task.trigger, TaskTrigger::Startup) {
            continue;
        }
        {
            let mut g = running.lock().await;
            if g.contains(&task.id) {
                continue;
            }
            g.insert(task.id.clone());
        }
        fired += 1;
        let app_for_fire = app.clone();
        let running_for_fire = Arc::clone(running);
        tauri::async_runtime::spawn(async move {
            fire(app_for_fire, task, running_for_fire).await;
        });
    }
    if fired > 0 {
        tracing::info!("tasks scheduler: dispatched {fired} startup task(s)");
    }
    Ok(())
}

/// One pass over the task list: enqueue every enabled task whose
/// trigger is due. Returns once the spawns are scheduled — the fires
/// themselves complete asynchronously.
async fn tick(app: &AppHandle, running: &Arc<Mutex<HashSet<String>>>) -> Result<()> {
    let state = app.zero();
    let tasks = tasks::list(&state.db).await?;
    let now = Utc::now();
    for task in tasks {
        if !task.enabled {
            continue;
        }
        match is_due(&task, now) {
            Ok(false) => continue,
            Err(e) => {
                tracing::warn!(
                    "tasks scheduler: cannot evaluate task {} ({}): {e:#}",
                    task.id,
                    task.name,
                );
                continue;
            }
            Ok(true) => {}
        }

        // Skip if a previous fire is still running. We don't queue up a
        // backlog — the next tick will pick the task up again once
        // the in-flight run finishes.
        {
            let mut g = running.lock().await;
            if g.contains(&task.id) {
                continue;
            }
            g.insert(task.id.clone());
        }

        let app_for_fire = app.clone();
        let running_for_fire = Arc::clone(running);
        tauri::async_runtime::spawn(async move {
            fire(app_for_fire, task, running_for_fire).await;
        });
    }
    Ok(())
}

/// Execute a single task: mark running, run the action, record the
/// outcome, drop the run-lock, and emit a `tasks://tick` so the UI
/// refreshes its row.
async fn fire(app: AppHandle, task: Task, running: Arc<Mutex<HashSet<String>>>) {
    let state = app.zero();
    if let Err(e) = tasks::record_run(&state.db, &task.id, "running").await {
        tracing::warn!(
            "scheduler: record_run(running) for {} failed: {e:#}",
            task.id
        );
    }

    let outcome = tasks::run_action(&app, &task.action).await;
    let status = if outcome.is_ok() { "ok" } else { "error" };
    if let Err(e) = tasks::record_run(&state.db, &task.id, status).await {
        tracing::warn!(
            "scheduler: record_run({status}) for {} failed: {e:#}",
            task.id
        );
    }
    match &outcome {
        Ok(summary) => tracing::info!(
            "scheduler: fired task {} ({}) -> {summary}",
            task.id,
            task.name
        ),
        Err(e) => tracing::warn!("scheduler: task {} ({}) failed: {e:#}", task.id, task.name),
    }

    running.lock().await.remove(&task.id);
    let _ = app.emit(events::TASKS_TICK, &task.id);
}

/// Decide whether `task` should fire at `now`. The baseline used for
/// "what was the last fire" is `last_run_at` if present, otherwise the
/// task's `created_at` — that way a brand-new cron task waits for the
/// next scheduled slot rather than firing immediately on the tick that
/// follows its creation.
fn is_due(task: &Task, now: DateTime<Utc>) -> Result<bool> {
    match &task.trigger {
        TaskTrigger::Manual => Ok(false),
        // The periodic ticker never fires `Startup` tasks — they're
        // dispatched once from [`run_startup_pass`] at boot and would
        // otherwise re-fire on every tick because there's no
        // "baseline" that progresses with time.
        TaskTrigger::Startup => Ok(false),
        TaskTrigger::Once { at } => {
            if task.last_run_at.is_some() {
                return Ok(false);
            }
            let due: DateTime<Utc> = DateTime::parse_from_rfc3339(at)
                .map_err(|e| anyhow!("invalid `once.at` timestamp `{at}`: {e}"))?
                .with_timezone(&Utc);
            Ok(due <= now)
        }
        TaskTrigger::Interval { seconds } => {
            let baseline = baseline_utc(task)?;
            let elapsed = now.signed_duration_since(baseline).num_seconds();
            Ok(elapsed >= *seconds as i64)
        }
        TaskTrigger::Cron { expr } => {
            // The `cron` crate expects 6 mandatory fields (with seconds);
            // the conventional UI cron is 5 fields. Normalise by
            // prepending a `0` seconds field when the user wrote the
            // shorter form.
            let normalized = normalize_cron(expr);
            let schedule = Schedule::from_str(&normalized)
                .map_err(|e| anyhow!("invalid cron `{expr}`: {e}"))?;
            // Work in local time so "0 8 * * *" means 8 AM on the
            // user's wall clock — what nearly everyone expects from
            // cron in a desktop app.
            let baseline_local = baseline_utc(task)?.with_timezone(&Local);
            let now_local = now.with_timezone(&Local);
            // `after` returns the next fire strictly after `baseline`.
            // If that fire has already passed, the task is due (we'll
            // catch up at most one slot — subsequent missed slots are
            // collapsed into a single fire on purpose).
            if let Some(next) = schedule.after(&baseline_local).next() {
                Ok(next <= now_local)
            } else {
                Ok(false)
            }
        }
    }
}

fn baseline_utc(task: &Task) -> Result<DateTime<Utc>> {
    let s = task.last_run_at.as_deref().unwrap_or(&task.created_at);
    DateTime::parse_from_rfc3339(s)
        .map_err(|e| anyhow!("invalid timestamp `{s}`: {e}"))
        .map(|dt| dt.with_timezone(&Utc))
}

fn normalize_cron(expr: &str) -> String {
    let parts = expr.split_whitespace().count();
    if parts == 5 {
        format!("0 {}", expr.trim())
    } else {
        expr.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskAction;
    use chrono::Duration as ChronoDuration;

    fn task_with(
        trigger: TaskTrigger,
        created_at: DateTime<Utc>,
        last_run_at: Option<DateTime<Utc>>,
    ) -> Task {
        Task {
            id: "t1".into(),
            name: "test".into(),
            description: String::new(),
            action: TaskAction::Notify {
                title: "x".into(),
                body: "y".into(),
            },
            trigger,
            enabled: true,
            last_run_at: last_run_at.map(|d| d.to_rfc3339()),
            last_status: None,
            created_at: created_at.to_rfc3339(),
        }
    }

    #[test]
    fn manual_is_never_due() {
        let now = Utc::now();
        let t = task_with(TaskTrigger::Manual, now - ChronoDuration::days(7), None);
        assert!(!is_due(&t, now).unwrap());
    }

    #[test]
    fn startup_is_never_due_in_periodic_tick() {
        // The periodic tick must never claim a `Startup` task is due —
        // those are dispatched once from `run_startup_pass` at boot. A
        // false `is_due` here would re-fire the task on every 30s tick.
        let now = Utc::now();
        let fresh = task_with(TaskTrigger::Startup, now - ChronoDuration::minutes(1), None);
        assert!(!is_due(&fresh, now).unwrap());
        let already_ran = task_with(
            TaskTrigger::Startup,
            now - ChronoDuration::hours(2),
            Some(now - ChronoDuration::hours(1)),
        );
        assert!(!is_due(&already_ran, now).unwrap());
    }

    #[test]
    fn once_fires_after_its_time_then_never_again() {
        let now = Utc::now();
        let past = now - ChronoDuration::minutes(5);
        let future = now + ChronoDuration::hours(1);

        let pending = task_with(
            TaskTrigger::Once {
                at: past.to_rfc3339(),
            },
            past,
            None,
        );
        assert!(is_due(&pending, now).unwrap());

        let already_ran = task_with(
            TaskTrigger::Once {
                at: past.to_rfc3339(),
            },
            past,
            Some(now - ChronoDuration::minutes(1)),
        );
        assert!(!is_due(&already_ran, now).unwrap());

        let not_yet = task_with(
            TaskTrigger::Once {
                at: future.to_rfc3339(),
            },
            now,
            None,
        );
        assert!(!is_due(&not_yet, now).unwrap());
    }

    #[test]
    fn interval_waits_one_full_period_before_first_fire() {
        let now = Utc::now();
        let just_made = task_with(
            TaskTrigger::Interval { seconds: 3600 },
            now - ChronoDuration::minutes(10),
            None,
        );
        assert!(!is_due(&just_made, now).unwrap());

        let overdue = task_with(
            TaskTrigger::Interval { seconds: 3600 },
            now - ChronoDuration::hours(2),
            None,
        );
        assert!(is_due(&overdue, now).unwrap());
    }

    #[test]
    fn interval_uses_last_run_as_baseline_after_first_fire() {
        let now = Utc::now();
        let recently_ran = task_with(
            TaskTrigger::Interval { seconds: 600 },
            now - ChronoDuration::days(1),
            Some(now - ChronoDuration::minutes(5)),
        );
        assert!(!is_due(&recently_ran, now).unwrap());

        let due_again = task_with(
            TaskTrigger::Interval { seconds: 600 },
            now - ChronoDuration::days(1),
            Some(now - ChronoDuration::minutes(15)),
        );
        assert!(is_due(&due_again, now).unwrap());
    }

    #[test]
    fn cron_normaliser_pads_seconds_for_five_field_input() {
        assert_eq!(normalize_cron("0 8 * * *"), "0 0 8 * * *");
        // Six-field input is left alone.
        assert_eq!(normalize_cron("30 0 8 * * *"), "30 0 8 * * *");
        // Extra whitespace gets trimmed off the boundaries.
        assert_eq!(normalize_cron("  */5 * * * *  "), "0 */5 * * * *");
    }

    #[test]
    fn invalid_cron_propagates_as_error() {
        let now = Utc::now();
        let t = task_with(
            TaskTrigger::Cron {
                expr: "not-a-cron".into(),
            },
            now - ChronoDuration::hours(1),
            None,
        );
        assert!(is_due(&t, now).is_err());
    }

    #[test]
    fn cron_fires_when_a_slot_has_passed_since_baseline() {
        // Every minute, baseline 10 minutes ago, "now" is now — at
        // least one slot has fired so the task is due.
        let now = Utc::now();
        let t = task_with(
            TaskTrigger::Cron {
                expr: "* * * * *".into(),
            },
            now - ChronoDuration::minutes(10),
            None,
        );
        assert!(is_due(&t, now).unwrap());
    }
}
