//! Storage roots for zero.
//!
//! Everything (DB, settings, model cache, OVMS runtime tree, logs, secrets)
//! lives under `~/.zero/`. The location is identical across platforms so
//! the same path shows up in docs, support tickets, and shell snippets.
//!
//! Prior versions used the platform-specific `ProjectDirs` layout
//! (`%LOCALAPPDATA%\zero\zero\data\` on Windows,
//! `~/Library/Application Support/com.zero.app/` on macOS,
//! `~/.local/share/zero/` on Linux). [`migrate_legacy_root_if_needed`] is
//! invoked once at startup so upgrading users keep their data without any
//! manual move.

use anyhow::{Context, Result};
use directories::{BaseDirs, ProjectDirs};
use std::path::{Path, PathBuf};

/// Returns the root data directory for zero, creating it if missing.
///
/// All platforms: `<home>/.zero/`
pub fn root() -> Result<PathBuf> {
    let p = root_uncreated()?;
    std::fs::create_dir_all(&p).with_context(|| format!("creating {p:?}"))?;
    Ok(p)
}

/// Same as [`root`] but without the `create_dir_all` side effect. Used by
/// the migration helper so it can detect a fresh install vs an upgrade
/// before we've materialised the new directory.
fn root_uncreated() -> Result<PathBuf> {
    let home = BaseDirs::new()
        .context("could not determine user home directory")?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".zero"))
}

/// Legacy storage root used before zero pinned everything under `~/.zero`.
/// Returns `None` only if the platform has no notion of a per-user data
/// dir (extremely unlikely; we just skip the migration in that case).
fn legacy_root() -> Option<PathBuf> {
    ProjectDirs::from("com", "zero", "zero").map(|d| d.data_local_dir().to_path_buf())
}

/// One-shot upgrade hook: if a pre-`.zero` layout exists on disk and the
/// new root hasn't been created yet, move the legacy tree across so the
/// user's models, settings, runtimes, and database survive the path
/// change. Idempotent — after a successful move (or if the user already
/// has data in the new location) this is a no-op.
///
/// Errors are returned so the caller can decide whether to surface them,
/// but [`state::AppState::init`] treats failures as non-fatal: a missing
/// migration just means the user starts with an empty profile, not that
/// the app fails to launch.
pub fn migrate_legacy_root_if_needed() -> Result<()> {
    let new = root_uncreated()?;
    let Some(legacy) = legacy_root() else {
        return Ok(());
    };
    if legacy == new || !legacy.exists() {
        return Ok(());
    }
    if new.exists() {
        // User already has data at the new location — never clobber it,
        // even if the legacy tree also still has files. Leave both in
        // place and let the user decide what to keep.
        tracing::info!(
            "skipping legacy migration: {} already exists; legacy data at {} is untouched",
            new.display(),
            legacy.display()
        );
        return Ok(());
    }

    if let Some(parent) = new.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {parent:?}"))?;
    }
    tracing::info!(
        "migrating zero data from legacy {} → {}",
        legacy.display(),
        new.display()
    );

    // Try a fast rename first. On Windows / same-volume this is atomic
    // and instant. If it fails (typically a cross-volume move on
    // Linux/macOS) we fall back to copy-then-remove.
    match std::fs::rename(&legacy, &new) {
        Ok(()) => {}
        Err(e) => {
            tracing::debug!("rename failed ({e}); falling back to copy-recursive");
            copy_dir_recursive(&legacy, &new)
                .with_context(|| format!("copy {} -> {}", legacy.display(), new.display()))?;
            if let Err(e) = std::fs::remove_dir_all(&legacy) {
                tracing::warn!(
                    "copied legacy data to {} but failed to clean up {}: {e}",
                    new.display(),
                    legacy.display()
                );
            }
        }
    }

    // Best-effort sweep of now-empty legacy parents (e.g.
    // `%LOCALAPPDATA%\zero\zero\` once `data\` is gone). `remove_dir`
    // only succeeds when the directory is empty, so this can't delete
    // anything else the user cares about.
    let mut cursor = legacy.parent().map(Path::to_path_buf);
    for _ in 0..3 {
        let Some(dir) = cursor else { break };
        if std::fs::remove_dir(&dir).is_err() {
            break;
        }
        cursor = dir.parent().map(Path::to_path_buf);
    }

    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ft.is_file() {
            std::fs::copy(&from, &to)?;
        } else {
            // Skip symlinks / other oddities rather than guessing.
            return Err(std::io::Error::other(format!(
                "unsupported file type at {}",
                from.display()
            )));
        }
    }
    Ok(())
}

pub fn db_file() -> Result<PathBuf> {
    Ok(root()?.join("zero.db"))
}

pub fn system_cache() -> Result<PathBuf> {
    Ok(root()?.join("system.json"))
}

pub fn recommended_models_cache() -> Result<PathBuf> {
    Ok(root()?.join("recommended_models.json"))
}

pub fn settings_file() -> Result<PathBuf> {
    Ok(root()?.join("settings.json"))
}

pub fn runtimes_dir() -> Result<PathBuf> {
    let p = root()?.join("runtimes");
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

pub fn ovms_dir() -> Result<PathBuf> {
    let p = runtimes_dir()?.join("ovms");
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

/// Working directory for the spawned `ovms.exe`. Same path as the install
/// dir — the generated `config.json` and `graph.pbtxt` graphs live next
/// to the binary so the layout matches what users expect from a single,
/// self-contained `runtimes/ovms/` folder.
///
/// Older zero versions kept this under a separate `state/` subdir; the
/// new install pipeline (see [`crate::ovms::install`]) wipes the install
/// root on every install so there's no migration step — the next install
/// produces the new flat layout automatically.
pub fn ovms_state_dir() -> Result<PathBuf> {
    ovms_dir()
}

pub fn models_dir() -> Result<PathBuf> {
    let p = root()?.join("models");
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

pub fn logs_dir() -> Result<PathBuf> {
    let p = root()?.join("logs");
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

/// Per-conversation attachments live under `<root>/attachments/<conv_id>/`.
/// Created lazily by [`crate::attachments::save`]. Kept under the same root
/// as everything else so backups / clean-up are a single `~/.zero/` move.
pub fn attachments_dir() -> Result<PathBuf> {
    let p = root()?.join("attachments");
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

/// User-authored skills live under `<root>/skills/<skill_id>/SKILL.md`.
/// Mirrors the layout used by Anthropic-style agent skills so packs can be
/// dropped in by hand and picked up on next list.
pub fn skills_dir() -> Result<PathBuf> {
    let p = root()?.join("skills");
    std::fs::create_dir_all(&p)?;
    Ok(p)
}
