//! OVMS readiness probes.
//!
//! After spawning the binary we don't get a clean "ready" signal on stdout —
//! readiness has two distinct phases:
//!
//! 1. **HTTP up** — the REST server has bound the port and answers basic
//!    requests. We probe the OpenAI-compat `/models` endpoint for this.
//!    This is what flips the bottom-bar `Ready` chip.
//! 2. **MediaPipe graph(s) loaded** — each text-gen / embed / rerank model
//!    is served as a MediaPipe pipeline whose graph still needs to compile
//!    and load weights *after* the HTTP server is up. Until this completes,
//!    chat requests fail with `404 Mediapipe graph definition with
//!    requested name is not found`. We poll the TFS `/v1/config` endpoint
//!    until every expected model reports state `AVAILABLE` before flipping
//!    the controller to `Running`.
//!
//! Both probes use capped exponential backoff and a shared overall deadline.

use crate::ovms::tfs;
use anyhow::{anyhow, Result};
use std::time::{Duration, Instant};

/// Block until OVMS answers a `GET <base_url>/models` with success, or
/// `timeout` elapses. Connection refused / 5xx responses are treated as
/// "not ready yet" and retried.
pub async fn wait_ready(
    http: &reqwest::Client,
    base_url: &str,
    timeout: Duration,
) -> Result<()> {
    let probe = format!("{}/models", base_url.trim_end_matches('/'));
    let started = Instant::now();
    let mut delay = Duration::from_millis(200);

    loop {
        if started.elapsed() >= timeout {
            return Err(anyhow!(
                "ovms did not become ready within {:?} (last probe: {})",
                timeout,
                probe
            ));
        }

        match http
            .get(&probe)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                tracing::trace!(
                    "ovms health: {} {} (still waiting)",
                    resp.status(),
                    probe
                );
            }
            Err(e) => {
                tracing::trace!("ovms health: {e} (still waiting)");
            }
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
}

/// Poll `GET /v1/config` until every model in `expected` reports state
/// `AVAILABLE`, or `timeout` elapses.
///
/// The HTTP server comes up well before MediaPipe graphs finish
/// compiling — without this second wait the controller flips to
/// `Running` on the first `/models` 200, the UI unlocks the chat
/// composer, and the user's first message hits OVMS with `404 Mediapipe
/// graph definition with requested name is not found`. Each iteration
/// also surfaces transient TFS errors (typically `connection refused`
/// during the brief window between port bind and config publication)
/// to trace logs without aborting.
pub async fn wait_models_available(
    http: &reqwest::Client,
    base_url: &str,
    expected: &[String],
    timeout: Duration,
) -> Result<()> {
    if expected.is_empty() {
        return Ok(());
    }
    let started = Instant::now();
    let mut delay = Duration::from_millis(250);
    let mut last_summary: Option<String> = None;

    loop {
        if started.elapsed() >= timeout {
            let detail = last_summary.unwrap_or_else(|| "no status reported".into());
            return Err(anyhow!(
                "ovms models did not become AVAILABLE within {:?}: {}",
                timeout,
                detail
            ));
        }

        match tfs::config_status(http, base_url).await {
            Ok(status) => {
                let names: Vec<&str> = expected.iter().map(String::as_str).collect();
                if tfs::all_available(&status, names.iter().copied()) {
                    return Ok(());
                }
                last_summary = tfs::unavailable_summary(&status, names.iter().copied());
                tracing::trace!(
                    "ovms model readiness: still waiting ({})",
                    last_summary.as_deref().unwrap_or("unknown")
                );
            }
            Err(e) => {
                tracing::trace!("ovms model readiness probe error: {e:#} (still waiting)");
            }
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
}
