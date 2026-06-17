//! Built-in `memory` tool — the agent's hands-on access to persistent
//! memory.
//!
//! Modelled directly on Nous Research's Hermes Agent `memory` tool. The
//! agent calls this whenever it learns something durable about its
//! environment, the user, or a workflow — and curates the same store
//! over time by replacing or removing stale entries.
//!
//! Three actions, two targets:
//!
//! | Action    | Required arguments                                  |
//! | --------- | --------------------------------------------------- |
//! | `add`     | `target`, `content`                                 |
//! | `replace` | `target`, `old_text`, `content`                     |
//! | `remove`  | `target`, `old_text`                                |
//!
//! There is **no `read` action**: both stores are injected into the
//! system prompt as a frozen snapshot at session start, so the agent
//! already sees them. (Mid-session writes update the file on disk
//! immediately but won't appear in-prompt until the next conversation
//! turn — the tool result echoes the updated state so the model can
//! reason about its current memory without re-reading the prompt.)
//!
//! Failure shape matches the rest of the built-in registry: capacity /
//! ambiguity / no-match errors return as `is_error = true` `ToolResult`s
//! with the exact same wording Hermes uses, so a model trained on the
//! Hermes corpus knows how to recover (consolidate, tighten the
//! substring, etc.) without our needing a custom prompt.

use crate::mcp::{Tool, ToolResult, ToolSchema};
use crate::memory::{self, MemoryError, MemorySnapshot, MemoryTarget};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Default)]
pub struct MemoryInvoke;

#[derive(Debug, Deserialize)]
struct Args {
    action: String,
    target: MemoryTarget,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    old_text: Option<String>,
}

#[async_trait]
impl Tool for MemoryInvoke {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "memory".into(),
            description: "Curate the agent's persistent memory. Two stores are available: \
                 `memory` (your personal notes — environment facts, conventions, \
                 lessons learned) and `user` (the user's preferences, identity, \
                 communication style). Both files are injected into the system \
                 prompt as a frozen snapshot at the start of every turn, so call \
                 this tool whenever you learn something durable. There is no \
                 `read` action — you already see the current entries in the \
                 system prompt. Use `add` for new facts, `replace` to update an \
                 existing entry (matching on a unique substring of the old \
                 entry), and `remove` to drop a stale or wrong entry. If the \
                 store is full, the call returns an error showing usage — \
                 consolidate overlapping entries with `replace` or drop low-value \
                 entries with `remove`, then retry your `add` in the same turn."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["action", "target"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add", "replace", "remove"],
                        "description": "What to do: add a new entry, replace an existing one, or remove one."
                    },
                    "target": {
                        "type": "string",
                        "enum": ["memory", "user"],
                        "description": "Which store to operate on. `memory` = your personal notes; `user` = the user profile."
                    },
                    "content": {
                        "type": "string",
                        "description": "Required for `add` and `replace`. The new entry text. Aim for compact, information-dense entries (one durable fact per entry)."
                    },
                    "old_text": {
                        "type": "string",
                        "description": "Required for `replace` and `remove`. A unique substring of the entry to target. Does not need to match the entry exactly; just enough to identify exactly one entry."
                    }
                }
            }),
            // Memory edits are local file writes and the per-store cap
            // bounds the blast radius — no destructive-confirm gate.
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let parsed: Args = match serde_json::from_value(args).context("memory: parse arguments") {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult {
                    content: format!("[memory: invalid arguments: {e:#}]"),
                    is_error: true,
                });
            }
        };

        let target = parsed.target;
        match parsed.action.as_str() {
            "add" => {
                let Some(content) = parsed.content.as_deref() else {
                    return Ok(missing("content", "add"));
                };
                deliver(memory::add(target, content).await, target, "add")
            }
            "replace" => {
                let Some(old_text) = parsed.old_text.as_deref() else {
                    return Ok(missing("old_text", "replace"));
                };
                let Some(content) = parsed.content.as_deref() else {
                    return Ok(missing("content", "replace"));
                };
                deliver(
                    memory::replace(target, old_text, content).await,
                    target,
                    "replace",
                )
            }
            "remove" => {
                let Some(old_text) = parsed.old_text.as_deref() else {
                    return Ok(missing("old_text", "remove"));
                };
                deliver(memory::remove(target, old_text).await, target, "remove")
            }
            other => Ok(ToolResult {
                content: format!(
                    "[memory: unknown action `{other}` (allowed: add, replace, remove)]"
                ),
                is_error: true,
            }),
        }
    }
}

pub fn all() -> Vec<Box<dyn Tool>> {
    vec![Box::new(MemoryInvoke)]
}

// ─── helpers ──────────────────────────────────────────────────────────

fn missing(field: &str, action: &str) -> ToolResult {
    ToolResult {
        content: format!("[memory: `{field}` is required for action `{action}`]"),
        is_error: true,
    }
}

/// Convert a memory operation result into a `ToolResult`. On success we
/// echo the post-write snapshot back to the model so it can reason
/// about the live state without waiting for the next turn's frozen
/// snapshot. On error we mirror Hermes' "Consolidate now: …" hint so a
/// Hermes-trained model recovers naturally.
fn deliver(
    res: Result<MemorySnapshot, MemoryError>,
    target: MemoryTarget,
    action: &str,
) -> Result<ToolResult> {
    match res {
        Ok(snap) => Ok(ToolResult {
            content: format_success(&snap, action),
            is_error: false,
        }),
        Err(MemoryError::OverCapacity {
            used, added, limit, ..
        }) => {
            // The wording here matches Hermes' on-the-wire error
            // verbatim — Hermes-trained models look for the phrase
            // "Consolidate now" as a recovery cue.
            let msg = format!(
                "[memory: {tgt} memory at {used}/{limit} chars. The {action} would add \
                 {added} chars and exceed the limit. Consolidate now: use \
                 `replace` to merge overlapping entries into shorter ones, or \
                 `remove` stale entries — then retry this {action} in the same \
                 turn. Current entries are visible in the system prompt under \
                 the persistent-memory block.]",
                tgt = target.as_str(),
                used = used,
                limit = limit,
                added = added,
                action = action,
            );
            Ok(ToolResult {
                content: msg,
                is_error: true,
            })
        }
        Err(MemoryError::AmbiguousMatch { count, .. }) => Ok(ToolResult {
            content: format!(
                "[memory: `old_text` matches {count} entries in {tgt} memory. \
                 Supply a more specific substring that identifies exactly one \
                 entry.]",
                tgt = target.as_str(),
            ),
            is_error: true,
        }),
        Err(MemoryError::NoMatch { .. }) => Ok(ToolResult {
            content: format!(
                "[memory: no entry in {tgt} memory contains the given `old_text`. \
                 Check the persistent-memory block in the system prompt for the \
                 current entries.]",
                tgt = target.as_str(),
            ),
            is_error: true,
        }),
        Err(MemoryError::EntryTooLong { len, max }) => Ok(ToolResult {
            content: format!(
                "[memory: entry is {len} chars; the per-entry cap is {max}. Split \
                 it into multiple shorter entries or compress.]"
            ),
            is_error: true,
        }),
        Err(MemoryError::EmptyEntry) => Ok(ToolResult {
            content: "[memory: entry text is empty after trimming]".into(),
            is_error: true,
        }),
        Err(MemoryError::Io(e)) => Ok(ToolResult {
            content: format!("[memory: write failed: {e}]"),
            is_error: true,
        }),
    }
}

fn format_success(snap: &MemorySnapshot, action: &str) -> String {
    let header = format!(
        "[memory: {action} ok — {tgt} now {used}/{limit} chars ({pct}%), {n} entr{noun}]",
        action = action,
        tgt = snap.target,
        used = snap.used,
        limit = snap.limit,
        pct = snap.percent(),
        n = snap.entries.len(),
        noun = if snap.entries.len() == 1 { "y" } else { "ies" },
    );
    if snap.entries.is_empty() {
        return header;
    }
    let mut out = String::with_capacity(header.len() + snap.used + 64);
    out.push_str(&header);
    out.push_str("\n\nCurrent entries:\n");
    for (i, e) in snap.entries.iter().enumerate() {
        // Number entries so the model can grep its own log when it
        // decides which one to consolidate or remove next.
        out.push_str(&format!("{n:>2}. ", n = i + 1));
        // Preserve newlines but indent continuation lines so each
        // entry is visually one row in a chat bubble.
        for (j, line) in e.lines().enumerate() {
            if j > 0 {
                out.push_str("\n    ");
            }
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_exposes_three_actions_and_two_targets() {
        let s = MemoryInvoke.schema();
        assert_eq!(s.name, "memory");
        assert!(!s.destructive);
        let actions = &s.input_schema["properties"]["action"]["enum"];
        assert_eq!(actions, &json!(["add", "replace", "remove"]));
        let targets = &s.input_schema["properties"]["target"]["enum"];
        assert_eq!(targets, &json!(["memory", "user"]));
    }

    #[tokio::test]
    async fn unknown_action_returns_structured_error() {
        let out = MemoryInvoke
            .call(json!({"action": "rm -rf", "target": "memory"}))
            .await
            .expect("schema-valid call");
        assert!(out.is_error);
        assert!(out.content.contains("unknown action"));
    }

    #[tokio::test]
    async fn add_without_content_returns_missing_field_error() {
        let out = MemoryInvoke
            .call(json!({"action": "add", "target": "memory"}))
            .await
            .expect("schema-valid call");
        assert!(out.is_error);
        assert!(out.content.contains("`content` is required"));
    }

    #[tokio::test]
    async fn replace_without_old_text_returns_missing_field_error() {
        let out = MemoryInvoke
            .call(json!({"action": "replace", "target": "user", "content": "x"}))
            .await
            .expect("schema-valid call");
        assert!(out.is_error);
        assert!(out.content.contains("`old_text` is required"));
    }
}
