//! Clipboard built-in tools (`clipboard.read`, `clipboard.write`).
//!
//! Backed by [`arboard`] so we don't have to thread the JS-side
//! clipboard-manager plugin through every tool call. `arboard` opens a
//! short-lived handle per request — on Windows / macOS the new value
//! persists in the OS clipboard after the handle is dropped, which is
//! exactly the behaviour the model expects.
//!
//! On Linux the persistence story is more involved (the clipboard
//! contents live in the X11 / Wayland *application*, not the system),
//! so `clipboard.write` on Linux only guarantees the value lives as
//! long as the zero process. That's acceptable for an agent that's
//! about to feed the result back to the user anyway.
//!
//! Both tools are marked non-destructive: pasting and reading the
//! clipboard are routine actions the user would otherwise do
//! themselves, and clipping every call behind a confirm prompt would
//! make the model unusable for "summarise this then put it in my
//! clipboard" workflows.

use crate::mcp::{Tool, ToolResult, ToolSchema};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

/// Cap on the size of a single `clipboard.write` so the model can't
/// accidentally stash a megabyte of context into the user's clipboard.
const MAX_WRITE_BYTES: usize = 1 * 1024 * 1024;

/// Cap on the size of a `clipboard.read` response. Anything larger is
/// truncated with the usual marker.
const MAX_READ_BYTES: usize = 256 * 1024;

#[derive(Debug, Default)]
pub struct ClipboardRead;

#[async_trait]
impl Tool for ClipboardRead {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "clipboard.read".into(),
            description: "Read the current text contents of the system \
                 clipboard. Returns an empty string if the clipboard is \
                 empty or contains non-text data. Responses larger than \
                 ~256 KiB are truncated."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            destructive: false,
        }
    }

    async fn call(&self, _args: Value) -> Result<ToolResult> {
        // arboard's handle is !Send on some platforms; run it on a
        // blocking thread so the async runtime stays clean.
        let text = tokio::task::spawn_blocking(read_text)
            .await
            .map_err(|e| anyhow!("clipboard.read task panicked: {e}"))??;

        let (out, truncated) = if text.len() > MAX_READ_BYTES {
            (text[..MAX_READ_BYTES].to_string(), true)
        } else {
            (text, false)
        };
        let body = if truncated {
            format!("{out}\n… [truncated]")
        } else {
            out
        };
        Ok(ToolResult {
            content: body,
            is_error: false,
        })
    }
}

#[derive(Debug, Default)]
pub struct ClipboardWrite;

#[derive(Debug, Deserialize)]
struct WriteArgs {
    text: String,
}

#[async_trait]
impl Tool for ClipboardWrite {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "clipboard.write".into(),
            description: "Replace the system clipboard with the given \
                 text. On Linux the value persists only while zero is \
                 running. Limited to ~1 MiB per call."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Replacement clipboard contents."
                    }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: WriteArgs =
            serde_json::from_value(args).context("clipboard.write: parse arguments")?;
        if a.text.len() > MAX_WRITE_BYTES {
            return Ok(ToolResult {
                content: format!(
                    "clipboard.write: payload is {} bytes, max is {}",
                    a.text.len(),
                    MAX_WRITE_BYTES
                ),
                is_error: true,
            });
        }
        let n = a.text.len();
        tokio::task::spawn_blocking(move || write_text(&a.text))
            .await
            .map_err(|e| anyhow!("clipboard.write task panicked: {e}"))??;
        Ok(ToolResult {
            content: format!("copied {n} bytes to clipboard"),
            is_error: false,
        })
    }
}

fn read_text() -> Result<String> {
    let mut cb = arboard::Clipboard::new().context("clipboard: open handle")?;
    match cb.get_text() {
        Ok(s) => Ok(s),
        // `ContentNotAvailable` is the normal "clipboard is empty or
        // holds non-text data" signal — surface it as an empty string
        // rather than an error so the model can branch on `if text != ""`.
        Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
        Err(e) => Err(anyhow!("clipboard: read text: {e}")),
    }
}

fn write_text(s: &str) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("clipboard: open handle")?;
    cb.set_text(s.to_string())
        .map_err(|e| anyhow!("clipboard: write text: {e}"))?;
    Ok(())
}

pub fn all() -> Vec<Box<dyn Tool>> {
    vec![Box::new(ClipboardRead), Box::new(ClipboardWrite)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schemas_advertise_expected_names_and_non_destructive() {
        assert_eq!(ClipboardRead.schema().name, "clipboard.read");
        assert!(!ClipboardRead.schema().destructive);
        assert_eq!(ClipboardWrite.schema().name, "clipboard.write");
        assert!(!ClipboardWrite.schema().destructive);
        assert_eq!(
            ClipboardWrite.schema().input_schema["required"],
            json!(["text"])
        );
    }

    #[tokio::test]
    async fn write_rejects_oversize_payload() {
        let big = "x".repeat(MAX_WRITE_BYTES + 1);
        let r = ClipboardWrite.call(json!({ "text": big })).await.unwrap();
        assert!(r.is_error);
        assert!(r.content.contains("max is"));
    }

    // We do *not* exercise the real OS clipboard in unit tests because
    // the CI environments this project targets are headless on Linux
    // and arboard's Linux backend requires X11 / Wayland. Manual smoke
    // testing on Windows/macOS covers the round-trip.
}
