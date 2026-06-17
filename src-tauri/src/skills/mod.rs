//! User-authored skills.
//!
//! A "skill" is a small markdown file (`SKILL.md`) plus optional supporting
//! resources, living under `~/.zero/skills/<id>/`. When enabled, the body
//! is appended to the system prompt for every new chat turn so the user
//! can teach the assistant project-specific conventions, tools, persona,
//! etc. — analogous to the agent-skills pattern other tools have adopted.
//!
//! Frontmatter parsing is intentionally minimal: we accept a `---`-fenced
//! YAML-ish block at the top of `SKILL.md` and extract two keys — `name`
//! and `description` — using a hand-rolled parser so we don't pull in
//! `serde_yaml` for this one feature. Everything below the closing fence
//! is the prompt body.

use crate::paths;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

/// Soft cap on a single skill body (after frontmatter). Skills are
/// concatenated into every system prompt — without a cap a stray 5MB
/// SKILL.md would silently nuke the model's context window every turn.
pub const MAX_SKILL_BODY_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Folder name. URL-safe (`a-zA-Z0-9_-`) so it can be referenced from
    /// settings.skills_enabled without escaping.
    pub id: String,
    /// Human-friendly label parsed from the frontmatter `name` field.
    /// Falls back to `id` when the file has no frontmatter.
    pub name: String,
    /// One-line summary from frontmatter `description`. Optional.
    #[serde(default)]
    pub description: Option<String>,
    /// Bytes of the prompt body (post-frontmatter). Surfaced in the UI so
    /// the user can see at a glance whether a skill is "small note" or
    /// "novella".
    pub body_bytes: u64,
    /// Absolute path of `SKILL.md`. Lets the UI open the file in $EDITOR.
    pub path: String,
}

/// Full skill record including the parsed prompt body. Used by the runner
/// when injecting enabled skills into the system prompt — never sent over
/// IPC so the body doesn't get round-tripped through JSON for the UI.
pub struct SkillWithBody {
    pub meta: Skill,
    pub body: String,
}

/// Enumerate every skill present on disk. Skills with malformed
/// frontmatter still load (they just get default metadata) so a typo
/// doesn't make the whole list disappear.
pub async fn list() -> Result<Vec<Skill>> {
    let dir = paths::skills_dir()?;
    let mut out = Vec::new();
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        if !file_type.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if !is_valid_id(&id) {
            tracing::debug!("skipping skill with invalid id: {id}");
            continue;
        }
        match load_meta(&id).await {
            Ok(meta) => out.push(meta),
            Err(e) => tracing::warn!("failed to load skill {id}: {e:#}"),
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// Load a single skill's metadata.
pub async fn load_meta(id: &str) -> Result<Skill> {
    let path = paths::skills_dir()?.join(id).join("SKILL.md");
    let bytes = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let (front, body) = split_frontmatter(&bytes);
    let (name, description) = parse_frontmatter(front);
    Ok(Skill {
        id: id.to_string(),
        name: name.unwrap_or_else(|| id.to_string()),
        description,
        body_bytes: body.len() as u64,
        path: path.to_string_lossy().to_string(),
    })
}

/// Load a skill's full body for prompt injection. Same parsing rules as
/// [`load_meta`], plus the post-frontmatter prompt text (length-capped).
pub async fn load_with_body(id: &str) -> Result<SkillWithBody> {
    let path = paths::skills_dir()?.join(id).join("SKILL.md");
    let bytes = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let (front, body) = split_frontmatter(&bytes);
    let (name, description) = parse_frontmatter(front);
    let body = truncate_body(body);
    let body_bytes = body.len() as u64;
    Ok(SkillWithBody {
        meta: Skill {
            id: id.to_string(),
            name: name.unwrap_or_else(|| id.to_string()),
            description,
            body_bytes,
            path: path.to_string_lossy().to_string(),
        },
        body,
    })
}

/// Create a new skill. `id` is validated, the directory is materialised,
/// and a starter `SKILL.md` is written. Errors if a skill with the same
/// id already exists.
pub async fn create(id: &str, name: &str, description: Option<&str>, body: &str) -> Result<Skill> {
    if !is_valid_id(id) {
        return Err(anyhow!(
            "invalid skill id `{id}`: use only letters, digits, `-` and `_`"
        ));
    }
    let dir = paths::skills_dir()?.join(id);
    if dir.exists() {
        return Err(anyhow!("skill `{id}` already exists"));
    }
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join("SKILL.md");
    tokio::fs::write(&path, render_skill_md(name, description, body))
        .await
        .with_context(|| format!("write {}", path.display()))?;
    load_meta(id).await
}

/// Rewrite an existing skill's `SKILL.md` from the supplied fields.
pub async fn update(id: &str, name: &str, description: Option<&str>, body: &str) -> Result<Skill> {
    let dir = paths::skills_dir()?.join(id);
    if !dir.exists() {
        return Err(anyhow!("no skill `{id}`"));
    }
    let path = dir.join("SKILL.md");
    tokio::fs::write(&path, render_skill_md(name, description, body))
        .await
        .with_context(|| format!("write {}", path.display()))?;
    load_meta(id).await
}

fn render_skill_md(name: &str, description: Option<&str>, body: &str) -> String {
    let mut content = String::new();
    content.push_str("---\n");
    content.push_str(&format!("name: {}\n", name.trim()));
    if let Some(desc) = description {
        let desc = desc.trim();
        if !desc.is_empty() {
            content.push_str(&format!("description: {desc}\n"));
        }
    }
    content.push_str("---\n\n");
    content.push_str(body.trim_start());
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content
}

/// Drop a skill folder (and everything under it). Idempotent.
pub async fn delete(id: &str) -> Result<()> {
    if !is_valid_id(id) {
        return Err(anyhow!("invalid skill id"));
    }
    let dir = paths::skills_dir()?.join(id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await?;
    }
    Ok(())
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() < 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn truncate_body(body: &str) -> String {
    if body.len() <= MAX_SKILL_BODY_BYTES {
        return body.to_string();
    }
    let mut cut = MAX_SKILL_BODY_BYTES;
    while !body.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    let mut out = body[..cut].to_string();
    out.push_str("\n\n[truncated]");
    out
}

/// Split `---`-fenced frontmatter off the top of the file. Returns
/// `(frontmatter_body, post_frontmatter_body)` where `frontmatter_body`
/// is `""` when no fence is present.
fn split_frontmatter(src: &str) -> (&str, &str) {
    let trimmed = src.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return ("", src);
    }
    let after_open = match trimmed.strip_prefix("---") {
        Some(rest) => rest.trim_start_matches('\r').trim_start_matches('\n'),
        None => return ("", src),
    };
    if let Some(end) = after_open.find("\n---") {
        let front = &after_open[..end];
        let mut body = &after_open[end + 4..]; // skip "\n---"
        body = body.trim_start_matches('\r').trim_start_matches('\n');
        (front, body)
    } else {
        ("", src)
    }
}

/// Pull `name` + `description` from the frontmatter. Accepts both quoted
/// and unquoted values. Other keys are ignored.
fn parse_frontmatter(front: &str) -> (Option<String>, Option<String>) {
    let mut name = None;
    let mut description = None;
    for raw in front.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let value = v.trim().trim_matches('"').trim_matches('\'').to_string();
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "name" => name = Some(value),
            "description" | "desc" => description = Some(value),
            _ => {}
        }
    }
    (name, description)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_frontmatter_extracts_yaml_block() {
        let src = "---\nname: foo\ndescription: bar\n---\n\nbody here\n";
        let (front, body) = split_frontmatter(src);
        assert!(front.contains("name: foo"));
        assert_eq!(body, "body here\n");
    }

    #[test]
    fn split_frontmatter_handles_no_fence() {
        let src = "just a body\n";
        let (front, body) = split_frontmatter(src);
        assert_eq!(front, "");
        assert_eq!(body, src);
    }

    #[test]
    fn parse_frontmatter_strips_quotes() {
        let (name, desc) = parse_frontmatter("name: \"hello\"\ndescription: 'world'\n");
        assert_eq!(name.as_deref(), Some("hello"));
        assert_eq!(desc.as_deref(), Some("world"));
    }

    #[test]
    fn truncate_body_appends_marker_on_overflow() {
        let big = "x".repeat(MAX_SKILL_BODY_BYTES + 10);
        let out = truncate_body(&big);
        assert!(out.ends_with("[truncated]"));
        assert!(out.len() <= MAX_SKILL_BODY_BYTES + 32);
    }

    #[test]
    fn is_valid_id_rejects_traversal() {
        assert!(!is_valid_id("../etc"));
        assert!(!is_valid_id("foo/bar"));
        assert!(!is_valid_id(""));
        assert!(is_valid_id("python-helper"));
        assert!(is_valid_id("ds_v2"));
    }
}
