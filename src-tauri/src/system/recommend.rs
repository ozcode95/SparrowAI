//! Specs-aware model recommendation engine.
//!
//! Text-generation models are discovered dynamically via the HuggingFace
//! API.  All other categories (embeddings, rerank, image generation,
//! speech‑to‑text, text‑to‑speech) use a hand‑picked static catalogue
//! that is filtered against the user's hardware budget.
//!
//! Results are cached to `~/.zero/recommended_models.json` (refreshed
//! every 24 h).

use crate::hf::api;
use crate::paths;
use crate::system::Specs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── wire types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRecommendations {
    /// ISO‑8601 timestamp of when the cache was populated.
    pub cached_at: String,
    /// Task‑kind → ranked model list.
    pub models: RecommendationMap,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedModel {
    pub hf_id: String,
    pub blurb: String,
    pub size_hint: String,
}

pub type RecommendationMap = HashMap<String, Vec<RecommendedModel>>;

// ─── memory estimation ──────────────────────────────────────────────────────

fn estimated_size_gb(param_b: f64, quant: &str) -> f64 {
    match quant {
        "int4" => param_b * 0.65,
        "int8" => param_b * 1.15,
        _ => param_b * 1.0,
    }
}

fn size_hint(param_b: f64, quant: &str) -> String {
    let gb = estimated_size_gb(param_b, quant);
    if gb < 1.0 {
        format!("~{} MB", (gb * 1024.0) as u32)
    } else {
        format!("~{:.1} GB", gb)
    }
}

fn parse_model_size(hf_id: &str) -> Option<(f64, String)> {
    let lower = hf_id.to_lowercase();
    let quant = if lower.contains("-int8") {
        "int8"
    } else if lower.contains("-int4") {
        "int4"
    } else {
        return None;
    };
    if let Some(b) = extract_b_number(&lower) {
        return Some((b, quant.to_string()));
    }
    // Whisper size aliases (approx params in billions)
    if lower.contains("whisper") {
        if lower.contains("distil-large") {
            return Some((0.76, quant.to_string()));
        }
        if lower.contains("large") {
            return Some((1.55, quant.to_string()));
        }
        if lower.contains("medium") {
            return Some((0.77, quant.to_string()));
        }
        if lower.contains("small") {
            return Some((0.24, quant.to_string()));
        }
        if lower.contains("base") {
            return Some((0.07, quant.to_string()));
        }
        if lower.contains("tiny") {
            return Some((0.04, quant.to_string()));
        }
    }
    // Known aliases — LLMs
    if lower.contains("phi-3.5-mini") {
        return Some((3.8, quant.to_string()));
    }
    if lower.contains("phi-3.5-vision") {
        return Some((4.2, quant.to_string()));
    }
    if lower.contains("gemma-4-e4b") {
        return Some((4.4, quant.to_string()));
    }
    if lower.contains("gemma-4-e2b") {
        return Some((2.6, quant.to_string()));
    }
    if lower.contains("gemma-4-26b") {
        return Some((26.0, quant.to_string()));
    }
    if lower.contains("gemma-4-31b") {
        return Some((31.0, quant.to_string()));
    }
    // Embedding / reranker models
    if lower.contains("bge-base-en-v1.5") {
        return Some((0.11, quant.to_string()));
    }
    if lower.contains("bge-reranker-base") {
        return Some((0.28, quant.to_string()));
    }
    // Image generation models
    if lower.contains("stable-diffusion-v1-5") || lower.contains("stable-diffusion-v1.5") {
        return Some((0.86, quant.to_string()));
    }
    if lower.contains("stable-diffusion-xl") && !lower.contains("v1") {
        return Some((2.6, quant.to_string()));
    }
    None
}

fn extract_b_number(lower: &str) -> Option<f64> {
    let bytes = lower.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'b' && i > 0 && (bytes[i - 1].is_ascii_digit() || bytes[i - 1] == b'.') {
            let mut start = i;
            while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
                start -= 1;
            }
            if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
                continue;
            }
            let num_str = std::str::from_utf8(&bytes[start..i]).ok()?;
            if num_str.contains('a') {
                continue;
            }
            if let Ok(n) = num_str.parse::<f64>() {
                return Some(n);
            }
        }
    }
    None
}

// ─── hardware budget ────────────────────────────────────────────────────────

struct HwBudget {
    model_budget_gb: f64,
}

impl HwBudget {
    fn from_specs(specs: &Specs) -> Self {
        let system_ram_gb = specs.ram_total_mb as f64 / 1024.0;
        let intel_vram_gb = specs
            .gpus
            .iter()
            .filter(|g| g.vendor.to_lowercase().contains("intel"))
            .filter_map(|g| g.vram_mb.map(|v| v as f64 / 1024.0))
            .fold(0.0, f64::max);

        let model_budget_gb = if intel_vram_gb > 1.0 {
            intel_vram_gb * 0.85
        } else {
            (system_ram_gb - 2.0).max(1.0) * 0.65
        };
        Self { model_budget_gb }
    }
}

fn blurb_for(kind: &str, param_b: Option<f64>, quant: &str) -> String {
    let size_str = param_b.map_or(String::new(), |b| format!("{:.0}B ", b));
    match kind {
        "text_generation" => format!("{}{} chat model", size_str, quant.to_uppercase()),
        "embeddings" => format!("{}{} embedding model", size_str, quant.to_uppercase()),
        "rerank" => format!("{}{} reranker", size_str, quant.to_uppercase()),
        "image_generation" => format!("{}{} image gen model", size_str, quant.to_uppercase()),
        "speech2text" => format!("{}{} speech‑to‑text model", size_str, quant.to_uppercase()),
        _ => format!("{}{} model", size_str, quant.to_uppercase()),
    }
}

// ─── static catalogue (non‑LLM categories) ───────────────────────────────────

/// Hardcoded recommendations for embeddings, rerank, image gen, STT, and TTS.
/// These are well‑tested OpenVINO int8 IR models that the HF search API may
/// not reliably surface (e.g. missing pipeline tags).
fn static_catalogue() -> RecommendationMap {
    let mut map: RecommendationMap = HashMap::new();

    // ── embeddings ───────────────────────────────────────────────────────
    map.insert(
        "embeddings".to_string(),
        vec![
            entry("OpenVINO/bge-base-en-v1.5-int8-ov", "embeddings"),
            entry("OpenVINO/Qwen3-Embedding-0.6B-int8-ov", "embeddings"),
        ],
    );

    // ── rerank ───────────────────────────────────────────────────────────
    map.insert(
        "rerank".to_string(),
        vec![
            entry("OpenVINO/bge-reranker-base-int8-ov", "rerank"),
            entry("OpenVINO/Qwen3-Reranker-0.6B-int8-ov", "rerank"),
        ],
    );

    // ── image generation ─────────────────────────────────────────────────
    map.insert(
        "image_generation".to_string(),
        vec![
            entry("OpenVINO/stable-diffusion-v1-5-int8-ov", "image_generation"),
            entry(
                "OpenVINO/stable-diffusion-xl-base-1.0-int8-ov",
                "image_generation",
            ),
        ],
    );

    // ── speech‑to‑text (Whisper family, smallest → largest) ──────────────
    map.insert(
        "speech2text".to_string(),
        vec![
            entry("OpenVINO/whisper-tiny-int8-ov", "speech2text"),
            entry("OpenVINO/whisper-base-int8-ov", "speech2text"),
            entry("OpenVINO/whisper-small-int8-ov", "speech2text"),
            entry("OpenVINO/whisper-medium-int8-ov", "speech2text"),
            entry("OpenVINO/whisper-large-v3-int8-ov", "speech2text"),
        ],
    );

    // ── text‑to‑speech (none available from OpenVINO org yet) ────────────
    map.insert("text2speech".to_string(), vec![]);

    map
}

fn entry(hf_id: &str, kind: &str) -> RecommendedModel {
    let (param_b, quant) = parse_model_size(hf_id).unwrap_or((0.0, "int8".to_string()));
    RecommendedModel {
        hf_id: hf_id.to_string(),
        blurb: blurb_for(kind, Some(param_b), &quant),
        size_hint: size_hint(param_b, &quant),
    }
}

// ─── HF search → pipeline_tag mapping ───────────────────────────────────────

fn classify_pipeline(pipeline_tag: Option<&str>, model_id: &str) -> Option<&'static str> {
    let id_lower = model_id.to_lowercase();
    match pipeline_tag.unwrap_or("") {
        "text-generation" | "image-text-to-text" | "conversational" => Some("text_generation"),
        "feature-extraction" | "sentence-similarity" => Some("embeddings"),
        "text-classification" | "text-ranking" => Some("rerank"),
        "text-to-image" | "image-to-image" => Some("image_generation"),
        "automatic-speech-recognition" => Some("speech2text"),
        "text-to-speech" => Some("text2speech"),
        _ => {
            // Fallback: many OpenVINO-org repos don't set pipeline_tag.
            // Infer from model id so we don't silently drop valid models.
            if id_lower.contains("stable-diffusion")
                || id_lower.contains("flux.1")
                || id_lower.contains("juggernaut")
                || id_lower.contains("dreamshaper")
                || id_lower.contains("lcm_dreamshaper")
            {
                Some("image_generation")
            } else if id_lower.contains("whisper") {
                Some("speech2text")
            } else if id_lower.contains("speecht5") || id_lower.contains("-tts-") {
                Some("text2speech")
            } else {
                None
            }
        }
    }
}

// ─── HF search ──────────────────────────────────────────────────────────────

/// Build the final recommendation map by combining the static catalogue
/// (non‑LLM categories) with dynamically‑discovered text‑generation models.
async fn search_hf(http: &reqwest::Client, specs: &Specs) -> RecommendationMap {
    let budget = HwBudget::from_specs(specs);

    // ── static catalogue (non‑LLM) ───────────────────────────────────────
    let mut map = static_catalogue();

    // Filter each static category against the hardware budget.
    for list in map.values_mut() {
        list.retain(|m| {
            let Some((param_b, quant)) = parse_model_size(&m.hf_id) else {
                return true; // keep if we can't estimate size
            };
            estimated_size_gb(param_b, &quant) <= budget.model_budget_gb
        });
        // Non‑LLM: sort largest first (best quality).
        list.sort_by(|a, b| {
            let sa = parse_model_size(&a.hf_id)
                .map(|(p, _)| estimated_size_gb(p, ""))
                .unwrap_or(0.0);
            let sb = parse_model_size(&b.hf_id)
                .map(|(p, _)| estimated_size_gb(p, ""))
                .unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // ── dynamic search for text‑generation models ────────────────────────
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let llm_queries: &[&str] = &["OpenVINO Qwen3.5", "OpenVINO gemma-4", "OpenVINO Phi-3.5"];

    for query in llm_queries {
        let results = match api::search(http, query).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("recommend: search '{query}' failed: {e:#}");
                continue;
            }
        };

        for m in results {
            // Require OpenVINO org.
            if !m.id.starts_with("OpenVINO/") {
                continue;
            }

            // int8 only.
            if !m.id.to_lowercase().contains("-int8") {
                continue;
            }

            // Must be a text-generation pipeline.
            let actual_kind = classify_pipeline(m.pipeline_tag.as_deref(), &m.id);
            if actual_kind != Some("text_generation") {
                continue;
            }

            // Deduplicate across overlapping queries.
            if !seen.insert(m.id.clone()) {
                continue;
            }

            let Some((param_b, quant)) = parse_model_size(&m.id) else {
                continue;
            };

            let size_gb = estimated_size_gb(param_b, &quant);
            if size_gb > budget.model_budget_gb {
                continue;
            }

            let rec = RecommendedModel {
                hf_id: m.id.clone(),
                blurb: blurb_for("text_generation", Some(param_b), &quant),
                size_hint: size_hint(param_b, &quant),
            };

            map.entry("text_generation".to_string())
                .or_default()
                .push(rec);
        }
    }

    // Sort text_generation: smaller models first (cheaper to run).
    if let Some(list) = map.get_mut("text_generation") {
        list.sort_by(|a, b| {
            let sa = parse_model_size(&a.hf_id)
                .map(|(p, _)| estimated_size_gb(p, ""))
                .unwrap_or(0.0);
            let sb = parse_model_size(&b.hf_id)
                .map(|(p, _)| estimated_size_gb(p, ""))
                .unwrap_or(0.0);
            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    map
}

// ─── cache ──────────────────────────────────────────────────────────────────

const CACHE_MAX_AGE_HOURS: i64 = 24;

fn load_cache() -> Option<RecommendationMap> {
    let path = paths::recommended_models_cache().ok()?;
    let data = std::fs::read_to_string(&path).ok()?;
    let cached: CachedRecommendations = serde_json::from_str(&data).ok()?;

    // Check staleness.
    if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&cached.cached_at) {
        let age = chrono::Utc::now() - ts.to_utc();
        if age.num_hours() < CACHE_MAX_AGE_HOURS {
            tracing::info!("recommend: using cached models ({} h old)", age.num_hours());
            return Some(cached.models);
        }
    }

    tracing::info!("recommend: cache expired, will refresh");
    None
}

fn save_cache(models: &RecommendationMap) {
    let path = match paths::recommended_models_cache() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("recommend: cache path error: {e:#}");
            return;
        }
    };

    let cached = CachedRecommendations {
        cached_at: chrono::Utc::now().to_rfc3339(),
        models: models.clone(),
    };

    match serde_json::to_vec_pretty(&cached) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, &json) {
                tracing::warn!("recommend: cache write failed: {e:#}");
            }
        }
        Err(e) => tracing::warn!("recommend: cache serialization failed: {e:#}"),
    }
}

// ─── public API ─────────────────────────────────────────────────────────────

/// Return spec-ranked recommendations for every task category.
///
/// Uses a 24‑h disk cache; falls back to live HF search when the cache is
/// stale or missing.
pub async fn recommend_all(http: &reqwest::Client, specs: &Specs) -> RecommendationMap {
    // Try cache first.
    if let Some(cached) = load_cache() {
        return cached;
    }

    // Live search + cache the result.
    let map = search_hf(http, specs).await;
    save_cache(&map);
    map
}
