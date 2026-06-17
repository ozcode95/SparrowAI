//! Code-navigation built-in tools: `fs.glob`, `fs.grep`, `fs.edit`.
//!
//! These complement the basic operations in [`super::fs`] with the
//! recursive search + targeted edit primitives the agent reaches for
//! when navigating an unfamiliar repository:
//!
//! - [`FsGlob`]  — recursive filename glob (`**/*.rs`, …) under a root.
//! - [`FsGrep`]  — recursive regex content search under a root.
//! - [`FsEdit`]  — targeted find/replace edit on a single file. The only
//!   destructive tool in this module; gated by the usual confirm flow.
//!
//! Path resolution piggybacks on [`super::fs::resolve_path`] so `~` and
//! relative-path semantics match the rest of the fs surface.
//!
//! Both search tools skip a small fixed list of noisy directories (VCS
//! metadata, `node_modules`, build outputs, virtualenvs, IDE caches) at
//! descent time. The list is intentionally conservative — anything you
//! actually want to inspect inside `target/` or `node_modules/` can
//! still be reached directly with `fs.read`.

use crate::mcp::tools::fs::resolve_path;
use crate::mcp::{Tool, ToolResult, ToolSchema};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use globset::{Glob, GlobMatcher};
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

/// Default cap on results returned by both search tools. Same rationale
/// as the `fs.list` limit — the model handles long listings poorly and
/// the caller can always re-call with a higher `max_results`.
const DEFAULT_GLOB_RESULTS: usize = 200;
const HARD_GLOB_RESULTS: usize = 5000;

const DEFAULT_GREP_RESULTS: usize = 200;
const HARD_GREP_RESULTS: usize = 1000;

/// Per-line truncation inside `fs.grep` output. Long minified lines or
/// pasted blobs would otherwise blow the model's context for no gain;
/// the line number is still emitted so the agent can `fs.read` the
/// exact range if it needs the full line.
const GREP_LINE_MAX: usize = 400;

/// Directories we never recurse into during glob / grep walks. Anything
/// here is either machine-generated, third-party, or local cache — the
/// agent rarely benefits from seeing it, and the cost of walking it is
/// huge (especially `node_modules`).
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
];

fn is_skipped_dir(name: &str) -> bool {
    SKIP_DIRS.iter().any(|s| *s == name)
}

/// Normalise a path to use forward slashes when matching against a
/// user-supplied glob pattern. Globs are conventionally written with
/// `/` even on Windows, and `globset` matches against the literal
/// path bytes, so we canonicalise the separator here.
fn rel_for_match(rel: &Path) -> String {
    let s = rel.to_string_lossy().into_owned();
    if std::path::MAIN_SEPARATOR == '/' {
        s
    } else {
        s.replace('\\', "/")
    }
}

// ─── fs.glob ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FsGlob;

#[derive(Debug, Deserialize)]
struct GlobArgs {
    /// Glob pattern matched against each file's path *relative to*
    /// `path`. Example: `**/*.rs`, `src/**/mod.rs`, `*.md`.
    pattern: String,
    /// Root directory to walk. Defaults to the zero process cwd.
    #[serde(default)]
    path: Option<String>,
    /// Cap on entries returned. Default 200, hard max 5000.
    #[serde(default)]
    max_results: Option<usize>,
}

#[async_trait]
impl Tool for FsGlob {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.glob".into(),
            description: "Recursively find files whose path (relative to `path`) matches a \
                 glob pattern (`**/*.rs`, `src/**/mod.rs`, …). Skips common noisy \
                 directories (.git, node_modules, target, dist, build, .next, \
                 .cache, __pycache__, .venv, venv, .idea). Results are sorted by \
                 modification time, newest first, and capped at `max_results` \
                 (default 200). Use before fs.read to find candidate files."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern, e.g. `**/*.rs` or `src/**/mod.rs`. \
                                        Matched against the path *relative to* `path` \
                                        using forward slashes."
                    },
                    "path": {
                        "type": "string",
                        "description": "Root directory to walk. Defaults to the current \
                                        working directory. `~` expands to home."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 5000,
                        "description": "Cap on results returned. Default 200."
                    }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: GlobArgs = serde_json::from_value(args).context("fs.glob: parse arguments")?;
        let limit = a
            .max_results
            .unwrap_or(DEFAULT_GLOB_RESULTS)
            .min(HARD_GLOB_RESULTS)
            .max(1);
        let root = resolve_path(a.path.as_deref().unwrap_or("."))?;
        let root_display = root.display().to_string();

        let matcher: GlobMatcher = Glob::new(&a.pattern)
            .with_context(|| format!("fs.glob: invalid pattern `{}`", a.pattern))?
            .compile_matcher();

        let pattern = a.pattern.clone();
        let (rows, total) = tokio::task::spawn_blocking(
            move || -> Result<(Vec<(PathBuf, Option<SystemTime>)>, usize)> {
                let meta =
                    std::fs::metadata(&root).with_context(|| format!("stat {}", root.display()))?;
                if !meta.is_dir() {
                    bail!("`{}` is not a directory", root.display());
                }
                let mut hits: Vec<(PathBuf, Option<SystemTime>)> = Vec::new();
                let walker = WalkDir::new(&root).follow_links(false).into_iter();
                let mut total: usize = 0;
                for entry in walker.filter_entry(|e| {
                    // Always allow the root itself.
                    if e.depth() == 0 {
                        return true;
                    }
                    let name = e.file_name().to_string_lossy();
                    if e.file_type().is_dir() && is_skipped_dir(&name) {
                        return false;
                    }
                    true
                }) {
                    let Ok(entry) = entry else { continue };
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    let rel = match entry.path().strip_prefix(&root) {
                        Ok(r) => r.to_path_buf(),
                        Err(_) => continue,
                    };
                    let key = rel_for_match(&rel);
                    if !matcher.is_match(&key) {
                        continue;
                    }
                    total += 1;
                    let mtime = entry.metadata().ok().and_then(|m| m.modified().ok());
                    hits.push((rel, mtime));
                }
                // Newest first; files without an mtime sort last.
                hits.sort_by(|a, b| match (a.1, b.1) {
                    (Some(x), Some(y)) => y.cmp(&x),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.0.cmp(&b.0),
                });
                Ok((hits, total))
            },
        )
        .await
        .map_err(|e| anyhow!("fs.glob task panicked: {e}"))??;

        let shown = rows.len().min(limit);
        let mut out = String::new();
        out.push_str(&format!(
            "{} (pattern `{}` — {} {})\n",
            root_display,
            pattern,
            total,
            if total == 1 { "match" } else { "matches" }
        ));
        for (rel, _) in rows.iter().take(shown) {
            out.push_str(&rel_for_match(rel));
            out.push('\n');
        }
        if total > shown {
            out.push_str(&format!("\n[truncated; {} more]\n", total - shown));
        }
        Ok(ToolResult {
            content: out,
            is_error: false,
        })
    }
}

// ─── fs.grep ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FsGrep;

#[derive(Debug, Deserialize)]
struct GrepArgs {
    /// Regular expression (Rust `regex` syntax) matched per-line.
    pattern: String,
    /// Root directory to walk. Defaults to the zero process cwd.
    #[serde(default)]
    path: Option<String>,
    /// Optional filename-glob filter applied to each file's path
    /// (relative to `path`) before reading it.
    #[serde(default)]
    glob: Option<String>,
    /// Match case-insensitively. Defaults to false.
    #[serde(default)]
    ignore_case: bool,
    /// Cap on matched lines returned. Default 200, hard max 1000.
    #[serde(default)]
    max_results: Option<usize>,
}

#[async_trait]
impl Tool for FsGrep {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.grep".into(),
            description: "Recursively search file contents under `path` for a regex \
                 pattern and return matching lines in `path:line:text` form. \
                 Skips the same noisy directories as fs.glob and silently \
                 skips binary / non-UTF-8 files. Use `glob` to narrow the \
                 file set (e.g. `**/*.rs`). Long matched lines are truncated \
                 to ~400 chars. Default cap 200 matches (max 1000)."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regular expression matched against each line."
                    },
                    "path": {
                        "type": "string",
                        "description": "Root directory to walk. Defaults to cwd. `~` expands to home."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Filename glob filter relative to `path` (e.g. `**/*.rs`)."
                    },
                    "ignore_case": {
                        "type": "boolean",
                        "description": "Match case-insensitively. Default false."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 1000,
                        "description": "Cap on matched lines returned. Default 200."
                    }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: GrepArgs = serde_json::from_value(args).context("fs.grep: parse arguments")?;
        let limit = a
            .max_results
            .unwrap_or(DEFAULT_GREP_RESULTS)
            .min(HARD_GREP_RESULTS)
            .max(1);
        let root = resolve_path(a.path.as_deref().unwrap_or("."))?;
        let root_display = root.display().to_string();

        let re = RegexBuilder::new(&a.pattern)
            .case_insensitive(a.ignore_case)
            .build()
            .with_context(|| format!("fs.grep: invalid regex `{}`", a.pattern))?;

        let glob_matcher: Option<GlobMatcher> = match a.glob.as_deref() {
            Some(g) => Some(
                Glob::new(g)
                    .with_context(|| format!("fs.grep: invalid glob `{g}`"))?
                    .compile_matcher(),
            ),
            None => None,
        };

        let pattern = a.pattern.clone();
        let (lines, total, files_scanned) =
            tokio::task::spawn_blocking(move || -> Result<(Vec<String>, usize, usize)> {
                let meta =
                    std::fs::metadata(&root).with_context(|| format!("stat {}", root.display()))?;
                if !meta.is_dir() {
                    bail!("`{}` is not a directory", root.display());
                }
                let mut out_lines: Vec<String> = Vec::new();
                let mut total: usize = 0;
                let mut files_scanned: usize = 0;
                let walker = WalkDir::new(&root).follow_links(false).into_iter();
                'files: for entry in walker.filter_entry(|e| {
                    if e.depth() == 0 {
                        return true;
                    }
                    let name = e.file_name().to_string_lossy();
                    if e.file_type().is_dir() && is_skipped_dir(&name) {
                        return false;
                    }
                    true
                }) {
                    let Ok(entry) = entry else { continue };
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    let rel = match entry.path().strip_prefix(&root) {
                        Ok(r) => r.to_path_buf(),
                        Err(_) => continue,
                    };
                    let rel_str = rel_for_match(&rel);
                    if let Some(m) = &glob_matcher {
                        if !m.is_match(&rel_str) {
                            continue;
                        }
                    }
                    // Silently skip non-UTF-8 / binary files.
                    let Ok(text) = std::fs::read_to_string(entry.path()) else {
                        continue;
                    };
                    files_scanned += 1;
                    for (idx, line) in text.lines().enumerate() {
                        if re.is_match(line) {
                            total += 1;
                            if out_lines.len() < limit {
                                let trimmed = truncate_for_grep(line);
                                out_lines.push(format!("{}:{}:{}", rel_str, idx + 1, trimmed));
                            } else if total > limit + 1 {
                                // Keep counting up to a sane bound, then bail to
                                // avoid pathological grep runs over huge trees.
                                if total >= limit * 10 {
                                    break 'files;
                                }
                            }
                        }
                    }
                }
                Ok((out_lines, total, files_scanned))
            })
            .await
            .map_err(|e| anyhow!("fs.grep task panicked: {e}"))??;

        let mut out = String::new();
        out.push_str(&format!(
            "{} (regex `{}` — {} {} in {} {})\n",
            root_display,
            pattern,
            total,
            if total == 1 { "match" } else { "matches" },
            files_scanned,
            if files_scanned == 1 { "file" } else { "files" }
        ));
        if lines.is_empty() {
            out.push_str("(no matches)\n");
        } else {
            for l in &lines {
                out.push_str(l);
                out.push('\n');
            }
        }
        if total > lines.len() {
            out.push_str(&format!(
                "\n[truncated; {} more matches]\n",
                total - lines.len()
            ));
        }
        Ok(ToolResult {
            content: out,
            is_error: false,
        })
    }
}

fn truncate_for_grep(line: &str) -> String {
    // Use char boundaries to avoid splitting multi-byte codepoints.
    if line.chars().count() <= GREP_LINE_MAX {
        return line.to_string();
    }
    let mut out: String = line.chars().take(GREP_LINE_MAX).collect();
    out.push_str(" … [truncated]");
    out
}

// ─── fs.edit ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FsEdit;

#[derive(Debug, Deserialize)]
struct EditArgs {
    /// Path to the file to edit. Must exist.
    path: String,
    /// Exact substring to find. Whitespace is *not* normalised.
    old_string: String,
    /// Replacement substring. May be empty (to delete).
    new_string: String,
    /// When false (the default), refuse to edit unless `old_string`
    /// occurs *exactly once* — preventing accidental mass rewrites.
    /// When true, every occurrence is replaced.
    #[serde(default)]
    replace_all: bool,
}

#[async_trait]
impl Tool for FsEdit {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fs.edit".into(),
            description: "Apply a targeted find/replace to a single text file. By \
                 default `old_string` must occur exactly once in the file — \
                 the tool errors otherwise so the agent can add surrounding \
                 context until the match is unique. Set `replace_all: true` \
                 to replace every occurrence. This tool is destructive — the \
                 user is prompted before each call unless they've disabled \
                 the confirm gate in Settings."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "old_string", "new_string"],
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path. `~` expands to home."
                    },
                    "old_string": {
                        "type": "string",
                        "description": "Exact substring to find. Include enough surrounding \
                                        context to make the match unique."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Replacement substring. May be empty to delete."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace every occurrence instead of requiring a \
                                        unique match. Default false."
                    }
                }
            }),
            destructive: true,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: EditArgs = serde_json::from_value(args).context("fs.edit: parse arguments")?;
        if a.old_string.is_empty() {
            bail!("fs.edit: `old_string` must not be empty");
        }
        if a.old_string == a.new_string {
            bail!("fs.edit: `old_string` and `new_string` are identical — nothing to do");
        }
        let path = resolve_path(&a.path)?;
        let display = path.display().to_string();

        let summary = tokio::task::spawn_blocking(move || -> Result<String> {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            let count = text.matches(&a.old_string).count();
            if count == 0 {
                bail!(
                    "fs.edit: `old_string` not found in {}. The file is unchanged. \
                     If you're sure of the contents, try fs.read first to confirm \
                     exact whitespace and indentation.",
                    path.display()
                );
            }
            if count > 1 && !a.replace_all {
                bail!(
                    "fs.edit: `old_string` is not unique in {} ({} matches). \
                     Add surrounding context to disambiguate, or pass \
                     `replace_all: true` to replace every occurrence.",
                    path.display(),
                    count
                );
            }
            let new_text = if a.replace_all {
                text.replace(&a.old_string, &a.new_string)
            } else {
                // count == 1
                text.replacen(&a.old_string, &a.new_string, 1)
            };
            std::fs::write(&path, new_text.as_bytes())
                .with_context(|| format!("write {}", path.display()))?;
            Ok(format!(
                "Edited {} ({} replacement{})",
                path.display(),
                count,
                if count == 1 { "" } else { "s" }
            ))
        })
        .await
        .map_err(|e| anyhow!("fs.edit task panicked: {e}"))??;

        let _ = display; // kept for symmetry with other fs tools; summary already contains the path
        Ok(ToolResult {
            content: summary,
            is_error: false,
        })
    }
}

// ─── Registry helper ───────────────────────────────────────────────────────

/// Build the boxed-trait list of every code-navigation tool. Called from
/// [`crate::mcp::builtin_registry`] alongside [`super::fs::all`].
pub fn all() -> Vec<Box<dyn Tool>> {
    vec![Box::new(FsGlob), Box::new(FsGrep), Box::new(FsEdit)]
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static CTR: AtomicU64 = AtomicU64::new(0);

    fn tmpdir() -> PathBuf {
        let i = CTR.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("zero-code-test-{}-{}", std::process::id(), i));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    // ─── fs.glob ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fs_glob_finds_freshly_created_file() {
        let d = tmpdir();
        std::fs::write(d.join("hello.rs"), b"fn main() {}").unwrap();
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src").join("lib.rs"), b"// lib").unwrap();
        std::fs::write(d.join("README.md"), b"# readme").unwrap();

        let out = FsGlob
            .call(json!({
                "pattern": "**/*.rs",
                "path": d.to_string_lossy(),
            }))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(
            out.content.contains("hello.rs"),
            "missing hello.rs in: {}",
            out.content
        );
        assert!(
            out.content.contains("src/lib.rs"),
            "missing src/lib.rs in: {}",
            out.content
        );
        assert!(!out.content.contains("README.md"));
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_glob_skips_git_subdir() {
        let d = tmpdir();
        std::fs::create_dir_all(d.join(".git")).unwrap();
        std::fs::write(d.join(".git").join("config"), b"[core]").unwrap();
        std::fs::write(d.join(".git").join("HEAD"), b"ref: refs/heads/main").unwrap();
        std::fs::write(d.join("keep.txt"), b"visible").unwrap();

        let out = FsGlob
            .call(json!({
                "pattern": "**/*",
                "path": d.to_string_lossy(),
            }))
            .await
            .unwrap();
        assert!(out.content.contains("keep.txt"));
        assert!(
            !out.content.contains(".git"),
            ".git contents leaked into glob output: {}",
            out.content
        );
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_glob_is_not_destructive() {
        assert!(!FsGlob.schema().destructive);
    }

    // ─── fs.grep ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fs_grep_finds_pattern_across_files() {
        let d = tmpdir();
        std::fs::write(d.join("a.txt"), b"alpha\nbravo TARGET here\ncharlie\n").unwrap();
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub").join("b.txt"), b"nothing\nTARGET line\n").unwrap();
        std::fs::write(d.join("noise.txt"), b"no hits here\n").unwrap();

        let out = FsGrep
            .call(json!({
                "pattern": "TARGET",
                "path": d.to_string_lossy(),
            }))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(
            out.content.contains("a.txt:2:"),
            "want a.txt:2 in: {}",
            out.content
        );
        assert!(
            out.content.contains("sub/b.txt:2:"),
            "want sub/b.txt:2 in: {}",
            out.content
        );
        assert!(out.content.contains("bravo TARGET here"));
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_grep_returns_no_matches_cleanly() {
        let d = tmpdir();
        std::fs::write(d.join("a.txt"), b"alpha\nbravo\n").unwrap();
        let out = FsGrep
            .call(json!({
                "pattern": "ZZZ_definitely_absent_ZZZ",
                "path": d.to_string_lossy(),
            }))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("(no matches)"), "got: {}", out.content);
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_grep_respects_ignore_case() {
        let d = tmpdir();
        std::fs::write(d.join("a.txt"), b"Hello World\nGOODBYE\n").unwrap();

        // Case-sensitive: no match for `hello`.
        let sensitive = FsGrep
            .call(json!({
                "pattern": "hello",
                "path": d.to_string_lossy(),
            }))
            .await
            .unwrap();
        assert!(sensitive.content.contains("(no matches)"));

        // Case-insensitive: matches `Hello`.
        let insensitive = FsGrep
            .call(json!({
                "pattern": "hello",
                "path": d.to_string_lossy(),
                "ignore_case": true,
            }))
            .await
            .unwrap();
        assert!(
            insensitive.content.contains("a.txt:1:Hello World"),
            "expected hit, got: {}",
            insensitive.content
        );
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_grep_is_not_destructive() {
        assert!(!FsGrep.schema().destructive);
    }

    // ─── fs.edit ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fs_edit_replaces_unique_occurrence() {
        let d = tmpdir();
        let f = d.join("x.txt");
        std::fs::write(&f, b"hello world").unwrap();
        let out = FsEdit
            .call(json!({
                "path": f.to_string_lossy(),
                "old_string": "world",
                "new_string": "Rust",
            }))
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(
            out.content.contains("1 replacement"),
            "got: {}",
            out.content
        );
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hello Rust");
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_edit_errors_on_missing_match() {
        let d = tmpdir();
        let f = d.join("x.txt");
        std::fs::write(&f, b"hello world").unwrap();
        let res = FsEdit
            .call(json!({
                "path": f.to_string_lossy(),
                "old_string": "absent",
                "new_string": "X",
            }))
            .await;
        assert!(res.is_err(), "expected error, got: {:?}", res);
        // File should be unchanged.
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hello world");
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_edit_errors_on_duplicate_without_replace_all() {
        let d = tmpdir();
        let f = d.join("x.txt");
        std::fs::write(&f, b"a\na\na\n").unwrap();
        let res = FsEdit
            .call(json!({
                "path": f.to_string_lossy(),
                "old_string": "a",
                "new_string": "b",
            }))
            .await;
        let err = res.expect_err("expected error on duplicate match");
        let msg = format!("{err:#}");
        assert!(msg.contains("not unique"), "unexpected error: {msg}");
        assert!(
            msg.contains("3 matches"),
            "want match count in error: {msg}"
        );
        // File should be unchanged.
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "a\na\na\n");
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_edit_replace_all_on_duplicate() {
        let d = tmpdir();
        let f = d.join("x.txt");
        std::fs::write(&f, b"a\na\na\n").unwrap();
        let out = FsEdit
            .call(json!({
                "path": f.to_string_lossy(),
                "old_string": "a",
                "new_string": "b",
                "replace_all": true,
            }))
            .await
            .unwrap();
        assert!(
            out.content.contains("3 replacements"),
            "got: {}",
            out.content
        );
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "b\nb\nb\n");
        cleanup(&d);
    }

    #[tokio::test]
    async fn fs_edit_is_flagged_destructive() {
        assert!(FsEdit.schema().destructive);
    }
}
