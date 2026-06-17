//! Shell-execution built-in tool.
//!
//! Exposes a single [`ShellExec`] tool that runs a command on the host
//! through the user's default shell (`cmd /C` on Windows, `sh -c`
//! elsewhere) *or* — when the model passes an explicit `args` array —
//! directly without shell interpretation. The latter form avoids quoting
//! pitfalls when the model already knows the binary and its arguments.
//!
//! Output handling:
//!
//! - `stdout` and `stderr` are captured separately and reported in full
//!   up to [`MAX_OUTPUT_BYTES`] each; longer streams are truncated with a
//!   trailing `… [truncated]` marker so the model knows it didn't see
//!   everything.
//! - The exit status is always included so the model can decide whether
//!   to retry / fix arguments.
//! - A `timeout_ms` (default [`DEFAULT_TIMEOUT_MS`]) bounds runtime; on
//!   timeout we kill the child and return an error result rather than
//!   hanging the chat turn.
//!
//! The tool is marked **destructive** so the global confirm gate trips
//! on every call — running arbitrary shell commands is the single most
//! dangerous capability the model has.

use crate::mcp::{Tool, ToolResult, ToolSchema};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Cap on captured stdout / stderr bytes per stream. ~64 KiB is plenty
/// for typical build / git / curl output while keeping the resulting
/// `tool` message well under typical model context budgets.
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Default timeout if the caller omits `timeout_ms`. Long enough for a
/// short build or test command, short enough that a runaway loop can't
/// gate the chat indefinitely.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Hard upper bound on `timeout_ms` so the model can't accidentally
/// (or otherwise) ask for a 24-hour-long run.
const MAX_TIMEOUT_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Default)]
pub struct ShellExec;

#[derive(Debug, Deserialize)]
struct ExecArgs {
    /// The command to run. When `args` is absent, this string is passed
    /// to the host shell verbatim (so `|`, `>`, `&&`, ... all work).
    /// When `args` is present, this is the binary name only.
    command: String,
    /// Optional argv. If supplied, `command` is invoked directly (no
    /// shell), avoiding all quoting/escaping ambiguity.
    #[serde(default)]
    args: Option<Vec<String>>,
    /// Working directory for the child. Defaults to the zero process
    /// cwd. `~` is *not* expanded here — pass an absolute path.
    #[serde(default)]
    cwd: Option<String>,
    /// Maximum runtime in milliseconds before the child is killed.
    /// Clamped to [`MAX_TIMEOUT_MS`].
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[async_trait]
impl Tool for ShellExec {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "shell.exec".into(),
            description: "Run a command on the host shell and return its \
                 stdout, stderr, and exit code. Without `args`, the \
                 command runs through the user's default shell \
                 (`cmd /C` on Windows, `sh -c` elsewhere) so pipes and \
                 redirects work. With `args`, the command is invoked \
                 directly without shell interpretation. Output is \
                 truncated to ~64 KiB per stream. This tool is \
                 destructive — the user is prompted before each call \
                 unless they've disabled the confirm gate in Settings."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command line (or program name when `args` is set)."
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional argv. When set, bypasses the shell."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory (absolute path recommended)."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Kill the child after this many milliseconds. Default 30000, max 300000."
                    }
                }
            }),
            destructive: true,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: ExecArgs = serde_json::from_value(args).context("shell.exec: parse arguments")?;
        if a.command.trim().is_empty() {
            return Ok(ToolResult {
                content: "shell.exec: `command` is empty".into(),
                is_error: true,
            });
        }

        let mut cmd = build_command(&a);
        if let Some(cwd) = a.cwd.as_deref() {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("shell.exec: spawn `{}`", a.command))?;

        // Pull stdout/stderr concurrently with the wait so a child that
        // fills its pipe buffers can't deadlock.
        let mut stdout = child.stdout.take().expect("stdout piped above");
        let mut stderr = child.stderr.take().expect("stderr piped above");
        let stdout_fut = read_capped(&mut stdout);
        let stderr_fut = read_capped(&mut stderr);

        let timeout = Duration::from_millis(
            a.timeout_ms
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .clamp(1, MAX_TIMEOUT_MS),
        );

        let run = async {
            let (out_res, err_res, status_res) = tokio::join!(stdout_fut, stderr_fut, child.wait());
            let stdout_buf = out_res.unwrap_or_else(|e| Capped {
                bytes: format!("[stdout read error: {e}]").into_bytes(),
                truncated: false,
            });
            let stderr_buf = err_res.unwrap_or_else(|e| Capped {
                bytes: format!("[stderr read error: {e}]").into_bytes(),
                truncated: false,
            });
            (stdout_buf, stderr_buf, status_res)
        };

        let (stdout_buf, stderr_buf, status_res) = match tokio::time::timeout(timeout, run).await {
            Ok(v) => v,
            Err(_) => {
                // Best-effort kill — `Child` is dropped when this returns
                // either way (which also kills on Unix via kill_on_drop
                // — but we don't set that flag, so kill explicitly).
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Ok(ToolResult {
                    content: format!(
                        "shell.exec: timed out after {} ms — child killed",
                        timeout.as_millis()
                    ),
                    is_error: true,
                });
            }
        };

        let status = status_res.context("shell.exec: wait on child")?;
        let exit = status.code();
        let signal_note = if exit.is_none() {
            " (killed by signal)"
        } else {
            ""
        };

        let mut body = String::new();
        body.push_str(&format!(
            "exit {}{signal_note}\n",
            exit.map(|c| c.to_string()).unwrap_or_else(|| "-".into())
        ));
        body.push_str("--- stdout ---\n");
        body.push_str(&stdout_buf.into_string());
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str("--- stderr ---\n");
        body.push_str(&stderr_buf.into_string());

        Ok(ToolResult {
            content: body,
            is_error: !status.success(),
        })
    }
}

/// Build a [`Command`] for the requested form (shelled vs. direct).
fn build_command(a: &ExecArgs) -> Command {
    if let Some(argv) = a.args.as_ref() {
        let mut c = Command::new(&a.command);
        c.args(argv);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(&a.command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&a.command);
        c
    }
}

struct Capped {
    bytes: Vec<u8>,
    truncated: bool,
}

impl Capped {
    fn into_string(self) -> String {
        let mut s = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            s.push_str("\n… [truncated]");
        }
        s
    }
}

async fn read_capped<R: AsyncReadExt + Unpin>(reader: &mut R) -> std::io::Result<Capped> {
    let mut buf = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        if buf.len() < MAX_OUTPUT_BYTES {
            let take = (MAX_OUTPUT_BYTES - buf.len()).min(n);
            buf.extend_from_slice(&chunk[..take]);
            if take < n {
                truncated = true;
                // keep draining so the child doesn't block on a full pipe
            }
        } else {
            truncated = true;
        }
    }
    Ok(Capped {
        bytes: buf,
        truncated,
    })
}

pub fn all() -> Vec<Box<dyn Tool>> {
    vec![Box::new(ShellExec)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_advertises_destructive_and_required_command() {
        let s = ShellExec.schema();
        assert_eq!(s.name, "shell.exec");
        assert!(s.destructive);
        assert_eq!(s.input_schema["required"], json!(["command"]));
    }

    #[tokio::test]
    async fn empty_command_is_an_error_result_not_a_panic() {
        let r = ShellExec.call(json!({ "command": "  " })).await.unwrap();
        assert!(r.is_error);
        assert!(r.content.contains("empty"));
    }

    #[tokio::test]
    async fn runs_a_trivial_command_and_captures_stdout() {
        // Pick a command that prints predictably on both shells.
        let cmd = if cfg!(windows) {
            json!({ "command": "echo hello" })
        } else {
            json!({ "command": "printf hello" })
        };
        let r = ShellExec.call(cmd).await.unwrap();
        assert!(!r.is_error, "expected success, got: {}", r.content);
        assert!(r.content.contains("hello"), "got: {}", r.content);
        assert!(r.content.contains("exit 0"), "got: {}", r.content);
    }

    #[tokio::test]
    async fn timeout_kills_long_running_child() {
        let cmd = if cfg!(windows) {
            // `ping` with 6 packets to localhost runs ~5s on Windows.
            json!({ "command": "ping -n 6 127.0.0.1", "timeout_ms": 200 })
        } else {
            json!({ "command": "sleep 5", "timeout_ms": 200 })
        };
        let r = ShellExec.call(cmd).await.unwrap();
        assert!(r.is_error);
        assert!(r.content.contains("timed out"));
    }

    #[tokio::test]
    async fn explicit_args_run_the_program_directly() {
        // With `args`, we invoke the program without going through our
        // shell wrapper. On Windows the demo target is still `cmd /C`
        // because we can't assume any other binary is on PATH; on Unix
        // we use `printf` which does no further parsing of its args.
        let cmd = if cfg!(windows) {
            json!({ "command": "cmd", "args": ["/C", "echo", "hello-args"] })
        } else {
            json!({ "command": "printf", "args": ["%s", "a|b"] })
        };
        let r = ShellExec.call(cmd).await.unwrap();
        assert!(!r.is_error, "got: {}", r.content);
        if cfg!(windows) {
            assert!(r.content.contains("hello-args"), "got: {}", r.content);
        } else {
            // The `|` survives because we never invoked a shell.
            assert!(r.content.contains("a|b"), "got: {}", r.content);
        }
    }
}
