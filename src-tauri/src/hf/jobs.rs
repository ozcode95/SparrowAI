//! Per-process registry of in-flight HF model downloads.
//!
//! Two responsibilities:
//!
//! 1. **Concurrency guard.** Calling `start(id)` for a model that already has
//!    an active job returns [`DownloadJobError::AlreadyRunning`] instead of
//!    racing two writers against the same `<models>/<org>/<repo>/` tree.
//! 2. **Cancellation.** Each registered job hands the runner a [`CancelHandle`]
//!    that exposes both a synchronous `is_cancelled()` flag (for cheap checks
//!    between files) and a [`tokio::sync::Notify`] (so `tokio::select!`
//!    inside the per-chunk loop can interrupt a slow connection without
//!    waiting for the next byte).
//!
//! The registry intentionally does *not* track progress — that's still owned
//! by the frontend store, which receives `models://download-progress` events.
//! This module only deals with lifecycle bookkeeping.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Notify, RwLock};

/// Handle the runner uses to observe cancellation. Cloneable so the chunk
/// loop and the file loop can both consume it independently.
#[derive(Clone)]
pub struct CancelHandle {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl CancelHandle {
    /// Cheap synchronous check. Use at loop boundaries that already `.await`.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// Future the runner can `.await` (or feed into `tokio::select!`). Wakes
    /// as soon as `cancel(id)` fires on the parent registry.
    pub async fn wait(&self) {
        // Fast path: already flipped.
        if self.is_cancelled() {
            return;
        }
        self.notify.notified().await;
    }
}

/// Owned slot in the registry. Dropping it (e.g. when the download future is
/// aborted, panics, or returns) removes the entry so a subsequent `start` for
/// the same model is allowed.
pub struct JobToken {
    model_id: String,
    handle: CancelHandle,
    parent: Arc<DownloadJobs>,
}

impl JobToken {
    pub fn handle(&self) -> CancelHandle {
        self.handle.clone()
    }
}

impl Drop for JobToken {
    fn drop(&mut self) {
        // Can't `.await` in Drop; spawn a tiny task to release the slot.
        let parent = Arc::clone(&self.parent);
        let id = self.model_id.clone();
        tokio::spawn(async move {
            parent.inner.write().await.remove(&id);
        });
    }
}

#[derive(Debug, Error)]
pub enum DownloadJobError {
    #[error("a download for `{0}` is already in progress")]
    AlreadyRunning(String),
}

#[derive(Default)]
pub struct DownloadJobs {
    inner: RwLock<HashMap<String, CancelHandle>>,
}

impl DownloadJobs {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Reserve a slot for `model_id`. Returns a [`JobToken`] the runner must
    /// keep alive for the duration of the download. Returns
    /// [`DownloadJobError::AlreadyRunning`] if another job already holds the
    /// slot.
    pub async fn start(
        self: &Arc<Self>,
        model_id: &str,
    ) -> Result<JobToken, DownloadJobError> {
        let mut guard = self.inner.write().await;
        if guard.contains_key(model_id) {
            return Err(DownloadJobError::AlreadyRunning(model_id.to_string()));
        }
        let handle = CancelHandle {
            flag: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        };
        guard.insert(model_id.to_string(), handle.clone());
        Ok(JobToken {
            model_id: model_id.to_string(),
            handle,
            parent: Arc::clone(self),
        })
    }

    /// Notify the runner that the user wants to cancel. Returns `true` when a
    /// job was registered (and thus signalled), `false` otherwise.
    pub async fn cancel(&self, model_id: &str) -> bool {
        if let Some(h) = self.inner.read().await.get(model_id).cloned() {
            h.flag.store(true, Ordering::Release);
            h.notify.notify_waiters();
            true
        } else {
            false
        }
    }

    /// Whether a job is currently registered for `model_id`. Mostly for UI
    /// badges; not used by the runner itself.
    pub async fn is_running(&self, model_id: &str) -> bool {
        self.inner.read().await.contains_key(model_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn second_start_for_same_id_is_rejected() {
        let jobs = DownloadJobs::new();
        let _t1 = jobs.start("org/repo").await.expect("first start ok");
        match jobs.start("org/repo").await {
            Err(DownloadJobError::AlreadyRunning(id)) => assert_eq!(id, "org/repo"),
            Ok(_) => panic!("expected AlreadyRunning, got Ok"),
        }
    }

    #[tokio::test]
    async fn different_ids_are_independent() {
        let jobs = DownloadJobs::new();
        let _a = jobs.start("org/a").await.unwrap();
        let _b = jobs.start("org/b").await.unwrap();
        assert!(jobs.is_running("org/a").await);
        assert!(jobs.is_running("org/b").await);
    }

    #[tokio::test]
    async fn cancel_signals_via_flag_and_notify() {
        let jobs = DownloadJobs::new();
        let token = jobs.start("org/repo").await.unwrap();
        let handle = token.handle();
        assert!(!handle.is_cancelled());

        let waiter = tokio::spawn(async move { handle.wait().await });
        // Yield so `waiter` reaches `notified().await` before we fire.
        tokio::task::yield_now().await;

        assert!(jobs.cancel("org/repo").await);
        // The waiter must wake; if cancellation were broken this would hang.
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter did not wake within 1s")
            .unwrap();

        // And the synchronous flag is observable for boundary checks.
        assert!(token.handle().is_cancelled());
    }

    #[tokio::test]
    async fn cancel_on_unknown_id_is_a_noop() {
        let jobs = DownloadJobs::new();
        assert!(!jobs.cancel("nope").await);
    }

    #[tokio::test]
    async fn dropping_token_releases_slot() {
        let jobs = DownloadJobs::new();
        {
            let _t = jobs.start("org/repo").await.unwrap();
        }
        // Drop spawns a remove task; give it a tick to run.
        for _ in 0..10 {
            if !jobs.is_running("org/repo").await {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!jobs.is_running("org/repo").await);
        // Now a fresh start should succeed.
        let _again = jobs.start("org/repo").await.expect("slot was released");
    }
}
