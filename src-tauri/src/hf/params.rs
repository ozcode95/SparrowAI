//! Per-model deployment parameters.
//!
//! Mirrors the knobs exposed by the upstream
//! `openvinotoolkit/model_server/demos/common/export_models/export_model.py`
//! tool, persisted as a `.zero_model.json` sidecar next to the model
//! weights. Consumed by [`crate::ovms::config`] when rendering
//! `graph.pbtxt` so the user's choices survive across restarts and are
//! always traceable back to a single, hand-editable file on disk.
//!
//! We intentionally keep the schema close to `export_model.py` field
//! names so anyone familiar with the upstream tool can map zero settings
//! one-to-one. Unknown / future fields are tolerated via `#[serde(default)]`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Filename of the sidecar written next to model weights. Hidden (`.`
/// prefix) so it doesn't clutter file listings, JSON for easy hand-edit.
pub const SIDECAR_FILE: &str = ".zero_model.json";

// ─── Helpers ────────────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}
fn default_max_num_seqs() -> u32 {
    256
}
fn default_pooling() -> String {
    "CLS".into()
}
fn default_max_doc_length() -> u32 {
    16000
}
fn default_one() -> u32 {
    1
}

// ─── Task-specific parameter blocks ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextGenParams {
    /// `LM` | `LM_CB` | `VLM` | `VLM_CB` | `AUTO`. `None` lets OVMS pick.
    #[serde(default)]
    pub pipeline_type: Option<String>,
    #[serde(default = "default_true")]
    pub enable_prefix_caching: bool,
    /// KV-cache size in GB. `0` = dynamic.
    #[serde(default)]
    pub cache_size: u32,
    #[serde(default = "default_max_num_seqs")]
    pub max_num_seqs: u32,
    /// `Some("u8")` halves KV-cache memory; `None` keeps the model default.
    #[serde(default)]
    pub kv_cache_precision: Option<String>,
    #[serde(default)]
    pub max_num_batched_tokens: Option<u32>,
    #[serde(default = "default_true")]
    pub dynamic_split_fuse: bool,
    /// NPU-only knob: maximum number of prompt tokens.
    #[serde(default)]
    pub max_prompt_len: Option<u32>,
    #[serde(default)]
    pub prompt_lookup_decoding: bool,
    /// `qwen3` | `gptoss` | `gemma4` — extracts thinking content from the
    /// stream. Mirrors the directories under OVMS
    /// `src/llm/io_processing/*/reasoning_parser.*`.
    #[serde(default)]
    pub reasoning_parser: Option<String>,
    /// `llama3` | `phi4` | `hermes3` | `mistral` | `qwen3coder` | `gptoss` |
    /// `devstral` | `lfm2` | `gemma4` — extracts tool-call args from the
    /// stream. Mirrors the directories under OVMS
    /// `src/llm/io_processing/*/tool_parser.*` (every family ships one
    /// except plain `qwen3`, which reuses `hermes3` per the OVMS docs).
    #[serde(default)]
    pub tool_parser: Option<String>,
    #[serde(default)]
    pub enable_tool_guided_generation: bool,
}

impl Default for TextGenParams {
    fn default() -> Self {
        Self {
            pipeline_type: None,
            enable_prefix_caching: true,
            cache_size: 0,
            max_num_seqs: 256,
            kv_cache_precision: None,
            max_num_batched_tokens: None,
            dynamic_split_fuse: true,
            max_prompt_len: None,
            prompt_lookup_decoding: false,
            reasoning_parser: None,
            tool_parser: None,
            enable_tool_guided_generation: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsParams {
    #[serde(default = "default_true")]
    pub normalize: bool,
    /// `CLS` | `LAST` | `MEAN`.
    #[serde(default = "default_pooling")]
    pub pooling: String,
    #[serde(default)]
    pub truncate: bool,
    #[serde(default = "default_one")]
    pub num_streams: u32,
}

impl Default for EmbeddingsParams {
    fn default() -> Self {
        Self {
            normalize: true,
            pooling: "CLS".into(),
            truncate: false,
            num_streams: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RerankParams {
    #[serde(default = "default_one")]
    pub num_streams: u32,
    #[serde(default = "default_max_doc_length")]
    pub max_doc_length: u32,
}

impl Default for RerankParams {
    fn default() -> Self {
        Self {
            num_streams: 1,
            max_doc_length: 16000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ImageGenParams {
    #[serde(default)]
    pub num_streams: u32,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub guidance_scale: Option<f32>,
    #[serde(default)]
    pub num_images_per_prompt: Option<u32>,
    #[serde(default)]
    pub max_resolution: Option<String>,
    #[serde(default)]
    pub default_resolution: Option<String>,
    #[serde(default)]
    pub max_num_images_per_prompt: Option<u32>,
    #[serde(default)]
    pub default_num_inference_steps: Option<u32>,
    #[serde(default)]
    pub max_num_inference_steps: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct T2sParams {
    #[serde(default)]
    pub num_streams: u32,
    #[serde(default)]
    pub speaker_name: Option<String>,
    /// Absolute or model-relative path to the `speaker.bin`.
    #[serde(default)]
    pub speaker_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct S2tParams {
    #[serde(default)]
    pub num_streams: u32,
    #[serde(default)]
    pub enable_word_timestamps: bool,
}

// ─── Task discriminator ────────────────────────────────────────────────────

/// Tagged union over the OVMS task families. The discriminator field is
/// `kind` to align with the convention used in the rest of the codebase
/// (chat error kinds, install stages, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelTask {
    TextGeneration(TextGenParams),
    Embeddings(EmbeddingsParams),
    Rerank(RerankParams),
    ImageGeneration(ImageGenParams),
    Text2Speech(T2sParams),
    Speech2Text(S2tParams),
}

impl Default for ModelTask {
    fn default() -> Self {
        ModelTask::TextGeneration(TextGenParams::default())
    }
}

impl ModelTask {
    /// Short stable identifier used in the UI and in log lines.
    pub fn kind_str(&self) -> &'static str {
        match self {
            ModelTask::TextGeneration(_) => "text_generation",
            ModelTask::Embeddings(_) => "embeddings",
            ModelTask::Rerank(_) => "rerank",
            ModelTask::ImageGeneration(_) => "image_generation",
            ModelTask::Text2Speech(_) => "text2speech",
            ModelTask::Speech2Text(_) => "speech2text",
        }
    }
}

/// Map per-model sidecar params back to a HuggingFace-style
/// `pipeline_tag` string (e.g. `text-generation`, `image-text-to-text`).
///
/// Used by [`crate::commands::models::models_list_local`] to backfill
/// the display tag for local rows whose upstream `pipeline_tag` was
/// never recorded (either downloaded before the column existed, or
/// upstream left it unset). The text-generation ↔ image-text-to-text
/// split is recovered from `TextGenParams.pipeline_type` (`VLM` /
/// `VLM_CB` map to `image-text-to-text`); everything else has a 1:1
/// task-kind → pipeline_tag mapping.
pub fn pipeline_tag_for(params: &ModelDeployParams) -> &'static str {
    match &params.task {
        ModelTask::TextGeneration(tg) => match tg.pipeline_type.as_deref() {
            Some(p) if p.eq_ignore_ascii_case("VLM") || p.eq_ignore_ascii_case("VLM_CB") => {
                "image-text-to-text"
            }
            _ => "text-generation",
        },
        ModelTask::Embeddings(_) => "feature-extraction",
        ModelTask::Rerank(_) => "text-ranking",
        ModelTask::ImageGeneration(_) => "text-to-image",
        ModelTask::Text2Speech(_) => "text-to-speech",
        ModelTask::Speech2Text(_) => "automatic-speech-recognition",
    }
}

// ─── Top-level sidecar shape ───────────────────────────────────────────────

/// Wraps [`ModelTask`] in a versioned envelope so the schema can evolve.
/// Stored at `<model_dir>/.zero_model.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelDeployParams {
    /// Schema version — bump in lockstep with breaking changes.
    #[serde(default = "current_version")]
    pub version: u32,
    /// Per-model OpenVINO device override (`CPU`, `GPU`, `NPU`, `AUTO`,
    /// `MULTI:CPU,GPU`, ...). `None` means inherit the runtime OVMS
    /// setting (`settings.ovms.device`). Honoured by
    /// [`crate::ovms::config::ensure_in_model_graph`] when rendering
    /// `graph.pbtxt`.
    #[serde(default)]
    pub device: Option<String>,
    pub task: ModelTask,
}

fn current_version() -> u32 {
    1
}

impl Default for ModelDeployParams {
    fn default() -> Self {
        Self {
            version: current_version(),
            device: None,
            task: ModelTask::default(),
        }
    }
}

// ─── Sidecar I/O ───────────────────────────────────────────────────────────

pub fn sidecar_path(model_dir: &Path) -> PathBuf {
    model_dir.join(SIDECAR_FILE)
}

/// Read params for `model_dir`. Returns [`ModelDeployParams::default`]
/// when the sidecar is missing or unparseable — both are non-fatal: the
/// user gets a sensible text-generation baseline and can override from
/// the UI.
pub fn read_or_default(model_dir: &Path) -> ModelDeployParams {
    let p = sidecar_path(model_dir);
    let Ok(bytes) = std::fs::read(&p) else {
        return ModelDeployParams::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        tracing::warn!(
            "model params sidecar at {} is malformed ({e}); falling back to defaults",
            p.display()
        );
        ModelDeployParams::default()
    })
}

pub fn write(model_dir: &Path, params: &ModelDeployParams) -> Result<()> {
    std::fs::create_dir_all(model_dir).with_context(|| format!("create {model_dir:?}"))?;
    let p = sidecar_path(model_dir);
    let bytes = serde_json::to_vec_pretty(params)?;
    std::fs::write(&p, bytes).with_context(|| format!("write {p:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static CTR: AtomicU64 = AtomicU64::new(0);
    fn tmpdir() -> PathBuf {
        let i = CTR.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("zero-params-test-{}-{}", std::process::id(), i));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn default_is_text_generation() {
        let p = ModelDeployParams::default();
        assert_eq!(p.version, 1);
        assert_eq!(p.task.kind_str(), "text_generation");
    }

    #[test]
    fn round_trip_through_disk() {
        let d = tmpdir();
        let original = ModelDeployParams {
            version: 1,
            device: Some("NPU".into()),
            task: ModelTask::TextGeneration(TextGenParams {
                tool_parser: Some("hermes3".into()),
                reasoning_parser: Some("qwen3".into()),
                cache_size: 4,
                ..Default::default()
            }),
        };
        write(&d, &original).unwrap();
        let restored = read_or_default(&d);
        assert_eq!(restored, original);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_sidecar_yields_default() {
        let d = tmpdir();
        let p = read_or_default(&d);
        assert_eq!(p, ModelDeployParams::default());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn malformed_sidecar_falls_back_to_default() {
        let d = tmpdir();
        std::fs::write(sidecar_path(&d), b"{ not json").unwrap();
        let p = read_or_default(&d);
        assert_eq!(p, ModelDeployParams::default());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn discriminator_is_kind_field() {
        let p = ModelDeployParams {
            version: 1,
            device: None,
            task: ModelTask::Embeddings(EmbeddingsParams::default()),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"kind\":\"embeddings\""));
        let back: ModelDeployParams = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn missing_device_field_is_tolerated() {
        // Sidecars written before per-model device override existed must
        // still load cleanly and surface `device: None` (= inherit runtime).
        let raw = br#"{"version":1,"task":{"kind":"text_generation"}}"#;
        let p: ModelDeployParams = serde_json::from_slice(raw).unwrap();
        assert!(p.device.is_none());
        assert_eq!(p.task.kind_str(), "text_generation");
    }
}
