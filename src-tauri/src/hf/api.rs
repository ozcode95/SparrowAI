//! Hugging Face HTTP API helpers.
//!
//! We hit `huggingface.co/api/models` directly with `reqwest`. Once the rest
//! of the app is stable we may swap to the `hf-hub` crate for cached/resumable
//! downloads, but doing it by hand for now keeps the dependency surface small
//! and lets us emit fine-grained progress events on our own terms.

use super::{load_token, HfModelSummary};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Deserialize;

pub const BASE: &str = "https://huggingface.co";

#[derive(Debug, Clone, Deserialize)]
pub struct Sibling {
    pub rfilename: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// Present on LFS-tracked files when the repo metadata is requested with
    /// `?blobs=true`. Carries the upstream sha256 we can verify against
    /// after streaming the file to disk.
    #[serde(default)]
    pub lfs: Option<LfsInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LfsInfo {
    /// LFS object size in bytes. Authoritative for large files (the
    /// non-`lfs` `size` field for these is the pointer-file size, not the
    /// real content size).
    #[serde(default)]
    pub size: Option<u64>,
    /// Hex-encoded sha256 of the resolved blob. We compare this against a
    /// streaming hash of the downloaded bytes; any mismatch aborts the
    /// install.
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RepoInfo {
    #[serde(rename = "id")]
    pub id: String,
    /// Commit SHA of the requested revision (defaults to `main`). HF returns
    /// this for any model info call, even when no `revision` query param is
    /// provided.
    #[serde(default)]
    pub sha: Option<String>,
    #[serde(default)]
    pub siblings: Vec<Sibling>,
    /// Free-form HF tags (`text-generation`, `multimodal`, `openvino`, …).
    /// Consumed by [`crate::hf::detect`] to disambiguate VLMs and rerankers
    /// when `pipeline_tag` alone isn't decisive.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Primary task tag the model author chose on the model page
    /// (`text-generation`, `image-text-to-text`, `feature-extraction`, …).
    /// Optional because some repos leave it unset.
    #[serde(default)]
    pub pipeline_tag: Option<String>,
}

#[derive(Deserialize)]
struct ApiModel {
    #[serde(rename = "id")]
    id: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    downloads: Option<u64>,
    #[serde(default)]
    likes: Option<u64>,
    #[serde(rename = "lastModified", default)]
    last_modified: Option<DateTime<Utc>>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    pipeline_tag: Option<String>,
    #[serde(default)]
    siblings: Vec<Sibling>,
}

/// Build the auth header set. Always returns at least the `Accept` header so
/// callers can attach it unconditionally.
pub fn auth_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("Accept", HeaderValue::from_static("application/json"));
    if let Some(tok) = load_token() {
        if let Ok(val) = HeaderValue::from_str(&format!("Bearer {tok}")) {
            h.insert(AUTHORIZATION, val);
        }
    }
    h
}

/// Search HF for models. Currently flags OpenVINO IR locally based on the
/// sibling list; later we'll push the filter to the API via `filter=openvino`.
pub async fn search(http: &reqwest::Client, query: &str) -> Result<Vec<HfModelSummary>> {
    let url = format!("{BASE}/api/models");
    let resp: Vec<ApiModel> = http
        .get(&url)
        .headers(auth_headers())
        .query(&[
            ("search", query),
            ("limit", "40"),
            ("full", "true"),
            ("sort", "downloads"),
            ("direction", "-1"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(resp.into_iter().map(map_summary).collect())
}

/// Fetch repo metadata + sibling list, optionally pinned to a revision.
///
/// `?blobs=true` asks HF to populate sibling sizes from the LFS pointer when
/// available; for tiny non-LFS files we fall back to a HEAD per file later.
pub async fn model_info(
    http: &reqwest::Client,
    id: &str,
    revision: Option<&str>,
) -> Result<RepoInfo> {
    let url = format!("{BASE}/api/models/{id}");
    let mut req = http
        .get(&url)
        .headers(auth_headers())
        .query(&[("blobs", "true")]);
    if let Some(rev) = revision {
        req = req.query(&[("revision", rev)]);
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("hf model_info {id}"))?;
    let info: RepoInfo = resp.json().await?;
    if info.siblings.is_empty() {
        return Err(anyhow!("hf: repo {id} has no files"));
    }
    Ok(info)
}

/// HEAD a resolve URL to discover the real content length (including the
/// LFS-redirected size). Returns `None` if the server doesn't advertise one.
pub async fn head_size(
    http: &reqwest::Client,
    id: &str,
    revision: &str,
    file: &str,
) -> Result<Option<u64>> {
    let url = resolve_url(id, revision, file);
    let resp = http
        .head(&url)
        .headers(auth_headers())
        .send()
        .await?
        .error_for_status()?;

    // Prefer the LFS-aware header HF sets on pointer hits, fall back to
    // content-length for plain blobs.
    if let Some(v) = resp.headers().get("x-linked-size") {
        if let Ok(s) = v.to_str() {
            if let Ok(n) = s.parse::<u64>() {
                return Ok(Some(n));
            }
        }
    }
    if let Some(n) = resp.content_length() {
        return Ok(Some(n));
    }
    Ok(None)
}

pub fn resolve_url(id: &str, revision: &str, file: &str) -> String {
    format!("{BASE}/{id}/resolve/{revision}/{file}")
}

fn map_summary(m: ApiModel) -> HfModelSummary {
    let has_ir = m
        .siblings
        .iter()
        .any(|s| s.rfilename.ends_with(".xml") || s.rfilename.contains("openvino"));
    let total: u64 = m.siblings.iter().filter_map(|s| s.size).sum();
    let author = m
        .author
        .or_else(|| m.id.split('/').next().map(|s| s.to_string()))
        .unwrap_or_default();
    HfModelSummary {
        id: m.id,
        author,
        downloads: m.downloads.unwrap_or(0),
        likes: m.likes.unwrap_or(0),
        updated_at: m.last_modified.map(|d| d.to_rfc3339()).unwrap_or_default(),
        tags: m.tags,
        pipeline_tag: m.pipeline_tag,
        has_openvino_ir: has_ir,
        total_size_bytes: if total > 0 { Some(total) } else { None },
    }
}
