//! Local filesystem tools the agent loop can call.
//!
//! Implements [`crate::mcp::Tool`] for a small but practically complete
//! set of operations: list a directory, read a file, write a file, and
//! stat a path. These run in-process under the user's own privileges —
//! anything the user can do at a shell, the model can do here once
//! these tools are enabled, with [`fs.write`] gated by the global
//! destructive-tool confirm flow.
//!
//! Path handling rules (kept consistent across every tool here):
//!
//! - Accept native separators on the host OS (`\\` on Windows, `/` on
//!   Unix) plus forward slashes everywhere as a convenience.
//! - Leading `~` / `~/` expands to the user's home directory.
//! - Relative paths resolve against `std::env::current_dir()`. We do
//!   *not* canonicalise — that would resolve symlinks under the user's
//!   feet, which is the opposite of what they usually want when listing.
//!
//! No allow-list of directories: zero is a local single-user app, and
//! over-restricting paths just trains users to disable the safeguard.
//! The confirm gate on `fs.write` is the actual safety boundary.

use crate::mcp::{Tool, ToolResult, ToolSchema};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::SystemTime;

/// Default upper bound on directory listings. The model handles long
/// listings poorly anyway; if the user wants more they can re-call with
/// an explicit `max_entries`.
const DEFAULT_LIST_LIMIT: usize = 200;

/// Default cap on bytes returned by `fs.read`. ~256 KB comfortably
/// covers most source files while keeping the resulting `tool` message
/// well under typical model context budgets.
const DEFAULT_READ_BYTES: u64 = 256 * 1024;

// ─── fs.list ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FsList;

#[derive(Debug, Deserialize)]
struct ListArgs {
    path: String,
    #[serde(default)]
    max_entries: Option<usize>,
    /// Show entries whose name starts with `.` (Unix hidden files) and
    /// files Windows marks `Hidden`. Off by default to keep the agent
    /// focused on user-visible content.
    #[serde(default)]
    include_hidden: bool,
}

#[async_trait]
impl Tool for FsList {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.list".into(),
            description: "List the entries in a directory on the local filesystem. Returns \
                 type / size / modified-time / name for each entry, capped at \
                 `max_entries` (default 200). Use this before fs.read to discover \
                 what's available."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative directory path. `~` expands to home."
                    },
                    "max_entries": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 5000,
                        "description": "Cap on entries returned. Default 200."
                    },
                    "include_hidden": {
                        "type": "boolean",
                        "description": "Include dotfiles / hidden entries. Default false."
                    }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: ListArgs = serde_json::from_value(args).context("fs.list: parse arguments")?;
        let limit = a.max_entries.unwrap_or(DEFAULT_LIST_LIMIT).min(5000).max(1);
        let dir = resolve_path(&a.path)?;
        let dir_display = dir.display().to_string();

        let result = tokio::task::spawn_blocking(move || -> Result<(Vec<Row>, usize, bool)> {
            let meta =
                std::fs::metadata(&dir).with_context(|| format!("stat {}", dir.display()))?;
            if !meta.is_dir() {
                bail!("`{}` is not a directory", dir.display());
            }
            let mut rows: Vec<Row> = Vec::new();
            let mut total: usize = 0;
            let mut truncated = false;
            for entry in
                std::fs::read_dir(&dir).with_context(|| format!("read_dir {}", dir.display()))?
            {
                let Ok(entry) = entry else { continue };
                let name = entry.file_name().to_string_lossy().into_owned();
                if !a.include_hidden && name.starts_with('.') {
                    continue;
                }
                total += 1;
                if rows.len() >= limit {
                    truncated = true;
                    continue;
                }
                let md = entry.metadata().ok();
                rows.push(Row {
                    kind: classify(md.as_ref()),
                    size: md.as_ref().map(|m| m.len()).unwrap_or(0),
                    modified: md.as_ref().and_then(|m| m.modified().ok()),
                    name,
                });
            }
            rows.sort_by(|a, b| match (a.kind, b.kind) {
                (Kind::Dir, Kind::File) => std::cmp::Ordering::Less,
                (Kind::File, Kind::Dir) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            });
            Ok((rows, total, truncated))
        })
        .await
        .map_err(|e| anyhow!("fs.list task panicked: {e}"))??;

        let (rows, total, truncated) = result;
        let mut out = String::new();
        out.push_str(&format!(
            "{} ({} {})\n",
            dir_display,
            total,
            if total == 1 { "entry" } else { "entries" }
        ));
        for r in &rows {
            out.push_str(&format!(
                "[{}] {:>10}  {}  {}\n",
                r.kind.tag(),
                format_size(r.size, r.kind),
                format_mtime(r.modified),
                r.name
            ));
        }
        if truncated {
            out.push_str(&format!(
                "\n[truncated; showing {} of {} entries — re-call with max_entries to see more]\n",
                rows.len(),
                total
            ));
        }
        Ok(ToolResult {
            content: out,
            is_error: false,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
    Symlink,
    Other,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::File => "f",
            Kind::Dir => "d",
            Kind::Symlink => "l",
            Kind::Other => "?",
        }
    }
}

#[derive(Debug)]
struct Row {
    kind: Kind,
    size: u64,
    modified: Option<SystemTime>,
    name: String,
}

fn classify(md: Option<&std::fs::Metadata>) -> Kind {
    let Some(md) = md else {
        return Kind::Other;
    };
    let ft = md.file_type();
    if ft.is_dir() {
        Kind::Dir
    } else if ft.is_symlink() {
        Kind::Symlink
    } else if ft.is_file() {
        Kind::File
    } else {
        Kind::Other
    }
}

fn format_size(bytes: u64, kind: Kind) -> String {
    if kind == Kind::Dir {
        return "-".into();
    }
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2}G", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn format_mtime(mt: Option<SystemTime>) -> String {
    match mt {
        Some(t) => {
            let dt: DateTime<Local> = t.into();
            dt.format("%Y-%m-%d %H:%M").to_string()
        }
        None => "-               ".into(),
    }
}

// ─── fs.read ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FsRead;

#[derive(Debug, Deserialize)]
struct ReadArgs {
    path: String,
    /// Maximum bytes to return. Defaults to `DEFAULT_READ_BYTES`; the
    /// resulting text is truncated with a marker when the file is larger.
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[async_trait]
impl Tool for FsRead {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.read".into(),
            description: "Read a text file from the local filesystem. Returns the file's \
                 contents, truncated to `max_bytes` (default 256 KiB) when larger. \
                 Binary content is returned as a lossy UTF-8 string."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path. `~` expands to home."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum bytes to return. Default 262144 (256 KiB)."
                    }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: ReadArgs = serde_json::from_value(args).context("fs.read: parse arguments")?;
        let path = resolve_path(&a.path)?;
        let limit = a.max_bytes.unwrap_or(DEFAULT_READ_BYTES).max(1);
        let display = path.display().to_string();

        let (text, was_truncated, total_size) =
            tokio::task::spawn_blocking(move || -> Result<(String, bool, u64)> {
                use std::io::Read;
                let md =
                    std::fs::metadata(&path).with_context(|| format!("stat {}", path.display()))?;
                if md.is_dir() {
                    bail!("`{}` is a directory — use fs.list instead", path.display());
                }
                let size = md.len();
                let cap = limit.min(size) as usize;
                let mut f = std::fs::File::open(&path)
                    .with_context(|| format!("open {}", path.display()))?;
                let mut buf = Vec::with_capacity(cap.min(1 << 20));
                f.by_ref()
                    .take(limit)
                    .read_to_end(&mut buf)
                    .with_context(|| format!("read {}", path.display()))?;
                let text = String::from_utf8_lossy(&buf).into_owned();
                Ok((text, size > limit, size))
            })
            .await
            .map_err(|e| anyhow!("fs.read task panicked: {e}"))??;

        let mut out = format!("{} ({} bytes)\n", display, total_size);
        out.push_str(&text);
        if was_truncated {
            out.push_str(&format!(
                "\n[truncated to {} bytes of {}; re-call with max_bytes to read more]\n",
                text.len(),
                total_size
            ));
        }
        Ok(ToolResult {
            content: out,
            is_error: false,
        })
    }
}

// ─── fs.stat ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FsStat;

#[derive(Debug, Deserialize)]
struct StatArgs {
    path: String,
}

#[async_trait]
impl Tool for FsStat {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.stat".into(),
            description: "Return metadata for a path: type (file / directory / symlink), \
                 size in bytes, and last-modified timestamp. Useful before \
                 deciding whether to fs.read or fs.list."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string" }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: StatArgs = serde_json::from_value(args).context("fs.stat: parse arguments")?;
        let path = resolve_path(&a.path)?;
        let display = path.display().to_string();
        let md = tokio::task::spawn_blocking(move || std::fs::symlink_metadata(&path))
            .await
            .map_err(|e| anyhow!("fs.stat task panicked: {e}"))?
            .with_context(|| format!("stat {display}"))?;
        let kind = classify(Some(&md));
        let modified = format_mtime(md.modified().ok());
        let out = format!(
            "{display}\n  type: {}\n  size: {} bytes\n  modified: {}\n",
            match kind {
                Kind::Dir => "directory",
                Kind::File => "file",
                Kind::Symlink => "symlink",
                Kind::Other => "other",
            },
            md.len(),
            modified,
        );
        Ok(ToolResult {
            content: out,
            is_error: false,
        })
    }
}

// ─── fs.write ──────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FsWrite;

#[derive(Debug, Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
    /// When true, append to an existing file instead of overwriting.
    /// Defaults to false (overwrite) to match the principle of least
    /// surprise from the tool name.
    #[serde(default)]
    append: bool,
    /// Create missing parent directories. Defaults to true.
    #[serde(default = "default_true")]
    create_parents: bool,
}

fn default_true() -> bool {
    true
}

#[async_trait]
impl Tool for FsWrite {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.write".into(),
            description: "Write text to a file on the local filesystem. By default, \
                 overwrites any existing file at the path and creates missing \
                 parent directories. Use `append: true` to append. This tool is \
                 destructive — the user will be prompted to confirm each call \
                 unless they've disabled the confirm gate in Settings."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "content"],
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path. `~` expands to home."
                    },
                    "content": {
                        "type": "string",
                        "description": "UTF-8 text to write."
                    },
                    "append": {
                        "type": "boolean",
                        "description": "Append instead of overwriting. Default false."
                    },
                    "create_parents": {
                        "type": "boolean",
                        "description": "Create missing parent directories. Default true."
                    }
                }
            }),
            destructive: true,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: WriteArgs = serde_json::from_value(args).context("fs.write: parse arguments")?;
        let path = resolve_path(&a.path)?;
        let display = path.display().to_string();
        let bytes_written = a.content.len();

        tokio::task::spawn_blocking(move || -> Result<()> {
            use std::io::Write;
            if a.create_parents {
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)
                            .with_context(|| format!("mkdir -p {}", parent.display()))?;
                    }
                }
            }
            let mut opts = std::fs::OpenOptions::new();
            opts.create(true).write(true);
            if a.append {
                opts.append(true);
            } else {
                opts.truncate(true);
            }
            let mut f = opts
                .open(&path)
                .with_context(|| format!("open {} for write", path.display()))?;
            f.write_all(a.content.as_bytes())
                .with_context(|| format!("write {}", path.display()))?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow!("fs.write task panicked: {e}"))??;

        Ok(ToolResult {
            content: format!(
                "wrote {bytes_written} bytes to {display} ({})",
                if a.append { "append" } else { "overwrite" }
            ),
            is_error: false,
        })
    }
}

// ─── Path resolution ───────────────────────────────────────────────────────

/// Turn a user-supplied string into an absolute [`PathBuf`].
///
/// - Expands a leading `~` / `~/` / `~\\` to the user's home directory.
/// - Resolves relative paths against the current working directory.
/// - Does **not** canonicalise — symlinks are preserved as written so a
///   `fs.list` of a symlinked directory returns its actual contents
///   without surprising the user.
pub(crate) fn resolve_path(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("path is empty");
    }
    // `~` / `~/something` / `~\\something` → home dir.
    let expanded = if trimmed == "~" {
        home_dir()?
    } else if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        home_dir()?.join(rest)
    } else {
        PathBuf::from(trimmed)
    };

    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        let cwd = std::env::current_dir().context("fs: read current dir for relative path")?;
        Ok(cwd.join(expanded))
    }
}

fn home_dir() -> Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or_else(|| anyhow!("could not determine user home directory"))
}

// ─── Registry helper ───────────────────────────────────────────────────────

/// Build the boxed-trait list of every built-in fs tool. Called from
/// [`crate::mcp::builtin_registry`] so the chat catalog picks them up.
pub fn all() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(FsList),
        Box::new(FsRead),
        Box::new(FsStat),
        Box::new(FsWrite),
    ]
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static CTR: AtomicU64 = AtomicU64::new(0);

    fn tmpdir() -> PathBuf {
        let i = CTR.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("zero-fs-test-{}-{}", std::process::id(), i));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    #[test]
    fn resolve_path_expands_tilde_root() {
        let p = resolve_path("~").unwrap();
        assert!(p.is_absolute());
        assert_eq!(p, directories::BaseDirs::new().unwrap().home_dir());
    }

    #[test]
    fn resolve_path_expands_tilde_subdir() {
        let p = resolve_path("~/Downloads").unwrap();
        let expected = directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .join("Downloads");
        assert_eq!(p, expected);
    }

    #[test]
    fn resolve_path_keeps_absolute_unchanged() {
        #[cfg(windows)]
        let raw = "C:\\Windows\\System32";
        #[cfg(not(windows))]
        let raw = "/usr/local/bin";
        let p = resolve_path(raw).unwrap();
        assert!(p.is_absolute());
        assert_eq!(p.to_string_lossy(), raw);
    }

    #[test]
    fn resolve_path_rejects_empty() {
        assert!(resolve_path("   ").is_err());
        assert!(resolve_path("").is_err());
    }

    #[tokio::test]
    async fn fs_list_returns_files_and_dirs_with_header() {
        let d = tmpdir();
        std::fs::write(d.join("a.txt"), b"hi").unwrap();
        std::fs::write(d.join("b.bin"), vec![0u8; 2048]).unwrap();
        std::fs::create_dir(d.join("nested")).unwrap();

        let out = FsList
            .call(json!({ "path": d.to_string_lossy() }))
            .await
            .unwrap();
        assert!(!out.is_error);
        let body = out.content;
        // Header carries the absolute path + correct entry count.
        assert!(body.contains("3 entries"), "body was: {body}");
        // Each entry is rendered with its type tag.
        assert!(body.contains("[d]"));
        assert!(body.contains("[f]"));
        assert!(body.contains("a.txt"));
        assert!(body.contains("b.bin"));
        assert!(body.contains("nested"));
        // Directories sort before files.
        let dir_pos = body.find("nested").unwrap();
        let file_pos = body.find("a.txt").unwrap();
        assert!(dir_pos < file_pos, "dirs should sort before files: {body}");
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_list_skips_hidden_unless_requested() {
        let d = tmpdir();
        std::fs::write(d.join(".secret"), b"x").unwrap();
        std::fs::write(d.join("visible"), b"x").unwrap();

        let hidden_off = FsList
            .call(json!({ "path": d.to_string_lossy() }))
            .await
            .unwrap();
        assert!(!hidden_off.content.contains(".secret"));
        assert!(hidden_off.content.contains("visible"));

        let hidden_on = FsList
            .call(json!({
                "path": d.to_string_lossy(),
                "include_hidden": true
            }))
            .await
            .unwrap();
        assert!(hidden_on.content.contains(".secret"));
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_list_truncates_at_max_entries() {
        let d = tmpdir();
        for i in 0..10 {
            std::fs::write(d.join(format!("f{i:02}.txt")), b"x").unwrap();
        }
        let out = FsList
            .call(json!({ "path": d.to_string_lossy(), "max_entries": 3 }))
            .await
            .unwrap();
        assert!(out.content.contains("10 entries"));
        assert!(out.content.contains("[truncated"));
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_list_errors_on_file_path() {
        let d = tmpdir();
        let f = d.join("file.txt");
        std::fs::write(&f, b"hi").unwrap();
        let res = FsList.call(json!({ "path": f.to_string_lossy() })).await;
        assert!(res.is_err());
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_read_returns_contents_with_header() {
        let d = tmpdir();
        let f = d.join("hello.txt");
        std::fs::write(&f, "hello, world\n").unwrap();
        let out = FsRead
            .call(json!({ "path": f.to_string_lossy() }))
            .await
            .unwrap();
        assert!(out.content.contains("hello, world"));
        assert!(out.content.contains("13 bytes"));
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_read_truncates_oversized_file() {
        let d = tmpdir();
        let f = d.join("big.txt");
        let bytes = vec![b'q'; 10_000];
        std::fs::write(&f, &bytes).unwrap();
        let out = FsRead
            .call(json!({ "path": f.to_string_lossy(), "max_bytes": 100 }))
            .await
            .unwrap();
        assert!(out.content.contains("[truncated"));
        // The body carries exactly the cap; we wrote distinct payload
        // bytes (`q`) so they can't be confused with the header path.
        assert_eq!(
            out.content.matches('q').count(),
            100,
            "expected 100 `q`s in body, got: {}",
            out.content
        );
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_read_errors_on_directory() {
        let d = tmpdir();
        let res = FsRead.call(json!({ "path": d.to_string_lossy() })).await;
        assert!(res.is_err());
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_stat_reports_kind_and_size() {
        let d = tmpdir();
        let f = d.join("x.txt");
        std::fs::write(&f, b"hello").unwrap();
        let out = FsStat
            .call(json!({ "path": f.to_string_lossy() }))
            .await
            .unwrap();
        assert!(out.content.contains("type: file"));
        assert!(out.content.contains("size: 5 bytes"));

        let dir_out = FsStat
            .call(json!({ "path": d.to_string_lossy() }))
            .await
            .unwrap();
        assert!(dir_out.content.contains("type: directory"));
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_write_overwrites_by_default() {
        let d = tmpdir();
        let f = d.join("out.txt");
        std::fs::write(&f, b"old").unwrap();
        let out = FsWrite
            .call(json!({
                "path": f.to_string_lossy(),
                "content": "new"
            }))
            .await
            .unwrap();
        assert!(out.content.contains("wrote 3 bytes"));
        assert!(out.content.contains("overwrite"));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "new");
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_write_appends_when_requested() {
        let d = tmpdir();
        let f = d.join("log.txt");
        std::fs::write(&f, b"first\n").unwrap();
        FsWrite
            .call(json!({
                "path": f.to_string_lossy(),
                "content": "second\n",
                "append": true
            }))
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "first\nsecond\n");
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_write_creates_missing_parents_by_default() {
        let d = tmpdir();
        let nested = d.join("a").join("b").join("c.txt");
        FsWrite
            .call(json!({
                "path": nested.to_string_lossy(),
                "content": "x"
            }))
            .await
            .unwrap();
        assert!(nested.exists());
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_write_is_flagged_destructive() {
        assert!(FsWrite.schema().destructive);
    }

    #[tokio::test]
    async fn read_and_stat_are_not_destructive() {
        assert!(!FsRead.schema().destructive);
        assert!(!FsList.schema().destructive);
        assert!(!FsStat.schema().destructive);
    }
}
