//! Model download / update / delete pipeline.
//!
//! `install_or_update` is the single entry point: if there is no local copy,
//! we download every relevant file from the latest revision; if there is, we
//! diff our `.zero_manifest.json` against the upstream commit SHA + sibling
//! list and re-download only changed files.
//!
//! Progress is reported via the `models://download-progress` Tauri event in
//! the shape the frontend already expects (see `src/stores/models.ts`).

use super::api;
use super::jobs::CancelHandle;
use super::params::{self, ModelDeployParams};
use super::{DownloadProgress, DownloadState, LocalModel};
use crate::events;
use crate::paths;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::fs;
use tokio::io::AsyncWriteExt;

const MANIFEST_FILE: &str = ".zero_manifest.json";

/// Sentinel error returned through the normal `Result` channel when a
/// download was aborted by [`super::DownloadJobs::cancel`]. The command
/// wrapper turns this into a terminal `cancelled` progress event instead of
/// the usual `error` event so the UI can render it differently.
#[derive(Debug, thiserror::Error)]
#[error("download cancelled")]
pub struct Cancelled;

/// Versioned on-disk manifest. Bump `version` whenever the schema changes so
/// older clients fail loudly instead of silently misinterpreting it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    hf_id: String,
    revision: String,
    files: HashMap<String, FileEntry>,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileEntry {
    size: u64,
    /// Hex sha256, when known. Set on download for LFS-tracked files
    /// (regular weights), and re-checked against a streaming hash so a
    /// corrupted byte during transit fails the install instead of silently
    /// persisting. `None` for small non-LFS files — HF doesn't publish a
    /// sha256 for those and computing one would cost an extra read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

impl Manifest {
    fn new(hf_id: &str, revision: &str) -> Self {
        Self {
            version: 1,
            hf_id: hf_id.to_string(),
            revision: revision.to_string(),
            files: HashMap::new(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }
}

/// Translate `org/repo` → `<models_root>/org/repo`. Returns an error if `id`
/// looks dangerous (`..`, absolute path, etc.) so we can never escape the
/// models directory.
pub fn model_dir(id: &str) -> Result<PathBuf> {
    let root = paths::models_dir()?;
    let safe = sanitize_repo_id(id)?;
    Ok(root.join(safe))
}

fn sanitize_repo_id(id: &str) -> Result<PathBuf> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("empty model id"));
    }
    let mut out = PathBuf::new();
    for seg in trimmed.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains('\\') {
            return Err(anyhow!("invalid model id segment: {seg:?}"));
        }
        out.push(seg);
    }
    Ok(out)
}

/// Returns `true` when the file is worth pulling for an OpenVINO/OVMS setup.
///
/// Default policy: keep everything except duplicate weight formats that OVMS
/// can't use (pytorch / safetensors / gguf / etc.). The user can override
/// this later via a "force all files" toggle when we expose it.
fn should_include(file: &str) -> bool {
    let lower = file.to_lowercase();

    // Skip per-format weight artefacts we don't need.
    const SKIP_EXT: &[&str] = &[
        ".safetensors",
        ".pt",
        ".pth",
        ".gguf",
        ".ggml",
        ".h5",
        ".tflite",
        ".ckpt",
        ".msgpack",
        ".ot",
        ".bin.index.json",
    ];
    for ext in SKIP_EXT {
        if lower.ends_with(ext) {
            return false;
        }
    }
    if lower.starts_with("pytorch_model") || lower.contains("/pytorch_model") {
        return false;
    }
    if lower.starts_with("flax_model") || lower.contains("/flax_model") {
        return false;
    }
    if lower.starts_with("tf_model") || lower.contains("/tf_model") {
        return false;
    }

    // Always skip dotfiles / git plumbing.
    if lower.starts_with(".git") || lower.contains("/.git") {
        return false;
    }
    if lower.ends_with(".gitattributes") || lower.ends_with(".gitignore") {
        return false;
    }

    true
}

fn emit(app: &AppHandle, p: &DownloadProgress) {
    let _ = app.emit(events::MODELS_DOWNLOAD_PROGRESS, p);
}

/// Idempotent install: if the local revision already matches upstream, this
/// is a fast no-op (manifest read + DB upsert). Otherwise it downloads the
/// changed/missing files and rewrites the manifest.
///
/// `cancel` is observed at file boundaries and inside the chunk stream. When
/// it fires the function returns [`Cancelled`] without touching the DB —
/// any `*.partial` files are left on disk so a follow-up `start` can resume.
///
/// `params_override` lets the caller persist a fresh [`ModelDeployParams`]
/// sidecar at install time (e.g. when the user picked a non-default task
/// or tweaked text-generation knobs in the UI). When `None`, an existing
/// sidecar is preserved and the default text-generation params are used
/// for fresh installs.
pub async fn install_or_update(
    app: &AppHandle,
    http: &reqwest::Client,
    db: &SqlitePool,
    id: &str,
    cancel: CancelHandle,
    params_override: Option<ModelDeployParams>,
) -> Result<LocalModel> {
    let dir = model_dir(id)?;
    fs::create_dir_all(&dir).await?;

    // ─── 1. Discover upstream state ──────────────────────────────────────
    let mut progress = DownloadProgress {
        model_id: id.to_string(),
        bytes_done: 0,
        bytes_total: None,
        files_done: 0,
        files_total: 0,
        state: DownloadState::Pending,
        error: None,
    };
    emit(app, &progress);

    let info = api::model_info(http, id, None).await?;
    let revision = info
        .sha
        .clone()
        .ok_or_else(|| anyhow!("hf: model_info {id} returned no sha"))?;

    // Filter siblings to what we actually want, then plan sizes.
    let wanted: Vec<_> = info
        .siblings
        .iter()
        .filter(|s| should_include(&s.rfilename))
        .cloned()
        .collect();
    if wanted.is_empty() {
        return Err(anyhow!(
            "no installable files in {id} after filtering (looking for OpenVINO IR + tokenizer files)"
        ));
    }

    // Resolve sizes + expected hashes (HEAD for any sibling missing a size
    // on the listing). Each plan entry carries the bytes-to-expect and, if
    // upstream advertised one, the sha256 to verify against post-download.
    let plan = resolve_plan(http, id, &revision, &wanted).await?;
    let bytes_total: u64 = plan.iter().map(|p| p.size.unwrap_or(0)).sum();

    let existing = read_manifest(&dir).await;
    let same_rev = existing.as_ref().is_some_and(|m| m.revision == revision);

    progress.files_total = plan.len() as u64;
    progress.bytes_total = if bytes_total > 0 {
        Some(bytes_total)
    } else {
        None
    };
    progress.state = DownloadState::Downloading;
    emit(app, &progress);

    // ─── 2. Pull files (skipping anything already present & matching) ────
    let mut manifest = existing
        .clone()
        .unwrap_or_else(|| Manifest::new(id, &revision));
    manifest.revision = revision.clone();
    manifest.hf_id = id.to_string();

    for entry in &plan {
        // Check cancellation at every file boundary too — between files no
        // chunk-level select is firing.
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }

        let target = dir.join(&entry.file);
        let needs_pull = !same_rev
            || match existing.as_ref().and_then(|m| m.files.get(&entry.file)) {
                None => true,
                Some(prev) => !matches_on_disk(&target, entry.size, prev).await,
            };

        if !needs_pull {
            progress.files_done += 1;
            if let Some(sz) = entry.size {
                progress.bytes_done = progress.bytes_done.saturating_add(sz);
            }
            emit(app, &progress);
            // Make sure the manifest knows about it (fresh installs). Preserve
            // any sha we previously recorded so we keep the integrity bit.
            let prev_sha = existing
                .as_ref()
                .and_then(|m| m.files.get(&entry.file))
                .and_then(|f| f.sha256.clone());
            manifest.files.insert(
                entry.file.clone(),
                FileEntry {
                    size: entry.size.unwrap_or(0),
                    sha256: entry.sha256.clone().or(prev_sha),
                },
            );
            continue;
        }

        let outcome = download_file(
            app,
            http,
            FileJob {
                id,
                revision: &revision,
                file: &entry.file,
                dest: &target,
                size_hint: entry.size,
                expected_sha256: entry.sha256.as_deref(),
            },
            &mut progress,
            &cancel,
        )
        .await
        .with_context(|| format!("download {}", entry.file))?;

        manifest.files.insert(
            entry.file.clone(),
            FileEntry {
                size: outcome.bytes_written,
                sha256: Some(outcome.sha256),
            },
        );
        progress.files_done += 1;
        emit(app, &progress);
    }

    // ─── 3. Persist manifest + DB row ───────────────────────────────────
    progress.state = DownloadState::Verifying;
    emit(app, &progress);
    manifest.updated_at = Utc::now().to_rfc3339();
    write_manifest(&dir, &manifest).await?;

    // ─── 3a. Generate the OVMS graph.pbtxt next to the weights ───────
    //
    // Mirrors what `export_model.py` does in the upstream OVMS demos: a
    // text-generation HF model is only directly serveable once it has a
    // `graph.pbtxt` describing the LLM calculator pipeline. We render it
    // here using the current `settings.ovms.device` so the model is
    // immediately serveable, and `ovms::config::prepare` will refresh it
    // again at start time if the device has since changed.
    //
    // The per-task parameter set is sourced in priority order:
    //   1. explicit `params_override` (UI flow where the user clicked
    //      "configure…" and picked task / parsers themselves).
    //   2. existing sidecar on disk (re-install / update — never silently
    //      stomp on a sidecar the user already tuned).
    //   3. smart detection (fresh install, user clicked plain "download")
    //      — task, VLM pipeline_type, and tool/reasoning parsers are
    //      inferred from HF metadata + `config.json`. See `hf::detect`.
    //   4. defaults (detection produced nothing usable).
    //
    // Failures are non-fatal: the user can still re-trigger generation by
    // starting OVMS against this model. We log loudly so it surfaces.
    let sidecar_exists = params::sidecar_path(&dir).exists();
    let resolved_params = if let Some(p) = params_override.clone() {
        if let Err(e) = params::write(&dir, &p) {
            tracing::warn!("failed to persist .zero_model.json for {id}: {e:#}");
        }
        p
    } else if sidecar_exists {
        params::read_or_default(&dir)
    } else {
        // No user override + no prior sidecar = fresh install. Use
        // `config.json` we already have access to via the HF API to
        // infer the right task + parsers, persist the result so the
        // params modal opens pre-filled, and feed it into the graph.
        let cfg = match &revision {
            r if !r.is_empty() => super::detect::fetch_config_json(http, id, r).await,
            _ => None,
        };
        let detected = super::detect::suggest_from_inputs(&super::DetectionInputs {
            id,
            pipeline_tag: info.pipeline_tag.as_deref(),
            tags: &info.tags,
            siblings: &info.siblings,
            config: cfg,
        });
        tracing::info!(
            "hf: auto-detected task={} for {id}",
            detected.task.kind_str()
        );
        if let Err(e) = params::write(&dir, &detected) {
            tracing::warn!("failed to persist detected .zero_model.json for {id}: {e:#}");
        }
        detected
    };
    let device = crate::settings::Settings::load()
        .await
        .map(|s| s.ovms.device)
        .unwrap_or_else(|_| "GPU".to_string());
    let dir_for_graph = dir.clone();
    let params_for_graph = resolved_params.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        crate::ovms::config::ensure_in_model_graph(&dir_for_graph, &device, &params_for_graph)
    })
    .await
    .map_err(|e| anyhow!("graph render task panicked: {e}"))
    .and_then(|r| r)
    {
        tracing::warn!(
            "failed to write graph.pbtxt for {id}: {e:#} — model files are on disk, but OVMS will fail to load until a graph is present"
        );
    }

    let total_bytes: u64 = manifest.files.values().map(|f| f.size).sum();
    let verified_count: u64 = manifest
        .files
        .values()
        .filter(|f| f.sha256.as_ref().is_some_and(|s| !s.is_empty()))
        .count() as u64;
    let now = Utc::now().to_rfc3339();
    // Capture the upstream HF `pipeline_tag` so the Models page can
    // render a per-model category badge without re-hitting the network.
    // Empty strings are normalised to NULL so the read path's backfill
    // (sidecar → derived tag) kicks in instead of displaying nothing.
    let pipeline_tag = info
        .pipeline_tag
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    sqlx::query(
        "INSERT INTO local_models (id, hf_id, path, bytes, added_at, revision, files, verified_files, pipeline_tag)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            hf_id          = excluded.hf_id,
            path           = excluded.path,
            bytes          = excluded.bytes,
            added_at       = excluded.added_at,
            revision       = excluded.revision,
            files          = excluded.files,
            verified_files = excluded.verified_files,
            pipeline_tag   = COALESCE(excluded.pipeline_tag, local_models.pipeline_tag)",
    )
    .bind(id)
    .bind(id)
    .bind(dir.to_string_lossy().to_string())
    .bind(total_bytes as i64)
    .bind(&now)
    .bind(&revision)
    .bind(manifest.files.len() as i64)
    .bind(verified_count as i64)
    .bind(pipeline_tag.as_deref())
    .execute(db)
    .await?;

    progress.state = DownloadState::Done;
    progress.bytes_done = total_bytes;
    progress.bytes_total = Some(total_bytes);
    progress.files_done = manifest.files.len() as u64;
    emit(app, &progress);

    Ok(LocalModel {
        id: id.to_string(),
        hf_id: Some(id.to_string()),
        path: dir.to_string_lossy().into_owned(),
        bytes: total_bytes,
        added_at: now,
        revision: Some(revision),
        files: Some(manifest.files.len() as u64),
        verified_files: Some(verified_count),
        pipeline_tag,
    })
}

/// Per-file row in the install plan: the relative filename, the size we
/// expect to receive (when known), and the sha256 the upstream LFS index
/// advertised (when known).
#[derive(Debug, Clone)]
struct PlanEntry {
    file: String,
    size: Option<u64>,
    sha256: Option<String>,
}

/// Build the per-file plan: prefer the sizes/hashes the API already gave us,
/// fall back to a HEAD for any sibling missing a size. Returned in the
/// original sibling order so the UI's file-count progress matches the order
/// of events the user sees.
async fn resolve_plan(
    http: &reqwest::Client,
    id: &str,
    revision: &str,
    siblings: &[api::Sibling],
) -> Result<Vec<PlanEntry>> {
    let mut out = Vec::with_capacity(siblings.len());
    for s in siblings {
        // LFS metadata, when present, beats the top-level `size` (which on
        // LFS-tracked files refers to the pointer file rather than the real
        // blob).
        let lfs_size = s.lfs.as_ref().and_then(|l| l.size);
        let lfs_sha = s.lfs.as_ref().and_then(|l| l.sha256.clone());
        let size = lfs_size.or(s.size);

        let size = if let Some(sz) = size {
            Some(sz)
        } else {
            // Don't abort the whole install just because one HEAD failed
            // — we'll still download it, just without contributing to
            // bytes_total.
            api::head_size(http, id, revision, &s.rfilename)
                .await
                .ok()
                .flatten()
        };
        out.push(PlanEntry {
            file: s.rfilename.clone(),
            size,
            sha256: lfs_sha,
        });
    }
    Ok(out)
}

/// Per-file inputs for `download_file`. Bundled so the helper's signature
/// stays inside clippy's `too_many_arguments` limit and so the call site
/// reads as a flat field list at the point of invocation.
struct FileJob<'a> {
    id: &'a str,
    revision: &'a str,
    file: &'a str,
    dest: &'a Path,
    size_hint: Option<u64>,
    /// Hex-encoded sha256 we expect the bytes to hash to, when upstream
    /// publishes one (LFS files). Verified after the stream completes; a
    /// mismatch aborts the install with a clear error and leaves the
    /// `.partial` removed so a retry starts clean.
    expected_sha256: Option<&'a str>,
}

/// Outcome of a successful `download_file`. We carry both the byte count and
/// the streaming-computed sha256 so the caller can persist them in the
/// manifest without re-reading the file from disk.
struct DownloadOutcome {
    bytes_written: u64,
    sha256: String,
}

/// Stream a single file to disk, writing to `<file>.partial` then renaming on
/// success. Emits throttled progress events for files larger than a tick.
///
/// `cancel` is observed in a `tokio::select!` against the chunk stream so an
/// in-flight large file aborts within milliseconds of the user clicking
/// cancel — we don't need to wait for the next chunk to arrive. If an
/// `expected_sha256` is provided, the function aborts with a clear error
/// when the computed hash doesn't match — the `.partial` is removed so the
/// next retry starts from a known-empty state.
async fn download_file(
    app: &AppHandle,
    http: &reqwest::Client,
    job: FileJob<'_>,
    progress: &mut DownloadProgress,
    cancel: &CancelHandle,
) -> Result<DownloadOutcome> {
    use sha2::{Digest, Sha256};

    let FileJob {
        id,
        revision,
        file,
        dest,
        size_hint,
        expected_sha256,
    } = job;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).await?;
    }
    let tmp = dest.with_extension(format!(
        "{}partial",
        dest.extension()
            .and_then(|s| s.to_str())
            .map(|s| format!("{s}."))
            .unwrap_or_default()
    ));

    let url = api::resolve_url(id, revision, file);
    let resp = http
        .get(&url)
        .headers(api::auth_headers())
        .send()
        .await?
        .error_for_status()?;
    let total = resp.content_length().or(size_hint);

    let mut sink = fs::File::create(&tmp).await?;
    let mut stream = resp.bytes_stream();
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    let mut last_emit = Instant::now();
    let baseline_bytes_done = progress.bytes_done;

    loop {
        tokio::select! {
            biased;
            _ = cancel.wait() => {
                // Drop the in-progress partial file so a half-byte chunk
                // doesn't masquerade as a valid `.partial` on resume.
                drop(sink);
                let _ = fs::remove_file(&tmp).await;
                return Err(Cancelled.into());
            }
            maybe = stream.next() => {
                let Some(chunk) = maybe else { break; };
                let bytes = chunk?;
                sink.write_all(&bytes).await?;
                hasher.update(&bytes);
                written += bytes.len() as u64;

                // Update aggregate counter relative to the file's own start so we
                // don't double-count if the hinted size was wrong.
                progress.bytes_done = baseline_bytes_done.saturating_add(written);

                if last_emit.elapsed().as_millis() >= 100 {
                    last_emit = Instant::now();
                    emit(app, progress);
                }
                // Update bytes_total mid-stream if we discovered the real size.
                if progress.bytes_total.is_none() {
                    if let Some(t) = total {
                        progress.bytes_total = Some(progress.bytes_done.max(t));
                    }
                }
            }
        }
    }
    sink.flush().await?;
    drop(sink);

    let actual_sha = hex_lower(&hasher.finalize());
    if let Some(expected) = expected_sha256 {
        // Hex compare is case-insensitive on HF's side; normalise both.
        if !expected.eq_ignore_ascii_case(&actual_sha) {
            let _ = fs::remove_file(&tmp).await;
            return Err(anyhow!(
                "sha256 mismatch for {file}: expected {expected}, got {actual_sha}"
            ));
        }
    }

    fs::rename(&tmp, dest)
        .await
        .with_context(|| format!("rename {tmp:?} → {dest:?}"))?;
    Ok(DownloadOutcome {
        bytes_written: written,
        sha256: actual_sha,
    })
}

/// Render a digest as lower-case hex without dragging the `hex` crate in.
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

async fn matches_on_disk(path: &Path, size_hint: Option<u64>, prev: &FileEntry) -> bool {
    // Cheap path: trust the size + the recorded sha256 from the manifest.
    // We do *not* re-hash a possibly-multi-GB file on every install — the
    // manifest is co-located and tamper-evident enough for our threat
    // model. A user who wants paranoid re-verification can delete the
    // manifest to force a re-pull.
    match fs::metadata(path).await {
        Ok(m) if m.is_file() => {
            let on_disk = m.len();
            match size_hint {
                Some(sz) if on_disk != sz => false,
                None if on_disk == 0 => false,
                _ => prev.size == 0 || prev.size == on_disk,
            }
        }
        _ => false,
    }
}

async fn read_manifest(dir: &Path) -> Option<Manifest> {
    let bytes = fs::read(dir.join(MANIFEST_FILE)).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn write_manifest(dir: &Path, m: &Manifest) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(m)?;
    fs::write(dir.join(MANIFEST_FILE), bytes).await?;
    Ok(())
}

/// Delete the on-disk model directory + the matching DB row.
///
/// Bails *before* touching the DB if the on-disk removal fails so the
/// catalogue never drifts out of sync with what's actually on disk —
/// the most common offender on Windows is a model that's still
/// memory-mapped by OVMS, which surfaces as `os error 32` (sharing
/// violation). Surfacing the error tells the UI to keep the card and
/// prompts the user to unload first.
pub async fn delete(db: &SqlitePool, id: &str) -> Result<()> {
    // Look up the row first so we know the real path (which may differ from
    // what `model_dir` would compute if the layout ever changes).
    let row = sqlx::query("SELECT path FROM local_models WHERE id = ? OR hf_id = ?")
        .bind(id)
        .bind(id)
        .fetch_optional(db)
        .await?;

    // Resolve the directory to attempt to remove. If the row is missing
    // we fall back to `model_dir(id)` so legacy entries with mismatched
    // ids still get cleaned up; if neither resolves, there's nothing to
    // delete on disk and we proceed straight to the DB scrub.
    let target_dir = match row {
        Some(ref r) => Some(PathBuf::from(r.get::<String, _>("path"))),
        None => model_dir(id).ok(),
    };

    if let Some(p) = target_dir.as_ref() {
        if p.exists() {
            if let Err(e) = fs::remove_dir_all(p).await {
                // Surface a human-friendly hint for the common
                // "weights still mapped by OVMS" case so the user knows
                // exactly what to do, instead of having to decode
                // `os error 32`. The original error is preserved as
                // context for the log.
                tracing::warn!("rm -rf {p:?} failed: {e}");
                return Err(anyhow!(
                    "failed to remove `{}`: {e}. If the model is currently \
                     loaded into OVMS, unload it from Settings → OVMS first.",
                    p.display(),
                ));
            }
            // Best-effort: prune the now-empty `<org>/` parent so the
            // models tree doesn't accumulate dead dirs after deletes.
            // Failures (ENOTEMPTY, perms) are ignored — the user-visible
            // contract is just that the model dir is gone.
            if let Some(parent) = p.parent() {
                let _ = fs::remove_dir(parent).await;
            }
        }
    }

    sqlx::query("DELETE FROM local_models WHERE id = ? OR hf_id = ?")
        .bind(id)
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// One-shot startup pass: for every `local_models` row with `verified_files`
/// still NULL, read the on-disk `.zero_manifest.json` and update the row
/// with the count of files that carry a recorded sha256. Rows whose
/// manifests are unreadable, missing, or malformed are left untouched (the
/// next install/update for that model will repopulate them).
///
/// Returns the number of rows successfully backfilled. Safe to run on every
/// boot — once a row has a non-NULL value it's skipped, and the work scales
/// with the count of legacy rows, not the total catalogue.
pub async fn backfill_verified(db: &SqlitePool) -> Result<u64> {
    let rows = sqlx::query("SELECT id, path FROM local_models WHERE verified_files IS NULL")
        .fetch_all(db)
        .await
        .context("select rows needing verified_files backfill")?;

    let mut updated: u64 = 0;
    for r in rows {
        let id: String = r.get("id");
        let path: String = r.get("path");
        let dir = PathBuf::from(&path);

        let manifest = match read_manifest(&dir).await {
            Some(m) => m,
            None => {
                tracing::debug!(
                    "backfill_verified: no readable manifest for {id} at {path} — skipping"
                );
                continue;
            }
        };
        let verified: i64 = manifest
            .files
            .values()
            .filter(|f| f.sha256.as_ref().is_some_and(|s| !s.is_empty()))
            .count() as i64;
        match sqlx::query("UPDATE local_models SET verified_files = ? WHERE id = ?")
            .bind(verified)
            .bind(&id)
            .execute(db)
            .await
        {
            Ok(_) => updated += 1,
            Err(e) => tracing::warn!("backfill_verified: update {id} failed: {e}"),
        }
    }
    if updated > 0 {
        tracing::info!("backfill_verified: filled in {updated} local_models row(s)");
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn hex_lower_matches_known_vectors() {
        // "abc" -> ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let mut h = Sha256::new();
        h.update(b"abc");
        assert_eq!(
            hex_lower(&h.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        // Empty input -> the well-known empty-string sha256.
        let empty = Sha256::new().finalize();
        assert_eq!(
            hex_lower(&empty),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hex_lower_is_lowercase_and_full_width() {
        let out = hex_lower(&[0x00, 0x0f, 0xf0, 0xff]);
        assert_eq!(out, "000ff0ff");
    }

    // ─── backfill_verified ────────────────────────────────────────────────

    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::SqlitePool;
    use std::str::FromStr;

    /// Spin up an in-memory SQLite pool with just the `local_models` columns
    /// we exercise here. The real schema lives in `db::mod`; we don't run
    /// the full migrate path because we don't need the rest of the tables.
    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        sqlx::query(
            "CREATE TABLE local_models (
                id              TEXT PRIMARY KEY,
                hf_id           TEXT,
                path            TEXT NOT NULL,
                bytes           INTEGER NOT NULL,
                added_at        TEXT NOT NULL,
                revision        TEXT,
                files           INTEGER,
                verified_files  INTEGER
             )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn insert_row(pool: &SqlitePool, id: &str, path: &str, verified: Option<i64>) {
        sqlx::query(
            "INSERT INTO local_models (id, path, bytes, added_at, verified_files)
             VALUES (?, ?, 0, '2025-01-01T00:00:00Z', ?)",
        )
        .bind(id)
        .bind(path)
        .bind(verified)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn read_verified(pool: &SqlitePool, id: &str) -> Option<i64> {
        sqlx::query("SELECT verified_files FROM local_models WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
            .try_get::<Option<i64>, _>("verified_files")
            .unwrap()
    }

    fn write_manifest_at(dir: &Path, files: &[(&str, u64, Option<&str>)]) {
        std::fs::create_dir_all(dir).unwrap();
        let mut entries: HashMap<String, FileEntry> = HashMap::new();
        for (name, size, sha) in files {
            entries.insert(
                (*name).into(),
                FileEntry {
                    size: *size,
                    sha256: sha.map(|s| s.to_string()),
                },
            );
        }
        let m = Manifest {
            version: 1,
            hf_id: "org/repo".into(),
            revision: "deadbeef".into(),
            files: entries,
            updated_at: "2025-01-01T00:00:00Z".into(),
        };
        std::fs::write(
            dir.join(MANIFEST_FILE),
            serde_json::to_vec_pretty(&m).unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn backfill_counts_only_sha_bearing_entries() {
        let pool = test_pool().await;
        let tmp = tempdir_unique("backfill-counts");
        write_manifest_at(
            &tmp,
            &[
                ("weights.bin", 1000, Some("aa")),
                ("config.json", 50, None),
                ("tokenizer.json", 200, Some("bb")),
                ("empty.txt", 10, Some("")), // empty sha should not count
            ],
        );
        insert_row(&pool, "m1", tmp.to_str().unwrap(), None).await;

        let n = backfill_verified(&pool).await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(read_verified(&pool, "m1").await, Some(2));

        // Second run is a no-op because the row is no longer NULL.
        let n2 = backfill_verified(&pool).await.unwrap();
        assert_eq!(n2, 0);
        cleanup(&tmp);
    }

    #[tokio::test]
    async fn backfill_skips_missing_manifest_and_keeps_null() {
        let pool = test_pool().await;
        let tmp = tempdir_unique("backfill-missing");
        std::fs::create_dir_all(&tmp).unwrap();
        // Intentionally no manifest written.
        insert_row(&pool, "m1", tmp.to_str().unwrap(), None).await;

        let n = backfill_verified(&pool).await.unwrap();
        assert_eq!(n, 0);
        assert_eq!(read_verified(&pool, "m1").await, None);
        cleanup(&tmp);
    }

    #[tokio::test]
    async fn backfill_skips_malformed_manifest_and_keeps_null() {
        let pool = test_pool().await;
        let tmp = tempdir_unique("backfill-malformed");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join(MANIFEST_FILE), b"not json at all").unwrap();
        insert_row(&pool, "m1", tmp.to_str().unwrap(), None).await;

        let n = backfill_verified(&pool).await.unwrap();
        assert_eq!(n, 0);
        assert_eq!(read_verified(&pool, "m1").await, None);
        cleanup(&tmp);
    }

    #[tokio::test]
    async fn backfill_ignores_rows_with_existing_count() {
        let pool = test_pool().await;
        let tmp = tempdir_unique("backfill-ignore");
        write_manifest_at(&tmp, &[("weights.bin", 1, Some("aa"))]);
        // Pre-existing value of 0 should NOT be overwritten with 1.
        insert_row(&pool, "m1", tmp.to_str().unwrap(), Some(0)).await;

        let n = backfill_verified(&pool).await.unwrap();
        assert_eq!(n, 0);
        assert_eq!(read_verified(&pool, "m1").await, Some(0));
        cleanup(&tmp);
    }

    #[tokio::test]
    async fn backfill_processes_multiple_rows_independently() {
        let pool = test_pool().await;
        let tmp_a = tempdir_unique("backfill-multi-a");
        let tmp_b = tempdir_unique("backfill-multi-b");
        write_manifest_at(&tmp_a, &[("w", 1, Some("aa")), ("c", 1, None)]);
        write_manifest_at(&tmp_b, &[("w", 1, Some("aa")), ("t", 1, Some("bb"))]);
        insert_row(&pool, "a", tmp_a.to_str().unwrap(), None).await;
        insert_row(&pool, "b", tmp_b.to_str().unwrap(), None).await;

        let n = backfill_verified(&pool).await.unwrap();
        assert_eq!(n, 2);
        assert_eq!(read_verified(&pool, "a").await, Some(1));
        assert_eq!(read_verified(&pool, "b").await, Some(2));
        cleanup(&tmp_a);
        cleanup(&tmp_b);
    }

    /// Tiny replacement for `tempfile::TempDir` so we don't need to add a
    /// new test-only dependency just for these tests. Each call gets a
    /// unique path under the OS temp dir; cleanup is explicit.
    fn tempdir_unique(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let p = std::env::temp_dir().join(format!("zero-test-{label}-{pid}-{n}"));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_dir_all(p);
    }
}
