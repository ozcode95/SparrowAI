//! Audio commands.
//!
//! Today the only command surfaces the OVMS speech-to-text endpoint
//! (`POST /v3/audio/transcriptions`, OpenAI-compatible) for the chat
//! page's voice-input button. Audio capture happens in the renderer
//! through the standard Web `MediaRecorder` API — the renderer hands us
//! the encoded blob and we forward it as `multipart/form-data` so the
//! Whisper-backed Mediapipe graph can decode + transcribe it.
//!
//! Keeping the HTTP hop on the Rust side (instead of `fetch` from the
//! webview) sidesteps two annoyances:
//!   * Tauri's webview sees the Tauri scheme as the origin and would
//!     need an explicit CORS exception on OVMS to talk to
//!     `http://127.0.0.1:8000`. Doing the POST here uses the shared
//!     `reqwest::Client` (configured with our corporate CA bundle), so
//!     everything just works.
//!   * The renderer doesn't have to think about content-type tweaks per
//!     codec — we'll happily forward whatever MIME type the browser
//!     produced (`audio/webm;codecs=opus`, `audio/ogg`, `audio/wav`).
//!     OVMS pipes the bytes through ffmpeg/libsndfile internally.

use crate::error::IpcResult;
use crate::ovms::OvmsStatus;
use crate::state::AppStateExt;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use tauri::AppHandle;

/// Subset of the OpenAI `audio/transcriptions` response shape we care
/// about. OVMS may include richer fields (`segments`, `words`, ...) when
/// `enable_word_timestamps` is on; we ignore them for the chat composer
/// flow, which only needs the flattened transcript.
#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
}

/// Transcribe a chunk of recorded audio using a Speech2Text model
/// currently loaded into OVMS. The caller passes the raw bytes the
/// browser's `MediaRecorder` produced plus the mime type so OVMS can
/// pick the right decoder.
///
/// Returns the bare transcript string so the renderer can drop it
/// straight into the composer textarea — keeping word timestamps /
/// segment info out of scope here since the chat use case doesn't
/// surface them.
#[tauri::command]
pub async fn audio_transcribe(
    app: AppHandle,
    model_id: String,
    audio: Vec<u8>,
    mime_type: Option<String>,
) -> IpcResult<String> {
    let state = app.zero();
    let info = state.ovms.info().await;

    // Fail fast with an actionable message instead of letting reqwest
    // surface a connection-refused at the user — OVMS isn't reachable
    // unless it's actually running.
    if !matches!(info.status, OvmsStatus::Running) {
        return Err(format!(
            "OVMS is not running (status: {:?}); start the server to use voice input",
            info.status
        )
        .into());
    }

    // Sanity-check the caller picked a model OVMS is actually serving
    // as speech-to-text. The renderer guards the UI on the same
    // condition, but a stale `info` snapshot or a race with an unload
    // could still slip through — surfacing a clear error here beats a
    // 400 from OVMS about an unknown model name.
    let stt_loaded = info
        .loaded_models
        .iter()
        .any(|m| m.id == model_id && m.task == "speech2text");
    if !stt_loaded {
        return Err(format!(
            "model `{model_id}` is not loaded as a speech-to-text endpoint"
        )
        .into());
    }

    let mime = mime_type
        .as_deref()
        .filter(|s| !s.is_empty())
        // Reasonable default — Chromium's MediaRecorder produces this
        // out of the box on Windows. OVMS handles it via ffmpeg.
        .unwrap_or("audio/webm");

    // Pick a filename extension that matches the mime so OVMS / ffmpeg
    // can sniff the container without needing an explicit `format` hint.
    let ext = match mime.split(';').next().unwrap_or(mime).trim() {
        "audio/webm" => "webm",
        "audio/ogg" => "ogg",
        "audio/mp4" | "audio/aac" => "m4a",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/flac" => "flac",
        _ => "webm",
    };

    let part = Part::bytes(audio)
        .file_name(format!("recording.{ext}"))
        .mime_str(mime)
        .map_err(|e| format!("invalid audio mime type `{mime}`: {e}"))?;

    let form = Form::new()
        .part("file", part)
        .text("model", model_id.clone())
        // `json` keeps the response shape stable and tiny — `text` would
        // return a raw string body which is awkward to distinguish from
        // an error body at the reqwest layer.
        .text("response_format", "json");

    // OVMS exposes the endpoint on the same root as chat/completions:
    // `<base_url>/audio/transcriptions`. `info.base_url` is already the
    // `/v3` root (e.g. `http://127.0.0.1:8000/v3`).
    let url = format!(
        "{}/audio/transcriptions",
        info.base_url.trim_end_matches('/')
    );

    let resp = state
        .http
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("read response body: {e}"))?;

    if !status.is_success() {
        // Pass the OVMS error body through verbatim — it usually
        // contains the actionable bit ("unsupported model", "decoding
        // failed", ...).
        return Err(format!("OVMS {status}: {body}").into());
    }

    let parsed: TranscriptionResponse = serde_json::from_str(&body).map_err(|e| {
        format!("invalid transcription response (expected `text` field): {e} — body: {body}")
    })?;

    Ok(parsed.text.trim().to_string())
}
