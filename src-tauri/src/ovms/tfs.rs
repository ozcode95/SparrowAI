//! Minimal client for OVMS's TensorFlow-Serving-compatible REST API.
//!
//! Reference:
//! <https://github.com/openvinotoolkit/model_server/blob/v2026.2/docs/model_server_rest_api_tfs.md>
//!
//! We only need the three administrative endpoints — predictions still
//! go through the OpenAI-compat `/v3/...` surface that lives on the same
//! port. The endpoints we wrap here are:
//!
//! - `POST /v1/config/reload` — pick up any edits the controller just
//!   made to `config.json` *without* restarting the server. This is what
//!   lets zero add / remove a model from the live serving set in-place
//!   instead of bouncing the whole OVMS process.
//! - `GET /v1/config` — aggregated status of every loaded model + DAG.
//! - `GET /v1/models/<name>` — status of one specific model. Used as a
//!   per-model readiness probe after reload.
//! - `GET /v1/models/<name>/metadata` — input/output tensor signature
//!   (names, dtypes, shapes) for one model. Powers the Settings page's
//!   model inspector. The response shape is mirror-of-TF-Serving JSON,
//!   too freeform (and too dependent on the served task) for us to type
//!   here — we just pass the parsed `serde_json::Value` through to the
//!   frontend which renders it inline.
//!
//! All three live under `/v1/...` regardless of where the OpenAI surface
//! sits — the controller stores its OpenAI base URL (usually
//! `http://127.0.0.1:<port>/v3`) so we re-derive the host:port prefix
//! once in [`tfs_base`].

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Per-version row inside the TFS status responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelVersionStatus {
    pub version: String,
    pub state: String,
    pub status: ModelStatusError,
}

/// `{"error_code", "error_message"}` block attached to each version row.
/// `error_code` is `"OK"` (string, not number) on success — OVMS mirrors
/// the TensorFlow Serving wire format here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStatusError {
    pub error_code: String,
    pub error_message: String,
}

/// Wrapper that matches the response shape for a single model's status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStatus {
    pub model_version_status: Vec<ModelVersionStatus>,
}

/// Map of model-name → status. Used by both [`reload_config`] and
/// [`config_status`].
pub type ConfigStatus = HashMap<String, ModelStatus>;

/// Strip the OpenAI-compat suffix (`/v3`, `/v3/`, `/v1`, …) off
/// `openai_base_url` and return the bare `scheme://host:port` prefix the
/// TFS endpoints are nested under.
///
/// We don't take a dependency on the `url` crate here — the OVMS base
/// URLs we deal with are well-formed by construction (we build them
/// ourselves in [`crate::settings::OvmsSettings::base_url`]) so a cheap
/// split-on-slashes is more than enough.
pub fn tfs_base(openai_base_url: &str) -> String {
    let trimmed = openai_base_url.trim_end_matches('/');
    // Walk back past the trailing path component (e.g. `/v3`) but keep
    // the `scheme://host[:port]` prefix intact. URLs without a path
    // segment after the authority fall through unchanged.
    let scheme_split = trimmed.find("://").map(|i| i + 3).unwrap_or(0);
    let authority_end = trimmed[scheme_split..]
        .find('/')
        .map(|i| scheme_split + i)
        .unwrap_or(trimmed.len());
    trimmed[..authority_end].to_string()
}

/// `POST /v1/config/reload`. Returns the aggregated status of every
/// model after reload (success), or an error containing the upstream
/// JSON `{"error": "..."}` body when OVMS rejected the reload.
///
/// HTTP status semantics, per the upstream docs:
/// - `201` — config or model versions were actually reloaded.
/// - `200` — reload not required (config unchanged).
/// - other — failure; body carries an `error` field.
pub async fn reload_config(http: &reqwest::Client, openai_base_url: &str) -> Result<ConfigStatus> {
    let url = format!("{}/v1/config/reload", tfs_base(openai_base_url));
    let resp = http
        .post(&url)
        // Reloads block on model init (can be slow for big LLMs on
        // first compile), so the timeout has to cover a worst-case
        // cold load — but not so long that a wedged reload pins the
        // controller forever.
        .timeout(Duration::from_secs(180))
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    decode_tfs_response("reload_config", resp).await
}

/// `GET /v1/config`. Aggregated status of every served model + DAG.
pub async fn config_status(http: &reqwest::Client, openai_base_url: &str) -> Result<ConfigStatus> {
    let url = format!("{}/v1/config", tfs_base(openai_base_url));
    let resp = http
        .get(&url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    decode_tfs_response("config_status", resp).await
}

/// `GET /v1/models/<name>`. Per-model readiness probe.
///
/// The model name is percent-encoded into a single path segment
/// (notably turning `/` into `%2F`) before being interpolated into the
/// URL. zero's model ids are HF-style `org/repo` strings, and OVMS
/// rejects an unencoded `/` in this URL with `HTTP 400 Invalid request
/// URL` because it tries to parse the second segment as a sub-resource
/// (`/v1/models/<name>/versions/<v>`). Encoding keeps the whole HF id
/// inside one segment, which OVMS then matches against the `name`
/// field of the loaded `mediapipe_config_list` entry.
pub async fn model_status(
    http: &reqwest::Client,
    openai_base_url: &str,
    model_name: &str,
) -> Result<ModelStatus> {
    let url = format!(
        "{}/v1/models/{}",
        tfs_base(openai_base_url),
        urlencoding::encode(model_name)
    );
    let resp = http
        .get(&url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!(
            "model_status `{model_name}`: HTTP {status}: {body}"
        ));
    }
    serde_json::from_str(&body)
        .with_context(|| format!("parse model_status response (model `{model_name}`): {body}"))
}

/// `GET /v1/models/<name>/metadata`. Returns the raw JSON document OVMS
/// emits — the shape is `{ modelSpec, metadata: { signature_def: ... } }`
/// for text-generation pipelines and similar-but-different for embeddings
/// / rerank / VLM models, so we leave parsing to the renderer instead of
/// typing every variant here.
///
/// Per the upstream contract any 2xx is success; non-2xx bodies usually
/// carry an `{"error":"..."}` field that we surface verbatim.
pub async fn model_metadata(
    http: &reqwest::Client,
    openai_base_url: &str,
    model_name: &str,
) -> Result<serde_json::Value> {
    let url = format!(
        "{}/v1/models/{}/metadata",
        tfs_base(openai_base_url),
        // Percent-encode for the same reason as `model_status` — OVMS
        // 400s an unencoded `/` in the name segment.
        urlencoding::encode(model_name)
    );
    let resp = http
        .get(&url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&body)
        {
            if let Some(serde_json::Value::String(msg)) = map.get("error") {
                return Err(anyhow!(
                    "model_metadata `{model_name}`: HTTP {status}: {msg}"
                ));
            }
        }
        return Err(anyhow!(
            "model_metadata `{model_name}`: HTTP {status}: {body}"
        ));
    }
    serde_json::from_str(&body)
        .with_context(|| format!("parse model_metadata response (model `{model_name}`): {body}"))
}

/// Whether every version of every model in `expected` is reported as
/// `AVAILABLE` in `status`. Models OVMS reports that aren't in
/// `expected` are ignored — in particular this is what lets us treat a
/// reload that drops a model as success: the removed model echoes back
/// as `END` (TFS' confirmation of an unload completing) and isn't in
/// the expected set, so it doesn't count against readiness. A missing
/// expected entry, or any non-`AVAILABLE` state on one, is treated as
/// not-ready.
///
/// Used after [`reload_config`] to confirm a freshly added model
/// actually finished loading before the controller flips its status
/// back to `Running`.
pub fn all_available<'a, I>(status: &ConfigStatus, expected: I) -> bool
where
    I: IntoIterator<Item = &'a str>,
{
    expected.into_iter().all(|name| {
        status.get(name).is_some_and(|m| {
            !m.model_version_status.is_empty()
                && m.model_version_status
                    .iter()
                    .all(|v| v.state == "AVAILABLE")
        })
    })
}

/// Best-effort one-line summary of any non-`AVAILABLE` rows for the
/// models in `expected`, suitable for inclusion in an error message.
///
/// Same filtering rule as [`all_available`]: rows for models OVMS
/// reports but that aren't in `expected` are skipped, so a successful
/// unload (which echoes back as `END`) doesn't pollute the error
/// message.
pub fn unavailable_summary<'a, I>(status: &ConfigStatus, expected: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut bad: Vec<String> = Vec::new();
    for name in expected {
        match status.get(name) {
            None => bad.push(format!("{name}: missing from reload response")),
            Some(m) if m.model_version_status.is_empty() => {
                bad.push(format!("{name}: no version reported"));
            }
            Some(m) => {
                for v in &m.model_version_status {
                    if v.state != "AVAILABLE" {
                        bad.push(format!(
                            "{name} v{} ({}): {} {}",
                            v.version, v.state, v.status.error_code, v.status.error_message
                        ));
                    }
                }
            }
        }
    }
    if bad.is_empty() {
        None
    } else {
        Some(bad.join("; "))
    }
}

/// Shared decoder for the two endpoints that return the same
/// `Map<name, ModelStatus>` shape.
async fn decode_tfs_response(label: &str, resp: reqwest::Response) -> Result<ConfigStatus> {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // OVMS returns `{"error": "..."}` on the error path; surface
        // that text directly if present so the controller's log isn't
        // just `HTTP 4xx`.
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&body)
        {
            if let Some(serde_json::Value::String(msg)) = map.get("error") {
                return Err(anyhow!("{label}: HTTP {status}: {msg}"));
            }
        }
        return Err(anyhow!("{label}: HTTP {status}: {body}"));
    }
    serde_json::from_str(&body).with_context(|| format!("parse {label} response: {body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tfs_base_strips_v3_path_suffix() {
        assert_eq!(
            tfs_base("http://127.0.0.1:8000/v3"),
            "http://127.0.0.1:8000"
        );
        assert_eq!(
            tfs_base("http://127.0.0.1:8000/v3/"),
            "http://127.0.0.1:8000"
        );
        assert_eq!(tfs_base("https://host:1234/v1"), "https://host:1234");
    }

    #[test]
    fn tfs_base_handles_bare_authority() {
        assert_eq!(tfs_base("http://127.0.0.1:8000"), "http://127.0.0.1:8000");
        assert_eq!(tfs_base("http://127.0.0.1:8000/"), "http://127.0.0.1:8000");
    }

    #[test]
    fn all_available_is_true_for_empty_expected_set() {
        // No models expected → vacuously available, regardless of what
        // OVMS reports. The controller never reaches this path with an
        // empty serving set anyway (it delegates to `stop` instead).
        let s: ConfigStatus = HashMap::new();
        assert!(all_available(&s, std::iter::empty::<&str>()));
    }

    #[test]
    fn all_available_ignores_unloaded_models_in_end_state() {
        // Reproduces the unload-reload bug: removing a model from
        // config.json causes OVMS to echo it back in `END` state.
        // Validation must only care about models we actually want
        // loaded — the just-removed one isn't expected, so the reload
        // counts as successful.
        let mut s: ConfigStatus = HashMap::new();
        s.insert(
            "org/qwen".into(),
            ModelStatus {
                model_version_status: vec![ModelVersionStatus {
                    version: "1".into(),
                    state: "AVAILABLE".into(),
                    status: ModelStatusError {
                        error_code: "OK".into(),
                        error_message: String::new(),
                    },
                }],
            },
        );
        s.insert(
            "org/gemma".into(),
            ModelStatus {
                model_version_status: vec![ModelVersionStatus {
                    version: "1".into(),
                    state: "END".into(),
                    status: ModelStatusError {
                        error_code: "OK".into(),
                        error_message: "OK".into(),
                    },
                }],
            },
        );
        assert!(all_available(&s, ["org/qwen"]));
        assert!(unavailable_summary(&s, ["org/qwen"]).is_none());
    }

    #[test]
    fn unavailable_summary_lists_bad_rows() {
        let mut s: ConfigStatus = HashMap::new();
        s.insert(
            "org/text".into(),
            ModelStatus {
                model_version_status: vec![ModelVersionStatus {
                    version: "1".into(),
                    state: "AVAILABLE".into(),
                    status: ModelStatusError {
                        error_code: "OK".into(),
                        error_message: String::new(),
                    },
                }],
            },
        );
        s.insert(
            "org/emb".into(),
            ModelStatus {
                model_version_status: vec![ModelVersionStatus {
                    version: "1".into(),
                    state: "LOADING".into(),
                    status: ModelStatusError {
                        error_code: "OK".into(),
                        error_message: String::new(),
                    },
                }],
            },
        );
        let summary =
            unavailable_summary(&s, ["org/text", "org/emb"]).expect("one bad row expected");
        assert!(summary.contains("org/emb"));
        assert!(summary.contains("LOADING"));
        assert!(!summary.contains("org/text"));
    }

    #[test]
    fn unavailable_summary_flags_missing_expected_model() {
        // OVMS reload response is missing a model we asked for — should
        // surface as a clear "missing from reload response" line
        // instead of silently passing.
        let s: ConfigStatus = HashMap::new();
        let summary = unavailable_summary(&s, ["org/qwen"]).expect("missing model");
        assert!(summary.contains("org/qwen"));
        assert!(summary.contains("missing"));
        assert!(!all_available(&s, ["org/qwen"]));
    }
}
