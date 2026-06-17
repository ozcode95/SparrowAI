//! OVMS readiness probe.
//!
//! After spawning the binary we don't get a clean "ready" signal on stdout —
//! the only reliable indicator is that the HTTP server has bound the port
//! and the configured model is loaded. We poll the OpenAI-compat
//! `/models` endpoint with capped exponential backoff until either it
//! returns 2xx or the overall deadline elapses.

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
