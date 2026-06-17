//! OVMS install pipeline: fetch release → download → extract → record in DB.
//!
//! Emits `ovms://install-progress` events with a `stage` so the UI can show
//! a granular progress bar (fetch → download → extract → verify → done).

use crate::db;
use crate::db::runtimes::RuntimeVersion;
use crate::events;
use crate::ovms::github::{self, Asset};
use crate::paths;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures_util::StreamExt;
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::fs;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStage {
    FetchRelease,
    Download,
    Extract,
    Verify,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallProgress {
    pub stage: InstallStage,
    pub message: String,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub percent: f32,
}

fn emit(app: &AppHandle, p: InstallProgress) {
    let _ = app.emit(events::OVMS_INSTALL_PROGRESS, p);
}

/// Fetches the latest release, downloads the python-free Windows zip,
/// wipes any prior install at `runtimes/ovms/` and re-extracts the
/// archive directly into that flat directory (stripping the leading
/// `ovms/` folder the upstream archive ships with), records the install
/// in `runtime_versions`, and returns the row.
pub async fn install_latest(
    app: &AppHandle,
    http: &reqwest::Client,
    db: &SqlitePool,
) -> Result<RuntimeVersion> {
    // ─── 1. Resolve latest release ──────────────────────────────
    emit(
        app,
        InstallProgress {
            stage: InstallStage::FetchRelease,
            message: "fetching latest release from GitHub…".into(),
            bytes_done: 0,
            bytes_total: None,
            percent: 0.0,
        },
    );
    let release = github::latest_release(http)
        .await
        .context("fetch latest release")?;

    let asset = release
        .find_asset(github::match_windows_python_off)
        .ok_or_else(|| {
            anyhow!(
                "no matching asset in release {} (expected *windows*python_off*.zip)",
                release.tag_name
            )
        })?
        .clone();

    // ─── 2. Stream-download the zip into a scratch file outside the
    //        install root, so wiping the install dir later doesn't nuke
    //        the archive we're about to extract from.
    let install_dir = paths::ovms_dir()?;
    let scratch_dir = std::env::temp_dir().join("zero-ovms-install");
    fs::create_dir_all(&scratch_dir).await?;
    let zip_path = scratch_dir.join(&asset.name);

    emit(
        app,
        InstallProgress {
            stage: InstallStage::Download,
            message: format!("downloading {} ({} MB)", asset.name, asset.size / 1024 / 1024),
            bytes_done: 0,
            bytes_total: Some(asset.size),
            percent: 0.0,
        },
    );
    download_with_progress(app, http, &asset, &zip_path).await?;

    // ─── 3. Extract into the (wiped) install root ───────────────────
    emit(
        app,
        InstallProgress {
            stage: InstallStage::Extract,
            message: "extracting archive…".into(),
            bytes_done: asset.size,
            bytes_total: Some(asset.size),
            percent: 0.95,
        },
    );
    let extract_target = install_dir.clone();
    let zip_for_extract = zip_path.clone();
    tokio::task::spawn_blocking(move || {
        wipe_install_dir(&extract_target)?;
        extract_zip(&zip_for_extract, &extract_target)
    })
    .await
    .map_err(|e| anyhow!("extract task panicked: {e}"))??;

    // We don't need the archive after extraction.
    let _ = fs::remove_file(&zip_path).await;
    let _ = fs::remove_dir(&scratch_dir).await;

    // ─── 4. Verify ovms.exe is present ─────────────────────────────
    emit(
        app,
        InstallProgress {
            stage: InstallStage::Verify,
            message: "locating ovms executable…".into(),
            bytes_done: asset.size,
            bytes_total: Some(asset.size),
            percent: 0.98,
        },
    );
    let exe = find_executable(&install_dir)
        .ok_or_else(|| anyhow!("ovms executable not found under {:?}", install_dir))?;

    // ─── 5. Record in DB ─────────────────────────────────────
    let rv = RuntimeVersion {
        name: "ovms".into(),
        version: release.tag_name.clone(),
        install_dir: install_dir.to_string_lossy().into_owned(),
        executable: exe.to_string_lossy().into_owned(),
        installed_at: Utc::now().to_rfc3339(),
        source_url: Some(asset.browser_download_url.clone()),
        metadata: Some(serde_json::json!({
            "asset": asset.name,
            "asset_size": asset.size,
            "release_name": release.name,
            "published_at": release.published_at,
        })),
    };
    db::runtimes::upsert(db, &rv).await?;

    emit(
        app,
        InstallProgress {
            stage: InstallStage::Done,
            message: format!("installed ovms {}", rv.version),
            bytes_done: asset.size,
            bytes_total: Some(asset.size),
            percent: 1.0,
        },
    );
    Ok(rv)
}

async fn download_with_progress(
    app: &AppHandle,
    http: &reqwest::Client,
    asset: &Asset,
    dest: &Path,
) -> Result<()> {
    let resp = http
        .get(&asset.browser_download_url)
        .send()
        .await?
        .error_for_status()?;
    let total = resp.content_length().unwrap_or(asset.size);

    let mut file = fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    let mut bytes_done: u64 = 0;
    let mut last_emit = Instant::now();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        file.write_all(&bytes).await?;
        bytes_done += bytes.len() as u64;

        // Throttle to ~10 events/sec so we don't flood the webview.
        if last_emit.elapsed().as_millis() >= 100 {
            last_emit = Instant::now();
            let percent = if total > 0 {
                (bytes_done as f32 / total as f32).min(0.94)
            } else {
                0.0
            };
            emit(
                app,
                InstallProgress {
                    stage: InstallStage::Download,
                    message: format!(
                        "downloading… {} / {} MB",
                        bytes_done / 1024 / 1024,
                        total / 1024 / 1024
                    ),
                    bytes_done,
                    bytes_total: Some(total),
                    percent,
                },
            );
        }
    }
    file.flush().await?;
    Ok(())
}

/// Synchronous zip extraction. Called inside `spawn_blocking`.
///
/// Detects whether every entry in the archive sits under a single shared
/// top-level directory (the upstream OVMS Windows zip ships everything
/// under `ovms/`) and strips that prefix on the way out, so files land
/// flat in `out_dir`. If there's no common prefix, entries are written
/// verbatim.
fn extract_zip(zip_path: &Path, out_dir: &Path) -> Result<()> {
    let f = std::fs::File::open(zip_path)
        .with_context(|| format!("open {zip_path:?}"))?;
    let mut archive = zip::ZipArchive::new(f).context("read zip central directory")?;

    let strip_prefix = detect_common_prefix(&archive);
    if let Some(p) = &strip_prefix {
        tracing::debug!("stripping common archive prefix `{}/` during extract", p.display());
    }

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            // Skip absolute paths / parent-escapes — defense against zip-slip.
            tracing::warn!("skipping suspicious zip entry: {}", entry.name());
            continue;
        };
        let stripped: PathBuf = match &strip_prefix {
            Some(prefix) => match rel.strip_prefix(prefix) {
                Ok(rest) if rest.as_os_str().is_empty() => continue, // the prefix dir itself
                Ok(rest) => rest.to_path_buf(),
                Err(_) => rel.to_path_buf(), // shouldn't happen, but be safe
            },
            None => rel.to_path_buf(),
        };
        if stripped.as_os_str().is_empty() {
            continue;
        }
        let target = out_dir.join(&stripped);

        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)
            .with_context(|| format!("create {target:?}"))?;
        std::io::copy(&mut entry, &mut out)?;

        // Preserve unix exec bit (no-op on Windows).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
            }
        }
    }
    Ok(())
}

/// Wipe the install directory before re-extracting so stale files from a
/// previous install (including the old `v<version>/` subdirs from the
/// pre-flat layout) can't survive an update. The directory itself is
/// preserved — we only remove its contents.
fn wipe_install_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {dir:?}"))?;
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {dir:?}"))? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        let res = if ft.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(e) = res {
            tracing::warn!("failed to remove {} during install wipe: {e}", path.display());
        }
    }
    Ok(())
}

/// Returns the single shared top-level component if every file entry in
/// the archive lives under the same root directory, otherwise `None`.
/// Used by [`extract_zip`] to flatten archives that wrap their contents
/// in a single folder (the upstream OVMS Windows zip wraps everything in
/// `ovms/`).
fn detect_common_prefix(archive: &zip::ZipArchive<std::fs::File>) -> Option<PathBuf> {
    let mut prefix: Option<PathBuf> = None;
    for name in archive.file_names() {
        // Skip explicit directory entries — they're recreated by parent
        // walks during extraction and don't constrain the prefix.
        if name.ends_with('/') {
            continue;
        }
        let rel = Path::new(name);
        let mut comps = rel.components();
        let Some(first) = comps.next() else {
            return None;
        };
        // A file at the archive root means there's no shared prefix.
        if comps.next().is_none() {
            return None;
        }
        let first_buf: PathBuf = first.as_os_str().into();
        match &prefix {
            None => prefix = Some(first_buf),
            Some(p) if p == &first_buf => {}
            Some(_) => return None,
        }
    }
    prefix
}

/// Walk `root` looking for `ovms.exe` (Windows) or `ovms` (other). We try the
/// most likely locations first to avoid a full traversal on big trees.
pub fn find_executable(root: &Path) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) { "ovms.exe" } else { "ovms" };

    // Common shapes: <root>/ovms.exe, <root>/ovms/ovms.exe, <root>/<dist>/ovms.exe
    let direct = root.join(exe_name);
    if direct.exists() {
        return Some(direct);
    }
    let nested = root.join("ovms").join(exe_name);
    if nested.exists() {
        return Some(nested);
    }
    // Fallback: shallow walk (max 3 deep).
    walk_for(root, exe_name, 3)
}

fn walk_for(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|s| s.to_str()) == Some(name) {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = walk_for(&path, name, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    use zip::write::SimpleFileOptions;

    static CTR: AtomicU64 = AtomicU64::new(0);
    fn tmpdir(tag: &str) -> PathBuf {
        let i = CTR.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "zero-install-test-{}-{}-{}",
            tag,
            std::process::id(),
            i
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// Build a zip with the given (path, body) entries. Paths use forward
    /// slashes; entries ending in `/` are written as directory markers.
    fn build_zip(zip_path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(zip_path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts = SimpleFileOptions::default();
        for (name, body) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, opts).unwrap();
            } else {
                w.start_file(*name, opts).unwrap();
                w.write_all(body).unwrap();
            }
        }
        w.finish().unwrap();
    }

    #[test]
    fn strips_single_top_level_prefix() {
        // Mirrors the upstream OVMS layout: everything under `ovms/`.
        let d = tmpdir("strip");
        let zip_path = d.join("src.zip");
        build_zip(
            &zip_path,
            &[
                ("ovms/", b""),
                ("ovms/ovms.exe", b"binary"),
                ("ovms/lib/openvino.dll", b"dll"),
                ("ovms/setupvars.bat", b"@echo off"),
            ],
        );
        let out = d.join("out");
        extract_zip(&zip_path, &out).unwrap();
        assert!(out.join("ovms.exe").is_file(), "ovms.exe should land flat");
        assert!(out.join("lib/openvino.dll").is_file());
        assert!(out.join("setupvars.bat").is_file());
        assert!(!out.join("ovms").exists(), "prefix dir should be stripped");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn keeps_layout_when_no_common_prefix() {
        // Two siblings at the archive root → no prefix to strip.
        let d = tmpdir("keep");
        let zip_path = d.join("src.zip");
        build_zip(
            &zip_path,
            &[("a/file.txt", b"x"), ("b/file.txt", b"y")],
        );
        let out = d.join("out");
        extract_zip(&zip_path, &out).unwrap();
        assert!(out.join("a/file.txt").is_file());
        assert!(out.join("b/file.txt").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn wipe_install_dir_clears_contents_but_keeps_root() {
        let d = tmpdir("wipe");
        std::fs::create_dir_all(d.join("v2026.2/ovms")).unwrap();
        std::fs::write(d.join("v2026.2/ovms/ovms.exe"), b"stale").unwrap();
        std::fs::write(d.join("loose.txt"), b"stale").unwrap();
        wipe_install_dir(&d).unwrap();
        assert!(d.is_dir(), "root preserved");
        let remaining: Vec<_> = std::fs::read_dir(&d).unwrap().collect();
        assert!(remaining.is_empty(), "contents wiped");
        let _ = std::fs::remove_dir_all(&d);
    }
}
