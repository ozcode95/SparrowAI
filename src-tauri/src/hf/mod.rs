//! Hugging Face integration — search, download, update, delete.
//!
//! Layout on disk (under `<app-local>/zero/models/`):
//!
//! ```text
//! models/
//!   <org>/<repo>/
//!     <file_a>
//!     <file_b>
//!     .zero_manifest.json   ← revision SHA + per-file size/sha
//! ```
//!
//! The directory layout is intentionally flat (no per-revision sub-dir) so it
//! can be pointed at by OVMS / OpenVINO GenAI without indirection. The
//! manifest carries enough information to update only changed files and to
//! reconcile against the SQLite `local_models` row.
//!
//! Cancellation + concurrency are coordinated through [`DownloadJobs`]:
//! attempting to start a download for a model that already has an in-flight
//! job returns [`DownloadJobError::AlreadyRunning`] instead of racing against
//! itself on the same directory.

pub mod api;
pub mod detect;
pub mod download;
pub mod jobs;
pub mod params;

pub use api::search;
pub use detect::{suggest_from_inputs, suggest_params, DetectionInputs, HfConfig};
pub use download::{backfill_verified, delete as delete_model, install_or_update, Cancelled};
pub use jobs::{CancelHandle, DownloadJobError, DownloadJobs, JobToken};
pub use params::{
    EmbeddingsParams, ImageGenParams, ModelDeployParams, ModelTask, RerankParams, S2tParams,
    T2sParams, TextGenParams,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfModelSummary {
    pub id: String,
    pub author: String,
    pub downloads: u64,
    pub likes: u64,
    pub updated_at: String,
    pub tags: Vec<String>,
    pub pipeline_tag: Option<String>,
    pub has_openvino_ir: bool,
    pub total_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModel {
    pub id: String,
    pub path: String,
    pub bytes: u64,
    pub added_at: String,
    pub hf_id: Option<String>,
    pub revision: Option<String>,
    pub files: Option<u64>,
    /// How many of `files` carry a recorded sha256 — i.e. were verified
    /// against an HF-published LFS digest at download time. `None` when the
    /// manifest pre-dates verification tracking; a follow-up install will
    /// repopulate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_files: Option<u64>,
    /// Upstream HuggingFace `pipeline_tag` captured at download time —
    /// e.g. `text-generation`, `image-text-to-text`, `feature-extraction`.
    /// Used by the Models page to render a per-model category badge.
    /// `None` on rows that pre-date the column; readers backfill from the
    /// per-model sidecar (and finally fall back to `text-generation`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline_tag: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Pending,
    Downloading,
    Verifying,
    Done,
    Cancelled,
    Error,
}

/// Progress event for `models://download-progress`. The shape matches the
/// frontend `DownloadProgress` type in `src/stores/models.ts`.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub model_id: String,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub files_done: u64,
    pub files_total: u64,
    pub state: DownloadState,
    pub error: Option<String>,
}

/// Reads the HF token via [`crate::secrets`] (OS keychain, with a plaintext
/// fallback for environments without one). Returns `None` when nothing is
/// stored or the storage backend errored — callers treat the absence the
/// same way as a deliberate unset.
pub fn load_token() -> Option<String> {
    match crate::secrets::get("hf_token") {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("hf: load_token failed ({e:#})");
            None
        }
    }
}
