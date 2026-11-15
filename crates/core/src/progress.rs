//! Progress reporting: engines report to a [`ProgressSink`]; front ends adapt
//! it to events. [`ThrottledSink`] enforces the 50 ms rule (about 20 updates
//! per second) so the webview is never flooded; final updates always pass.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::lock;

/// Minimum interval between two forwarded progress updates.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// What a long operation is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Walking directories / enumerating the MFT.
    Walking,
    /// Loading or saving the scan cache.
    Cache,
    /// Hashing file contents (partial or full).
    Hashing,
    /// Running detectors and optimiser analysis.
    Detecting,
    /// Computing estimates (dry run).
    Estimating,
    /// Applying actions.
    Applying,
    /// Undoing / restoring.
    Undoing,
    /// Writing an export or report.
    Exporting,
}

/// One progress update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub phase: Phase,
    /// Items done in this phase.
    pub done: u64,
    /// Items in this phase, when known.
    pub total: Option<u64>,
    /// Bytes processed so far in this phase.
    pub bytes: u64,
    /// Item being processed, if meaningful.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub current: Option<PathBuf>,
}

impl Progress {
    /// A progress value with no current item.
    pub fn new(phase: Phase, done: u64, total: Option<u64>) -> Self {
        Progress { phase, done, total, bytes: 0, current: None }
    }

    /// True when `done >= total`.
    pub fn is_final(&self) -> bool {
        matches!(self.total, Some(t) if self.done >= t)
    }
}

/// Receiver of progress updates. Must be cheap and non-blocking.
pub trait ProgressSink: Send + Sync {
    fn report(&self, progress: &Progress);
}

/// Discards everything.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullProgress;

impl ProgressSink for NullProgress {
    fn report(&self, _: &Progress) {}
}

impl<F: Fn(&Progress) + Send + Sync> ProgressSink for F {
    fn report(&self, progress: &Progress) {
        self(progress)
    }
}

/// Forwards at most one update per `interval`, plus every final update and
/// every phase change.
pub struct ThrottledSink<S: ProgressSink> {
    inner: S,
    interval: Duration,
    last: Mutex<Option<(Instant, Phase)>>,
}

impl<S: ProgressSink> ThrottledSink<S> {
    /// Throttle with [`PROGRESS_INTERVAL`].
    pub fn new(inner: S) -> Self {
        Self::with_interval(inner, PROGRESS_INTERVAL)
    }

    /// Throttle with a custom interval.
    pub fn with_interval(inner: S, interval: Duration) -> Self {
        ThrottledSink { inner, interval, last: Mutex::new(None) }
    }

    /// The wrapped sink.
    pub fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: ProgressSink> ProgressSink for ThrottledSink<S> {
    fn report(&self, p: &Progress) {
        let now = Instant::now();
        let mut last = lock(&self.last);
        let pass = match *last {
            None => true,
            Some((t, phase)) => phase != p.phase || p.is_final() || now.duration_since(t) >= self.interval,
        };
        if pass {
            *last = Some((now, p.phase));
            drop(last);
            self.inner.report(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn throttles_but_keeps_final_and_phase_changes() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let sink = ThrottledSink::with_interval(
            move |_: &Progress| {
                c.fetch_add(1, Ordering::SeqCst);
            },
            Duration::from_secs(3600),
        );
        for i in 0..1000 {
            sink.report(&Progress::new(Phase::Walking, i, Some(1000)));
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        sink.report(&Progress::new(Phase::Walking, 1000, Some(1000)));
        assert_eq!(count.load(Ordering::SeqCst), 2);
        sink.report(&Progress::new(Phase::Hashing, 0, None));
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }
}
