//! Smart inference of [`ModelDeployParams`] from upstream HF metadata.
//!
//! OVMS itself does not auto-detect the task / pipeline / tool parser for
//! a downloaded repo — `export_model.py` requires the user to pass those
//! explicitly. We close that gap here so the user gets sensible, model-
//! aware defaults instead of a blank text-generation form every time.
//!
//! Signals consulted (in priority order):
//!
//!   1. The repo `config.json` `architectures` field — most authoritative
//!      because it's emitted by the model authors themselves (e.g.
//!      `Qwen3ForCausalLM`, `Qwen3VLForConditionalGeneration`,
//!      `LlamaForCausalLM`, `BertModel`, `WhisperForConditionalGeneration`).
//!   2. The HF `pipeline_tag` (`text-generation`, `image-text-to-text`,
//!      `feature-extraction`, `text-to-image`, …).
//!   3. The HF `tags` list (often carries `multimodal`, `vision`, etc.
//!      that the pipeline_tag doesn't capture).
//!   4. The repo file listing — VLM-specific OpenVINO IR files
//!      (`openvino_vision_embeddings_model.xml`, …) and pipeline marker
//!      files (`model_index.json` for diffusers, etc.).
//!   5. The model id itself — pattern-matches the model family for
//!      [`TextGenParams::tool_parser`] / [`TextGenParams::reasoning_parser`]
//!      since those are properties of how the model was *trained* to emit
//!      tool calls and reasoning, which HF does not expose structurally.
//!
//! Mapping rules for tool / reasoning parsers come straight from
//! <https://docs.openvino.ai/2026/model-server/ovms_docs_llm_reference.html>
//! ("Output parsing settings" → supported parsers).

use super::api::{self, Sibling};
use super::params::{ModelDeployParams, ModelTask, TextGenParams};
use serde::Deserialize;

/// Convenience handle for everything detection consumes — bundles HF
/// `model_info` outputs with an optional `config.json` so individual
/// helpers can take it by reference.
#[derive(Debug, Default, Clone)]
pub struct DetectionInputs<'a> {
    pub id: &'a str,
    pub pipeline_tag: Option<&'a str>,
    pub tags: &'a [String],
    pub siblings: &'a [Sibling],
    pub config: Option<HfConfig>,
}

/// Subset of HF `config.json` we care about. All fields are optional so
/// missing keys / unrecognised model types degrade gracefully.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HfConfig {
    #[serde(default)]
    pub architectures: Vec<String>,
    #[serde(default)]
    pub model_type: Option<String>,
}

/// High-level: build a [`ModelDeployParams`] for `id` by hitting HF for
/// metadata + a `config.json` fetch, then folding everything into a
/// suggested set of defaults. Never errors — falls back to plain
/// text-generation defaults when HF is unreachable or the repo doesn't
/// match anything we recognise.
pub async fn suggest_params(http: &reqwest::Client, id: &str) -> ModelDeployParams {
    let info = match api::model_info(http, id, None).await {
        Ok(i) => i,
        Err(e) => {
            tracing::debug!("hf detect: model_info failed for {id}: {e:#} — using defaults");
            return ModelDeployParams::default();
        }
    };
    let revision = info.sha.as_deref();
    let config = match revision {
        Some(rev) => fetch_config_json(http, id, rev).await,
        None => None,
    };

    let mut params = ModelDeployParams::default();
    let detect = DetectionInputs {
        id,
        pipeline_tag: info.pipeline_tag.as_deref(),
        tags: &info.tags,
        siblings: &info.siblings,
        config,
    };
    apply(&mut params, &detect);
    params
}

/// Same as [`suggest_params`] but takes pre-fetched inputs. Used from
/// inside the download path where we already have a [`RepoInfo`] in scope.
pub fn suggest_from_inputs(inputs: &DetectionInputs<'_>) -> ModelDeployParams {
    let mut params = ModelDeployParams::default();
    apply(&mut params, inputs);
    params
}

/// Try to fetch `<repo>/config.json` at `revision`. Quietly returns
/// `None` for repos without one (image-gen, audio, datasets…), or on any
/// transport error.
pub async fn fetch_config_json(
    http: &reqwest::Client,
    id: &str,
    revision: &str,
) -> Option<HfConfig> {
    let url = api::resolve_url(id, revision, "config.json");
    let resp = http
        .get(&url)
        .headers(api::auth_headers())
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    serde_json::from_slice::<HfConfig>(&bytes).ok()
}

// ─── Core dispatch ─────────────────────────────────────────────────────────

fn apply(params: &mut ModelDeployParams, d: &DetectionInputs<'_>) {
    let task = detect_task(d);
    params.task = match task {
        TaskKind::TextGeneration { vlm } => {
            let mut p = TextGenParams::default();
            if vlm {
                // VLM_CB is the default servable for CPU/GPU; OVMS will
                // promote to plain `VLM` (stateful) automatically when
                // the user pins the model to NPU. Setting this
                // explicitly surfaces the VLM detection in the UI.
                p.pipeline_type = Some("VLM_CB".into());
            }
            p.tool_parser = detect_tool_parser(d);
            p.reasoning_parser = detect_reasoning_parser(d);
            ModelTask::TextGeneration(p)
        }
        TaskKind::Embeddings => ModelTask::Embeddings(Default::default()),
        TaskKind::Rerank => ModelTask::Rerank(Default::default()),
        TaskKind::ImageGeneration => ModelTask::ImageGeneration(Default::default()),
        TaskKind::Text2Speech => ModelTask::Text2Speech(Default::default()),
        TaskKind::Speech2Text => ModelTask::Speech2Text(Default::default()),
    };
}

// ─── Task detection ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskKind {
    TextGeneration { vlm: bool },
    Embeddings,
    Rerank,
    ImageGeneration,
    Text2Speech,
    Speech2Text,
}

fn detect_task(d: &DetectionInputs<'_>) -> TaskKind {
    // 1. `architectures` from config.json — most authoritative.
    if let Some(cfg) = &d.config {
        for arch in &cfg.architectures {
            let a = arch.to_ascii_lowercase();
            // VLM architectures usually carry "vl" / "vision" / "mllama" /
            // "conditional" / "image" markers.
            if is_vlm_architecture(&a) {
                return TaskKind::TextGeneration { vlm: true };
            }
            if a.ends_with("forcausallm")
                || a.ends_with("lmheadmodel")
                || a.ends_with("forconditionalgeneration") && looks_like_text_lm(&a)
            {
                return TaskKind::TextGeneration {
                    vlm: has_vlm_siblings(d.siblings),
                };
            }
            if a.contains("whisper") || a.contains("speech2text") || a.contains("wav2vec") {
                return TaskKind::Speech2Text;
            }
            if a.contains("speecht5") || a.contains("vits") || a.contains("bark") {
                return TaskKind::Text2Speech;
            }
            if a.ends_with("forsequenceclassification") {
                return if looks_like_reranker(d) {
                    TaskKind::Rerank
                } else {
                    // No 1:1 OVMS endpoint for plain text-classification —
                    // fall through to the heuristic below rather than
                    // returning an unsupported task.
                    continue;
                };
            }
            if a.ends_with("model") && (a.contains("bert") || a.contains("roberta")) {
                // Bare encoder models — usually embeddings unless the repo
                // is explicitly a cross-encoder reranker.
                return if looks_like_reranker(d) {
                    TaskKind::Rerank
                } else {
                    TaskKind::Embeddings
                };
            }
        }
    }

    // 2. HF pipeline_tag.
    if let Some(tag) = d.pipeline_tag.map(str::to_ascii_lowercase) {
        match tag.as_str() {
            "text-generation" => {
                return TaskKind::TextGeneration {
                    vlm: has_vlm_siblings(d.siblings) || tags_indicate_vlm(d.tags),
                };
            }
            "image-text-to-text" | "visual-question-answering" | "image-to-text" => {
                return TaskKind::TextGeneration { vlm: true };
            }
            "feature-extraction" | "sentence-similarity" => {
                return if looks_like_reranker(d) {
                    TaskKind::Rerank
                } else {
                    TaskKind::Embeddings
                };
            }
            "text-classification" | "text-ranking" => {
                if looks_like_reranker(d) {
                    return TaskKind::Rerank;
                }
            }
            "text-to-image" | "image-to-image" | "text-to-video" => {
                return TaskKind::ImageGeneration;
            }
            "text-to-speech" => return TaskKind::Text2Speech,
            "automatic-speech-recognition" => return TaskKind::Speech2Text,
            _ => {}
        }
    }

    // 3. Diffusers convention: `model_index.json` means image generation.
    if d.siblings.iter().any(|s| s.rfilename == "model_index.json") {
        return TaskKind::ImageGeneration;
    }

    // 4. Tag fallbacks.
    if d.tags.iter().any(|t| t.eq_ignore_ascii_case("reranker"))
        || d.id.to_ascii_lowercase().contains("rerank")
    {
        return TaskKind::Rerank;
    }
    if d.tags
        .iter()
        .any(|t| t.eq_ignore_ascii_case("sentence-transformers"))
        || d.id.to_ascii_lowercase().contains("embed")
    {
        return TaskKind::Embeddings;
    }
    if d.id.to_ascii_lowercase().contains("whisper") {
        return TaskKind::Speech2Text;
    }
    if d.id.to_ascii_lowercase().contains("speecht5") {
        return TaskKind::Text2Speech;
    }

    // 5. Final fallback — text generation, with VLM bit set only if the
    // sibling list clearly carries vision IR files.
    TaskKind::TextGeneration {
        vlm: has_vlm_siblings(d.siblings),
    }
}

fn is_vlm_architecture(a: &str) -> bool {
    // Architectures like `Qwen3VLForConditionalGeneration`,
    // `MllamaForConditionalGeneration`, `LlavaForConditionalGeneration`,
    // `InternVLChatModel`, `Phi3VForCausalLM`, `Idefics2ForConditionalGeneration`.
    a.contains("vl")
        && (a.contains("forcausallm")
            || a.contains("forconditionalgeneration")
            || a.contains("chatmodel"))
        || a.starts_with("llava")
        || a.starts_with("mllama")
        || a.starts_with("idefics")
        || a.starts_with("intern") && a.contains("vl")
        || a.contains("vision") && a.contains("causallm")
        || a.starts_with("phi")
            && a.contains("v")
            && (a.ends_with("forcausallm") || a.ends_with("forconditionalgeneration"))
}

fn looks_like_text_lm(a: &str) -> bool {
    // Encoder–decoder text models like t5/bart/pegasus emit
    // `*ForConditionalGeneration` too. Filter VLM out — caller already
    // checked `is_vlm_architecture` first — but block pure speech /
    // image models that share the suffix.
    !a.contains("whisper")
        && !a.contains("speecht5")
        && !a.contains("vits")
        && !a.contains("musicgen")
        && !a.contains("bark")
}

fn has_vlm_siblings(siblings: &[Sibling]) -> bool {
    siblings.iter().any(|s| {
        let f = s.rfilename.to_ascii_lowercase();
        // OVMS-converted VLMs ship split IR: a vision encoder + a
        // language model + optional text embeddings tower.
        f.ends_with("openvino_vision_embeddings_model.xml")
            || f.ends_with("openvino_text_embeddings_model.xml")
            || f.ends_with("openvino_language_model.xml")
            // Hugging Face VLM repos almost always include a processor.
            || f == "preprocessor_config.json"
            || f == "processor_config.json"
    })
}

fn tags_indicate_vlm(tags: &[String]) -> bool {
    tags.iter().any(|t| {
        let l = t.to_ascii_lowercase();
        l == "multimodal"
            || l == "vision"
            || l == "image-text-to-text"
            || l == "visual-question-answering"
    })
}

fn looks_like_reranker(d: &DetectionInputs<'_>) -> bool {
    let id = d.id.to_ascii_lowercase();
    if id.contains("rerank") || id.contains("reranker") {
        return true;
    }
    d.tags
        .iter()
        .any(|t| t.eq_ignore_ascii_case("reranker") || t.eq_ignore_ascii_case("text-ranking"))
}

// ─── Tool / reasoning parser detection ─────────────────────────────────────

/// Map a model id (and optional `config.json`) to one of the OVMS tool
/// parsers. Pattern matching mirrors the families listed in the docs.
fn detect_tool_parser(d: &DetectionInputs<'_>) -> Option<String> {
    let id = d.id.to_ascii_lowercase();
    let arch = d
        .config
        .as_ref()
        .and_then(|c| c.architectures.first())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let model_type = d
        .config
        .as_ref()
        .and_then(|c| c.model_type.as_deref())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    // Highest-specificity matches first.
    if id.contains("gpt-oss") || id.contains("gptoss") {
        return Some("gptoss".into());
    }
    if id.contains("qwen3-coder") || id.contains("qwen3coder") || id.contains("qwen2.5-coder") {
        return Some("qwen3coder".into());
    }
    if id.contains("devstral") {
        return Some("devstral".into());
    }
    if id.contains("hermes") {
        return Some("hermes3".into());
    }
    if id.contains("lfm2") || id.contains("lfm-2") {
        return Some("lfm2".into());
    }
    // Gemma 3+ ships an OVMS `gemma4` parser (see
    // `src/llm/io_processing/gemma4/gemma4_tool_parser.cpp`). Older Gemma
    // 1/2 chat templates use the same `<start_of_turn>` framing, so we
    // match the family broadly and let the user override in the UI if
    // needed.
    if id.contains("gemma") || arch.starts_with("gemma") || model_type == "gemma" {
        return Some("gemma4".into());
    }
    if id.contains("phi-4") || id.contains("phi4") || arch.starts_with("phi4") {
        return Some("phi4".into());
    }
    if id.contains("llama-3")
        || id.contains("llama3")
        || (arch.starts_with("llama") && id.contains("3"))
    {
        return Some("llama3".into());
    }
    // Per OVMS docs: hermes3 also works for Qwen3 models.
    if id.contains("qwen3")
        || id.contains("qwen-3")
        || arch.starts_with("qwen3")
        || model_type == "qwen3"
    {
        return Some("hermes3".into());
    }
    if id.contains("qwen2.5") {
        return Some("hermes3".into());
    }
    if id.contains("mistral") || arch.starts_with("mistral") {
        return Some("mistral".into());
    }
    None
}

fn detect_reasoning_parser(d: &DetectionInputs<'_>) -> Option<String> {
    let id = d.id.to_ascii_lowercase();
    let arch = d
        .config
        .as_ref()
        .and_then(|c| c.architectures.first())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if id.contains("gpt-oss") || id.contains("gptoss") {
        return Some("gptoss".into());
    }
    // Qwen3 (chat + thinking variants) emit `<think>` blocks — enable the
    // qwen3 reasoning parser for any model in that family.
    if id.contains("qwen3") || id.contains("qwen-3") || arch.starts_with("qwen3") {
        return Some("qwen3".into());
    }
    // Gemma 3+ thinking variants ship an OVMS `gemma4` reasoning parser.
    // The `gemma4` directory in `src/llm/io_processing/gemma4/` provides
    // both reasoning and tool parsers, so we surface the reasoning side
    // here and let it run on Gemma chat templates (older Gemmas without
    // think tags will simply produce no thinking content, which is fine).
    if id.contains("gemma") || arch.starts_with("gemma") {
        return Some("gemma4".into());
    }
    None
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hf::api::Sibling;

    fn sib(name: &str) -> Sibling {
        Sibling {
            rfilename: name.into(),
            size: None,
            lfs: None,
        }
    }

    fn cfg(archs: &[&str]) -> HfConfig {
        HfConfig {
            architectures: archs.iter().map(|s| (*s).to_string()).collect(),
            model_type: None,
        }
    }

    fn inputs<'a>(
        id: &'a str,
        pipeline: Option<&'a str>,
        tags: &'a [String],
        siblings: &'a [Sibling],
        config: Option<HfConfig>,
    ) -> DetectionInputs<'a> {
        DetectionInputs {
            id,
            pipeline_tag: pipeline,
            tags,
            siblings,
            config,
        }
    }

    // ── Task detection ──

    #[test]
    fn architectures_drive_text_generation_for_qwen3() {
        let sibs = vec![sib("openvino_model.xml"), sib("config.json")];
        let d = inputs(
            "Qwen/Qwen3-8B",
            Some("text-generation"),
            &[],
            &sibs,
            Some(cfg(&["Qwen3ForCausalLM"])),
        );
        let p = suggest_from_inputs(&d);
        assert_eq!(p.task.kind_str(), "text_generation");
        let ModelTask::TextGeneration(tg) = &p.task else {
            panic!("expected text_generation");
        };
        // Qwen3 → hermes3 tool parser + qwen3 reasoning parser per OVMS docs.
        assert_eq!(tg.tool_parser.as_deref(), Some("hermes3"));
        assert_eq!(tg.reasoning_parser.as_deref(), Some("qwen3"));
        // Plain LM → no explicit pipeline_type override.
        assert!(tg.pipeline_type.is_none());
    }

    #[test]
    fn vlm_architecture_sets_vlm_pipeline_type() {
        let sibs = vec![
            sib("openvino_language_model.xml"),
            sib("openvino_vision_embeddings_model.xml"),
            sib("preprocessor_config.json"),
        ];
        let d = inputs(
            "OpenVINO/Qwen3-VL-7B-Instruct-int4-ov",
            Some("image-text-to-text"),
            &[],
            &sibs,
            Some(cfg(&["Qwen3VLForConditionalGeneration"])),
        );
        let p = suggest_from_inputs(&d);
        let ModelTask::TextGeneration(tg) = &p.task else {
            panic!("expected text_generation for VLM");
        };
        assert_eq!(tg.pipeline_type.as_deref(), Some("VLM_CB"));
    }

    #[test]
    fn vlm_detected_from_siblings_when_arch_is_generic() {
        // No config.json available but the file list contains a vision
        // tower — we still flip to VLM_CB.
        let sibs = vec![
            sib("openvino_language_model.xml"),
            sib("openvino_vision_embeddings_model.xml"),
        ];
        let d = inputs("some/vlm-repo", Some("text-generation"), &[], &sibs, None);
        let p = suggest_from_inputs(&d);
        let ModelTask::TextGeneration(tg) = &p.task else {
            panic!();
        };
        assert_eq!(tg.pipeline_type.as_deref(), Some("VLM_CB"));
    }

    #[test]
    fn whisper_id_pattern_picks_speech2text() {
        let sibs = vec![sib("openvino_model.xml")];
        let d = inputs("openai/whisper-small", None, &[], &sibs, None);
        let p = suggest_from_inputs(&d);
        assert_eq!(p.task.kind_str(), "speech2text");
    }

    #[test]
    fn whisper_architecture_picks_speech2text() {
        let sibs = vec![sib("openvino_model.xml")];
        let d = inputs(
            "some/repo",
            None,
            &[],
            &sibs,
            Some(cfg(&["WhisperForConditionalGeneration"])),
        );
        let p = suggest_from_inputs(&d);
        assert_eq!(p.task.kind_str(), "speech2text");
    }

    #[test]
    fn diffusers_model_index_picks_image_generation() {
        let sibs = vec![sib("model_index.json"), sib("unet/config.json")];
        let d = inputs("stabilityai/sd", None, &[], &sibs, None);
        let p = suggest_from_inputs(&d);
        assert_eq!(p.task.kind_str(), "image_generation");
    }

    #[test]
    fn text_to_image_pipeline_tag_picks_image_generation() {
        let sibs = vec![sib("openvino_model.xml")];
        let d = inputs("any/flux", Some("text-to-image"), &[], &sibs, None);
        let p = suggest_from_inputs(&d);
        assert_eq!(p.task.kind_str(), "image_generation");
    }

    #[test]
    fn bge_reranker_picks_rerank() {
        let sibs = vec![sib("config.json")];
        let d = inputs(
            "BAAI/bge-reranker-large",
            Some("text-classification"),
            &[],
            &sibs,
            None,
        );
        let p = suggest_from_inputs(&d);
        assert_eq!(p.task.kind_str(), "rerank");
    }

    #[test]
    fn bert_encoder_picks_embeddings_when_not_reranker() {
        let sibs = vec![sib("config.json")];
        let d = inputs(
            "BAAI/bge-large-en-v1.5",
            Some("feature-extraction"),
            &[],
            &sibs,
            Some(cfg(&["BertModel"])),
        );
        let p = suggest_from_inputs(&d);
        assert_eq!(p.task.kind_str(), "embeddings");
    }

    #[test]
    fn sentence_similarity_pipeline_picks_embeddings() {
        let sibs = vec![sib("config.json")];
        let d = inputs(
            "sentence-transformers/all-MiniLM-L6-v2",
            Some("sentence-similarity"),
            &[],
            &sibs,
            None,
        );
        let p = suggest_from_inputs(&d);
        assert_eq!(p.task.kind_str(), "embeddings");
    }

    #[test]
    fn speecht5_picks_text2speech() {
        let sibs = vec![sib("config.json")];
        let d = inputs(
            "microsoft/speecht5_tts",
            Some("text-to-speech"),
            &[],
            &sibs,
            None,
        );
        let p = suggest_from_inputs(&d);
        assert_eq!(p.task.kind_str(), "text2speech");
    }

    // ── Tool parser detection ──

    #[test]
    fn tool_parser_llama3() {
        let sibs = vec![];
        let d = inputs(
            "meta-llama/Meta-Llama-3-8B-Instruct",
            Some("text-generation"),
            &[],
            &sibs,
            Some(cfg(&["LlamaForCausalLM"])),
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert_eq!(tg.tool_parser.as_deref(), Some("llama3"));
    }

    #[test]
    fn tool_parser_qwen3coder_beats_hermes3() {
        let sibs = vec![];
        let d = inputs(
            "Qwen/Qwen3-Coder-7B-Instruct",
            Some("text-generation"),
            &[],
            &sibs,
            None,
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert_eq!(tg.tool_parser.as_deref(), Some("qwen3coder"));
    }

    #[test]
    fn tool_parser_gptoss_sets_both_parsers() {
        let sibs = vec![];
        let d = inputs(
            "openai/gpt-oss-20b",
            Some("text-generation"),
            &[],
            &sibs,
            None,
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert_eq!(tg.tool_parser.as_deref(), Some("gptoss"));
        assert_eq!(tg.reasoning_parser.as_deref(), Some("gptoss"));
    }

    #[test]
    fn tool_parser_mistral() {
        let sibs = vec![];
        let d = inputs(
            "mistralai/Mistral-7B-Instruct-v0.3",
            Some("text-generation"),
            &[],
            &sibs,
            Some(cfg(&["MistralForCausalLM"])),
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert_eq!(tg.tool_parser.as_deref(), Some("mistral"));
    }

    #[test]
    fn tool_parser_devstral_beats_mistral() {
        let sibs = vec![];
        let d = inputs(
            "mistralai/Devstral-Small-2507",
            Some("text-generation"),
            &[],
            &sibs,
            None,
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert_eq!(tg.tool_parser.as_deref(), Some("devstral"));
    }

    #[test]
    fn tool_parser_phi4() {
        let sibs = vec![];
        let d = inputs(
            "microsoft/Phi-4-mini-instruct",
            Some("text-generation"),
            &[],
            &sibs,
            None,
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert_eq!(tg.tool_parser.as_deref(), Some("phi4"));
    }

    #[test]
    fn tool_parser_hermes_beats_qwen3_lookup() {
        let sibs = vec![];
        let d = inputs(
            "NousResearch/Hermes-3-Llama-3.1-8B",
            Some("text-generation"),
            &[],
            &sibs,
            None,
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert_eq!(tg.tool_parser.as_deref(), Some("hermes3"));
    }

    #[test]
    fn tool_parser_lfm2() {
        let sibs = vec![];
        let d = inputs(
            "LiquidAI/LFM2-1.2B",
            Some("text-generation"),
            &[],
            &sibs,
            None,
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert_eq!(tg.tool_parser.as_deref(), Some("lfm2"));
    }

    #[test]
    fn tool_parser_gemma4_sets_both_parsers() {
        // OVMS `src/llm/io_processing/gemma4/` ships both a tool_parser
        // and a reasoning_parser, so a Gemma model should land structured
        // function-calling and thinking extraction without manual setup.
        let sibs = vec![];
        let d = inputs(
            "OpenVINO/gemma-4-E2B-it-int8-ov",
            Some("text-generation"),
            &[],
            &sibs,
            None,
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert_eq!(tg.tool_parser.as_deref(), Some("gemma4"));
        assert_eq!(tg.reasoning_parser.as_deref(), Some("gemma4"));
    }

    #[test]
    fn tool_parser_gemma_family_matches_broadly() {
        // Older Gemma releases share the chat template the `gemma4`
        // parser targets, so we accept the whole family.
        for id in [
            "google/gemma-2-9b-it",
            "google/gemma-3-4b-it",
            "OpenVINO/gemma-2b-it-int4-ov",
        ] {
            let sibs = vec![];
            let d = inputs(id, Some("text-generation"), &[], &sibs, None);
            let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
                panic!("{id}: expected text_generation");
            };
            assert_eq!(
                tg.tool_parser.as_deref(),
                Some("gemma4"),
                "{id}: unexpected tool_parser"
            );
        }
    }

    #[test]
    fn no_match_leaves_tool_parser_unset() {
        let sibs = vec![];
        let d = inputs(
            "some-org/obscure-finetune",
            Some("text-generation"),
            &[],
            &sibs,
            None,
        );
        let ModelTask::TextGeneration(tg) = &suggest_from_inputs(&d).task else {
            panic!();
        };
        assert!(tg.tool_parser.is_none());
        assert!(tg.reasoning_parser.is_none());
    }
}
