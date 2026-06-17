//! OVMS child-process management.
//!
//! Owns a `tokio::process::Child` plus two reader tasks that pump stdout /
//! stderr into `ovms://log` events, and a *monitor* task that owns the child
//! and waits for it to exit. The monitor fires the caller-supplied `on_exit`
//! callback with an [`ExitReason`] so the controller can distinguish a
//! deliberate shutdown from a crash and flip status accordingly.
//!
//! Shutdown semantics: requesting [`OvmsProcess::shutdown`] sends a kill
//! signal to the monitor, which then issues `child.kill()` and awaits the real
//! exit before the callback fires. Dropping the struct without calling
//! `shutdown` also kills the child (the oneshot sender is dropped, the
//! monitor's `kill_rx` resolves with `Err`, and it treats that the same way).
//! Either way `on_exit` is always invoked exactly once.
//!
//! We don't attempt a graceful SIGTERM / Ctrl-Break dance — OVMS owns no
//! external state we need to flush cleanly, so a hard kill keeps behaviour
//! predictable across platforms.

use crate::events;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize)]
pub struct OvmsLogLine {
    pub ts: String,
    pub level: String,
    pub line: String,
}

/// How a monitored OVMS instance ended its life.
#[derive(Debug, Clone, Copy)]
pub enum ExitReason {
    /// `shutdown()` was called (or the handle was dropped) and the kill
    /// signal was honoured. The controller should leave its existing status
    /// transitions (e.g. `Stopping` → `Stopped`) alone.
    Expected,
    /// Process died without us asking it to. Controller should flip to
    /// `Error` with the captured exit code in the message.
    Crashed { code: Option<i32> },
}

/// Live handle to a running OVMS instance.
///
/// Holding this struct keeps the child + log + monitor tasks alive; calling
/// `shutdown` (preferred) tears them down and awaits the real exit. Dropping
/// the struct still kills the child, but the `on_exit` callback may fire
/// asynchronously after the drop returns.
pub struct OvmsProcess {
    pid: u32,
    /// `Some` until `shutdown` is called. Dropping it (either via `shutdown`
    /// or via `Drop`) signals the monitor task to kill the child.
    kill_tx: Option<oneshot::Sender<()>>,
    /// Joined inside `shutdown`. The monitor owns the `Child` and is the only
    /// place we call `child.wait()`.
    monitor: Option<JoinHandle<()>>,
    /// Joined inside `shutdown`. Kept alive otherwise so the pump tasks keep
    /// draining stdout/stderr for the entire process lifetime.
    log_task: Option<JoinHandle<()>>,
    /// Bounded ring of the most-recent `ERROR`-classified log lines. Lets
    /// the controller pull the *actual* OVMS failure (e.g. a Mediapipe
    /// graph init error during model load) out of the log stream when
    /// readiness times out, instead of surfacing the generic
    /// `did not become ready` message on its own.
    error_buf: Arc<Mutex<VecDeque<String>>>,
}

/// Cap on retained error lines. OVMS prints a few-line stack trace per
/// failure, so we keep enough to capture one full one without letting a
/// crashing model accumulate megabytes of log here.
const ERROR_BUF_CAP: usize = 32;

impl OvmsProcess {
    /// Spawn `executable` with `args` in `working_dir` and start the monitor
    /// task. `on_exit` runs exactly once when the child terminates (whether
    /// the termination was requested or not).
    pub async fn spawn<F>(
        app: &AppHandle,
        executable: &Path,
        args: &[String],
        working_dir: &Path,
        on_exit: F,
    ) -> Result<Self>
    where
        F: FnOnce(ExitReason) + Send + 'static,
    {
        let mut cmd = Command::new(executable);
        cmd.args(args)
            .current_dir(working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);

        // Detach from the parent's console so spawning OVMS doesn't pop a
        // black cmd.exe window in front of the user's app.
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", executable.display()))?;
        let pid = child
            .id()
            .ok_or_else(|| anyhow!("spawned ovms reported no pid"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("ovms stdout pipe missing"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("ovms stderr pipe missing"))?;

        let stdout_app = app.clone();
        let stderr_app = app.clone();
        let error_buf: Arc<Mutex<VecDeque<String>>> =
            Arc::new(Mutex::new(VecDeque::with_capacity(ERROR_BUF_CAP)));
        let stdout_buf = Arc::clone(&error_buf);
        let stderr_buf = Arc::clone(&error_buf);
        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let level = classify(&line);
                emit_log(&stdout_app, level, &line);
                if level == "ERROR" {
                    push_error(&stdout_buf, &line).await;
                }
            }
        });
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // OVMS writes most diagnostic output to stderr; classify on
                // content rather than on the stream it arrived from.
                let level = classify(&line);
                emit_log(&stderr_app, level, &line);
                if level == "ERROR" {
                    push_error(&stderr_buf, &line).await;
                }
            }
        });
        let log_task = tokio::spawn(async move {
            let _ = stdout_task.await;
            let _ = stderr_task.await;
        });

        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let monitor = tokio::spawn(async move {
            // Either we get told to kill, or the child exits on its own.
            // Both branches end with `child.wait()` having returned.
            let reason = tokio::select! {
                _ = kill_rx => {
                    if let Err(e) = child.start_kill() {
                        tracing::warn!("ovms start_kill failed: {e}");
                    }
                    let _ = child.wait().await;
                    ExitReason::Expected
                }
                status = child.wait() => {
                    let code = status.ok().and_then(|s| s.code());
                    ExitReason::Crashed { code }
                }
            };
            on_exit(reason);
        });

        Ok(Self {
            pid,
            kill_tx: Some(kill_tx),
            monitor: Some(monitor),
            log_task: Some(log_task),
            error_buf,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Summarise the most recent OVMS error output as a one-paragraph
    /// excerpt the controller can stash in `info.last_error`. Returns
    /// `None` when no error lines have been captured (either OVMS is
    /// healthy or the failure happened pre-stdout, e.g. a missing exe).
    pub async fn error_excerpt(&self) -> Option<String> {
        let lines: Vec<String> = self.error_buf.lock().await.iter().cloned().collect();
        summarize_ovms_errors(&lines)
    }

    /// Kill the child and await its exit. Subsequent log + monitor tasks are
    /// joined too so no stray events arrive after this returns. The
    /// caller-supplied `on_exit` fires from the monitor task right before
    /// this `await` completes.
    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(tx) = self.kill_tx.take() {
            let _ = tx.send(());
        }
        if let Some(m) = self.monitor.take() {
            let _ = m.await;
        }
        if let Some(l) = self.log_task.take() {
            let _ = l.await;
        }
        Ok(())
    }
}

impl Drop for OvmsProcess {
    fn drop(&mut self) {
        // If shutdown wasn't called, signal the monitor so the child gets
        // killed promptly instead of waiting for `kill_on_drop` semantics
        // to bubble through after the monitor handle is leaked.
        if let Some(tx) = self.kill_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn emit_log(app: &AppHandle, level: &'static str, line: &str) {
    let _ = app.emit(
        events::OVMS_LOG,
        OvmsLogLine {
            ts: Utc::now().to_rfc3339(),
            level: level.to_string(),
            line: line.to_string(),
        },
    );
}

/// Append `line` to the bounded error ring, dropping the oldest entry
/// when capacity is exceeded.
async fn push_error(buf: &Arc<Mutex<VecDeque<String>>>, line: &str) {
    let mut g = buf.lock().await;
    if g.len() == ERROR_BUF_CAP {
        g.pop_front();
    }
    g.push_back(line.to_string());
}

/// Turn a window of OVMS `ERROR`-classified lines into a short,
/// user-actionable summary. Detects a few well-known failure modes and
/// attaches a `— hint:` clause pointing at the right zero workaround;
/// falls back to the joined raw lines (truncated) when no pattern matches.
///
/// Public-via-mod so unit tests can exercise the pattern table without
/// spinning up an OVMS process.
pub(crate) fn summarize_ovms_errors(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }

    // Join the buffer so single-error multi-line tracebacks can be
    // searched as one blob.
    let blob = lines.join("\n");
    let mut hint: Option<&'static str> = None;
    let mut headline: Option<String> = None;

    // ─ Known patterns, ordered most-specific first ────────────────
    if blob.contains("SDPAToPagedAttention")
        || blob.contains("No ScaledDotProductAttention operation observed")
    {
        headline = Some(
            "OVMS could not convert this model to PagedAttention: the OpenVINO IR has no SDPA op to fold."
                .into(),
        );
        hint = Some(
            "this export of the model is incompatible with continuous batching — open Model Params and set `pipeline_type` to `VLM` (or `LM` for text-only models) to use the non-CB path",
        );
    } else if blob.contains("Mediapipe graph definition with requested name is not found") {
        headline = Some("OVMS reports the requested model graph isn't loaded.".into());
        hint = Some(
            "OVMS only serves one model at a time; if you just switched models in the chat header, give it a moment to swap, then retry",
        );
    } else if blob.contains("bind") && blob.contains("address already in use") {
        headline = Some("OVMS could not bind its REST port.".into());
        hint = Some(
            "another process is using the REST port — change `ovms.rest_port` in Settings or stop the conflicting process",
        );
    } else if blob.contains("Cannot load library")
        || blob.contains("DLL load failed")
        || blob.contains("cannot open shared object file")
    {
        headline = Some("OVMS failed to load a required shared library.".into());
        hint = Some(
            "reinstall OVMS from the Server page; if that doesn't help, your platform may be missing a Visual C++ / glibc dependency listed in the OVMS release notes",
        );
    }

    // Pull the first line that *looks like* an OVMS-formatted error
    // message so the user sees the actual upstream text even when no
    // pattern matched; the standard format is
    // `[ts][pid][component][error][file:line] message`.
    let first_error_line = lines
        .iter()
        .find(|l| l.contains("[error]") || l.starts_with("[E:"))
        .or_else(|| lines.first())
        .cloned()
        .unwrap_or_default();
    let first_error_line = truncate(&first_error_line, 320);

    let mut out = String::new();
    if let Some(h) = headline {
        out.push_str(&h);
        out.push(' ');
    }
    if !first_error_line.is_empty() {
        out.push_str("OVMS said: ");
        out.push_str(&first_error_line);
    }
    if let Some(h) = hint {
        out.push_str(" — hint: ");
        out.push_str(h);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Cheap classifier that pattern-matches the prefix OVMS prints on each log
/// line (`[I:`, `[E:`, etc.) and falls back to keyword sniffing.
fn classify(line: &str) -> &'static str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("[E:")
        || trimmed.starts_with("[F:")
        || trimmed.contains(" ERROR ")
        || trimmed.contains(" error ")
    {
        "ERROR"
    } else if trimmed.starts_with("[W:")
        || trimmed.contains(" WARN ")
        || trimmed.contains(" warning ")
    {
        "WARN"
    } else {
        "INFO"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_returns_none_for_empty_input() {
        assert!(summarize_ovms_errors(&[]).is_none());
    }

    #[test]
    fn summarize_detects_sdpa_to_paged_attention_and_points_at_pipeline_type() {
        let lines = vec![
            "[2026-06-13 10:58:19.449][16500][serving][error][servable_initializer.cpp:237] Error during llm node initialization for models_path: ./ exception: Check 'ov::op::util::has_op_with_type<ov::op::v13::ScaledDotProductAttention>(model)' failed at src\\core\\src\\pass\\sdpa_to_paged_attention.cpp:80:".to_string(),
            "No ScaledDotProductAttention operation observed in the graph, cannot perform the SDPAToPagedAttention transformation.".to_string(),
        ];
        let out = summarize_ovms_errors(&lines).expect("should summarise");
        assert!(out.contains("PagedAttention"));
        assert!(out.contains("pipeline_type"));
        assert!(out.contains("VLM"));
        // Includes the actual upstream line so the user can grep logs.
        assert!(out.contains("OVMS said:"));
    }

    #[test]
    fn summarize_detects_port_in_use() {
        let lines = vec!["[E:1234] failed to bind socket: address already in use".to_string()];
        let out = summarize_ovms_errors(&lines).expect("should summarise");
        assert!(out.to_lowercase().contains("port"));
        assert!(out.contains("rest_port"));
    }

    #[test]
    fn summarize_falls_back_to_raw_line_when_no_pattern_matches() {
        let lines = vec!["[E:9999] some novel failure mode".to_string()];
        let out = summarize_ovms_errors(&lines).expect("should summarise");
        assert!(out.contains("OVMS said:"));
        assert!(out.contains("novel failure mode"));
        // No hint clause when we didn't recognise the pattern.
        assert!(!out.contains("hint:"));
    }
}
