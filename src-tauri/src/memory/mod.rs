//! Persistent agent memory.
//!
//! Modelled on Nous Research's Hermes Agent memory subsystem: two small,
//! character-bounded markdown files curated by the agent itself, injected
//! into the system prompt as a *frozen snapshot* at the start of every
//! conversation turn.
//!
//! Two stores, two purposes:
//!
//! | File                          | Target  | Holds                                        | Cap (chars) |
//! | ----------------------------- | ------- | -------------------------------------------- | ----------- |
//! | `~/.zero/memories/MEMORY.md`  | `memory`| environment facts, conventions, lessons      | 2200        |
//! | `~/.zero/memories/USER.md`    | `user`  | the user's preferences, style, identity      | 1375        |
//!
//! Each file is plain UTF-8: a list of free-form entries separated by the
//! `§` (section sign) delimiter on its own line. We deliberately do **not**
//! use SQLite — Hermes' insight is that the model itself is the curator,
//! and round-tripping rows through a query language gets in the way of
//! the model treating memory as "notes I edit on disk".
//!
//! The agent edits memory via the built-in [`crate::mcp::tools::memory`]
//! tool (`add` / `replace` / `remove`). The UI (Memory page) edits the
//! same files through Tauri commands. Both write the file atomically and
//! re-validate the cap on every change so the user can't blow past it
//! from the UI either.
//!
//! Capacity rule (matches Hermes):
//!
//! * Writes that would overflow the cap return [`MemoryError::OverCapacity`]
//!   instead of silently truncating. The agent is expected to consolidate
//!   or remove entries and retry in the same turn.
//! * Exact duplicates (substring-trim equality) are a successful no-op so
//!   the agent doesn't need to read-before-add.
//!
//! Substring matching for `replace`/`remove`:
//! * `old_text` only needs to be a **unique substring** of the target
//!   entry. Ambiguous matches surface as [`MemoryError::AmbiguousMatch`]
//!   so the caller can re-issue with a tighter substring.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use tokio::fs;

use crate::paths;

/// Delimiter that separates entries inside `MEMORY.md` / `USER.md`. Always
/// emitted on its own line so a casual `cat` of the file stays readable
/// and the model recognises the boundary in its frozen snapshot.
pub const ENTRY_DELIMITER: &str = "§";

/// Default char cap for `MEMORY.md`. Roughly ~800 tokens.
pub const DEFAULT_MEMORY_LIMIT: usize = 2200;

/// Default char cap for `USER.md`. Roughly ~500 tokens.
pub const DEFAULT_USER_LIMIT: usize = 1375;

/// Maximum size a single entry is allowed to occupy. Keeps a runaway
/// `add` call from filling the whole file with one mega-entry, which
/// would defeat the consolidate-or-remove flow at capacity.
pub const MAX_ENTRY_CHARS: usize = 1200;

/// Which of the two stores to operate on. The tool surface uses the
/// `lowercase` form on the wire so the model's tool-call JSON reads
/// naturally (`{"target": "memory", ...}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryTarget {
    /// Agent's personal notes — environment, conventions, lessons.
    Memory,
    /// User profile — preferences, style, identity.
    User,
}

impl MemoryTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::User => "user",
        }
    }

    /// Header label rendered into the system prompt snapshot.
    pub fn header_label(self) -> &'static str {
        match self {
            Self::Memory => "MEMORY (your personal notes)",
            Self::User => "USER PROFILE",
        }
    }

    pub fn default_limit(self) -> usize {
        match self {
            Self::Memory => DEFAULT_MEMORY_LIMIT,
            Self::User => DEFAULT_USER_LIMIT,
        }
    }
}

/// Errors surfaced by the memory API. We use a dedicated enum (rather
/// than just `anyhow::Error`) because the chat-tool layer needs to turn
/// `OverCapacity` and `AmbiguousMatch` into structured `is_error = true`
/// tool results the model can recover from.
#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Write rejected: the resulting file would exceed `limit`. Carries
    /// the current usage so the caller can render the same hint Hermes
    /// shows the model ("Consolidate now: use 'replace' …").
    #[error(
        "{target} memory at {used}/{limit} chars; adding {added} chars would exceed the limit"
    )]
    OverCapacity {
        target: &'static str,
        used: usize,
        added: usize,
        limit: usize,
    },

    /// A single entry was larger than [`MAX_ENTRY_CHARS`]. Cheap guard
    /// against the model trying to dump an entire log into memory.
    #[error("entry is {len} chars; the per-entry cap is {max}")]
    EntryTooLong { len: usize, max: usize },

    /// `old_text` matched no entry in the target store.
    #[error("no entry in {target} memory matches the given old_text")]
    NoMatch { target: &'static str },

    /// `old_text` matched more than one entry. Ask for a tighter substring.
    #[error(
        "old_text matches {count} entries in {target} memory; supply a more specific substring"
    )]
    AmbiguousMatch { target: &'static str, count: usize },

    /// Entry is empty after trimming. Saves a useless slot.
    #[error("entry is empty")]
    EmptyEntry,
}

pub type MemoryResult<T> = std::result::Result<T, MemoryError>;

/// A single render-ready snapshot of one memory store. Cheap to clone —
/// the chat runner pulls one of these per turn for system prompt
/// injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    pub target: String,
    pub entries: Vec<String>,
    pub used: usize,
    pub limit: usize,
    pub path: String,
}

impl MemorySnapshot {
    pub fn percent(&self) -> u8 {
        if self.limit == 0 {
            return 0;
        }
        // saturating_mul wouldn't help here (usize), but the cap is tiny
        // — overflow isn't a real concern.
        ((self.used * 100) / self.limit).min(100) as u8
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Both stores in one shot. Used by the IPC layer and the chat runner
/// (which reads both at the top of every turn to build the frozen
/// snapshot block).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryState {
    pub memory: MemorySnapshot,
    pub user: MemorySnapshot,
}

// ─── Public API ────────────────────────────────────────────────────────

/// Read the current state of both stores. Missing files are returned as
/// empty snapshots so a fresh install just shows zero entries instead of
/// failing the chat turn.
pub async fn load_state() -> Result<MemoryState> {
    let memory = load_snapshot(MemoryTarget::Memory).await?;
    let user = load_snapshot(MemoryTarget::User).await?;
    Ok(MemoryState { memory, user })
}

/// Read one store. Public so the Memory page can refresh just the half
/// the user is editing without re-reading the other file.
pub async fn load_snapshot(target: MemoryTarget) -> Result<MemorySnapshot> {
    load_snapshot_inner(target).await.map_err(Into::into)
}

/// Same as [`load_snapshot`] but returns the crate's typed error. Used
/// by the write APIs so a `?` operator inside `add` / `replace` /
/// `remove` lifts cleanly into [`MemoryResult`] without an intermediate
/// `map_err`.
async fn load_snapshot_inner(target: MemoryTarget) -> MemoryResult<MemorySnapshot> {
    let path = file_path(target).map_err(io_other)?;
    let limit = target.default_limit();
    let text = match fs::read_to_string(&path).await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(MemoryError::Io(e)),
    };
    let entries = parse_entries(&text);
    let used = serialize_entries(&entries).chars().count();
    Ok(MemorySnapshot {
        target: target.as_str().into(),
        entries,
        used,
        limit,
        path: path.to_string_lossy().into_owned(),
    })
}

/// Append `content` as a new entry. Returns the post-write snapshot.
///
/// Duplicate detection: if a trimmed-equal entry already exists, this is
/// a no-op (matches Hermes' "no duplicate added" behaviour) so the model
/// doesn't have to read-before-add to be safe.
pub async fn add(target: MemoryTarget, content: &str) -> MemoryResult<MemorySnapshot> {
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        return Err(MemoryError::EmptyEntry);
    }
    let entry_len = trimmed.chars().count();
    if entry_len > MAX_ENTRY_CHARS {
        return Err(MemoryError::EntryTooLong {
            len: entry_len,
            max: MAX_ENTRY_CHARS,
        });
    }

    let mut snap = load_snapshot_inner(target).await?;
    if snap.entries.iter().any(|e| e.trim() == trimmed) {
        // Duplicate — silently succeed so the model doesn't error out.
        return Ok(snap);
    }

    let projected = projected_usage(&snap.entries, Some(&trimmed));
    if projected > snap.limit {
        return Err(MemoryError::OverCapacity {
            target: target.as_str(),
            used: snap.used,
            added: projected - snap.used,
            limit: snap.limit,
        });
    }

    snap.entries.push(trimmed);
    write_entries(target, &snap.entries).await?;
    snap.used = projected;
    Ok(snap)
}

/// Replace the entry uniquely identified by `old_text` (substring match)
/// with `new_content`.
pub async fn replace(
    target: MemoryTarget,
    old_text: &str,
    new_content: &str,
) -> MemoryResult<MemorySnapshot> {
    let needle = old_text.trim();
    if needle.is_empty() {
        return Err(MemoryError::EmptyEntry);
    }
    let new_trim = new_content.trim().to_string();
    if new_trim.is_empty() {
        return Err(MemoryError::EmptyEntry);
    }
    let entry_len = new_trim.chars().count();
    if entry_len > MAX_ENTRY_CHARS {
        return Err(MemoryError::EntryTooLong {
            len: entry_len,
            max: MAX_ENTRY_CHARS,
        });
    }

    let mut snap = load_snapshot_inner(target).await?;
    let idx = find_unique_match(&snap.entries, needle, target)?;

    // Build the projected entry list and check the cap *before* writing,
    // so a too-long replacement surfaces the same OverCapacity hint the
    // agent's already taught to react to.
    let mut projected_entries = snap.entries.clone();
    projected_entries[idx] = new_trim;
    let projected = projected_usage(&projected_entries, None);
    if projected > snap.limit {
        return Err(MemoryError::OverCapacity {
            target: target.as_str(),
            used: snap.used,
            added: projected.saturating_sub(snap.used),
            limit: snap.limit,
        });
    }

    snap.entries = projected_entries;
    write_entries(target, &snap.entries).await?;
    snap.used = projected;
    Ok(snap)
}

/// Remove the entry uniquely identified by `old_text` (substring match).
pub async fn remove(target: MemoryTarget, old_text: &str) -> MemoryResult<MemorySnapshot> {
    let needle = old_text.trim();
    if needle.is_empty() {
        return Err(MemoryError::EmptyEntry);
    }
    let mut snap = load_snapshot_inner(target).await?;
    let idx = find_unique_match(&snap.entries, needle, target)?;
    snap.entries.remove(idx);
    write_entries(target, &snap.entries).await?;
    snap.used = serialize_entries(&snap.entries).chars().count();
    Ok(snap)
}

/// Overwrite the entire store with `raw`. Used by the Memory page's "raw
/// edit" mode. Splits on the delimiter and re-validates against the cap
/// so the UI gets the same guarantees the tool surface does.
pub async fn set_raw(target: MemoryTarget, raw: &str) -> MemoryResult<MemorySnapshot> {
    let entries = parse_entries(raw);
    let used = serialize_entries(&entries).chars().count();
    let limit = target.default_limit();
    if used > limit {
        return Err(MemoryError::OverCapacity {
            target: target.as_str(),
            used,
            added: 0,
            limit,
        });
    }
    write_entries(target, &entries).await?;
    let path = file_path(target).map_err(io_other)?;
    Ok(MemorySnapshot {
        target: target.as_str().into(),
        entries,
        used,
        limit,
        path: path.to_string_lossy().into_owned(),
    })
}

/// Render both stores as a single system-prompt block, ready to splice
/// into the chat runner's `build_system_prompt`. Returns an empty string
/// when both stores are empty so we don't leak a useless header into
/// the prompt for fresh installs.
///
/// Mirrors Hermes' display format:
///
/// ```text
/// ══════════════════════════════════════════════
/// MEMORY (your personal notes) [67% — 1474/2200 chars]
/// ══════════════════════════════════════════════
/// entry one
/// §
/// entry two
/// ```
pub fn render_prompt_block(state: &MemoryState) -> String {
    let mut out = String::new();
    if !state.memory.is_empty() {
        out.push_str(&render_one(&state.memory));
    }
    if !state.user.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&render_one(&state.user));
    }
    if out.is_empty() {
        return String::new();
    }
    // Wrap the block in a leading section header so the model sees it
    // as a distinct, persistent part of the system prompt rather than
    // free-floating text.
    format!("\n\n# Persistent memory\n{out}")
}

// ─── Internals ─────────────────────────────────────────────────────────

fn file_path(target: MemoryTarget) -> Result<PathBuf> {
    let dir = memories_dir()?;
    Ok(dir.join(match target {
        MemoryTarget::Memory => "MEMORY.md",
        MemoryTarget::User => "USER.md",
    }))
}

/// `~/.zero/memories/` — created lazily on first read/write.
fn memories_dir() -> Result<PathBuf> {
    let p = paths::root()?.join("memories");
    std::fs::create_dir_all(&p).with_context(|| format!("creating {p:?}"))?;
    Ok(p)
}

/// Parse a file body into individual entries. The delimiter must appear
/// on its own line; embedded `§` characters inside an entry are kept.
fn parse_entries(text: &str) -> Vec<String> {
    let normalised = text.replace("\r\n", "\n");
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in normalised.split('\n') {
        if line.trim() == ENTRY_DELIMITER {
            let trimmed = cur.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
            cur.clear();
        } else {
            if !cur.is_empty() {
                cur.push('\n');
            }
            cur.push_str(line);
        }
    }
    let tail = cur.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

/// Inverse of [`parse_entries`]. Note we don't append a trailing newline
/// after the last entry — that way `chars().count()` reflects the actual
/// content size, not a phantom byte.
fn serialize_entries(entries: &[String]) -> String {
    let mut out = String::new();
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            out.push('\n');
            out.push_str(ENTRY_DELIMITER);
            out.push('\n');
        }
        out.push_str(e.trim());
    }
    out
}

/// Char count that *would* result from adding `extra` (if any) to the
/// existing entry list. Used for the cap pre-check on `add` and
/// `replace` so we never write a file that exceeds the limit.
fn projected_usage(entries: &[String], extra: Option<&str>) -> usize {
    let mut projected: Vec<String> = entries.to_vec();
    if let Some(extra) = extra {
        projected.push(extra.to_string());
    }
    serialize_entries(&projected).chars().count()
}

async fn write_entries(target: MemoryTarget, entries: &[String]) -> MemoryResult<()> {
    let path = file_path(target).map_err(io_other)?;
    let body = serialize_entries(entries);
    // Best-effort atomic write: write next to the destination then
    // rename. On Windows `rename` over an existing file requires
    // `MOVEFILE_REPLACE_EXISTING`, which `tokio::fs::rename` provides
    // via the Rust std layer.
    let tmp = path.with_extension("md.tmp");
    fs::write(&tmp, body.as_bytes()).await?;
    if let Err(e) = fs::rename(&tmp, &path).await {
        // Fall back to a direct write if rename fails (e.g. the file
        // doesn't exist yet on some exotic FS) — better to land the
        // edit than to lose it for filesystem pedantry.
        let _ = fs::remove_file(&tmp).await;
        fs::write(&path, body.as_bytes()).await?;
        tracing::warn!("memory rename failed, fell back to direct write: {e:#}");
    }
    Ok(())
}

fn find_unique_match(
    entries: &[String],
    needle: &str,
    target: MemoryTarget,
) -> MemoryResult<usize> {
    let mut matches: Vec<usize> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if e.contains(needle) {
            matches.push(i);
        }
    }
    match matches.len() {
        0 => Err(MemoryError::NoMatch {
            target: target.as_str(),
        }),
        1 => Ok(matches[0]),
        n => Err(MemoryError::AmbiguousMatch {
            target: target.as_str(),
            count: n,
        }),
    }
}

fn render_one(snap: &MemorySnapshot) -> String {
    // 46 char rule — same width Hermes uses; just wide enough to make
    // the header stand out without wrapping in a terminal-style chat
    // UI.
    let rule = "══════════════════════════════════════════════";
    let target = MemoryTarget::from_str(&snap.target).unwrap_or(MemoryTarget::Memory);
    let header = format!(
        "{rule}\n{label} [{pct}% — {used}/{limit} chars]\n{rule}",
        label = target.header_label(),
        pct = snap.percent(),
        used = snap.used,
        limit = snap.limit,
    );
    let body = serialize_entries(&snap.entries);
    format!("{header}\n{body}\n")
}

impl MemoryTarget {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "memory" => Some(Self::Memory),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

/// Convert an `anyhow::Error` from `paths::root()` into a `MemoryError::Io`
/// so the public API surface stays in terms of [`MemoryError`].
fn io_other(e: anyhow::Error) -> MemoryError {
    MemoryError::Io(std::io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_through_serialize() {
        let entries = vec!["alpha".to_string(), "beta\nmultiline".to_string()];
        let text = serialize_entries(&entries);
        let parsed = parse_entries(&text);
        assert_eq!(parsed, entries);
    }

    #[test]
    fn parse_ignores_blank_entries_and_trims_each() {
        let text = format!(
            "  one  \n{d}\n\n{d}\ntwo\nstill two\n{d}\n",
            d = ENTRY_DELIMITER
        );
        let parsed = parse_entries(&text);
        assert_eq!(
            parsed,
            vec!["one".to_string(), "two\nstill two".to_string()]
        );
    }

    #[test]
    fn serialize_uses_delimiter_on_its_own_line() {
        let s = serialize_entries(&["a".into(), "b".into()]);
        assert_eq!(s, "a\n§\nb");
    }

    #[test]
    fn projected_usage_accounts_for_delimiter_overhead() {
        // "a" alone = 1 char, "a\n§\nb" = 5 chars, so adding "b" to
        // ["a"] should cost 4 extra chars (the new entry + the
        // delimiter framing).
        let cur = vec!["a".to_string()];
        let proj = projected_usage(&cur, Some("b"));
        assert_eq!(proj, 5);
    }

    #[test]
    fn snapshot_percent_caps_at_100() {
        let snap = MemorySnapshot {
            target: "memory".into(),
            entries: vec![],
            used: 9000,
            limit: 2200,
            path: String::new(),
        };
        assert_eq!(snap.percent(), 100);
    }

    #[test]
    fn render_prompt_block_is_empty_when_both_stores_are_empty() {
        let state = MemoryState {
            memory: MemorySnapshot {
                target: "memory".into(),
                entries: vec![],
                used: 0,
                limit: DEFAULT_MEMORY_LIMIT,
                path: String::new(),
            },
            user: MemorySnapshot {
                target: "user".into(),
                entries: vec![],
                used: 0,
                limit: DEFAULT_USER_LIMIT,
                path: String::new(),
            },
        };
        assert!(render_prompt_block(&state).is_empty());
    }

    #[test]
    fn render_prompt_block_includes_section_header_and_percent() {
        let state = MemoryState {
            memory: MemorySnapshot {
                target: "memory".into(),
                entries: vec!["fact one".into(), "fact two".into()],
                used: 18,
                limit: 2200,
                path: String::new(),
            },
            user: MemorySnapshot {
                target: "user".into(),
                entries: vec!["user prefers concise replies".into()],
                used: 27,
                limit: 1375,
                path: String::new(),
            },
        };
        let block = render_prompt_block(&state);
        assert!(block.starts_with("\n\n# Persistent memory\n"));
        assert!(block.contains("MEMORY (your personal notes)"));
        assert!(block.contains("USER PROFILE"));
        assert!(block.contains("fact one"));
        assert!(block.contains("user prefers concise replies"));
        // Delimiter must appear between memory entries.
        assert!(block.contains("\n§\n"));
    }

    #[test]
    fn find_unique_match_reports_no_match_and_ambiguous_separately() {
        let entries = vec!["dark mode".into(), "dark theme".into(), "tabs".into()];
        let err = find_unique_match(&entries, "missing", MemoryTarget::Memory).unwrap_err();
        assert!(matches!(err, MemoryError::NoMatch { .. }));
        let err = find_unique_match(&entries, "dark", MemoryTarget::Memory).unwrap_err();
        assert!(matches!(err, MemoryError::AmbiguousMatch { count: 2, .. }));
        let idx = find_unique_match(&entries, "tabs", MemoryTarget::Memory).unwrap();
        assert_eq!(idx, 2);
    }
}
