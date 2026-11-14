//! Forwards engine events to the webview (`app.emit(name, payload)`). Owner: W3-D.
//!
//! The engine already throttles progress to one update per 50 ms per source; the
//! bridge applies the same rule again per source (scan generation, job id) as a
//! guard against floods, and never drops a final progress value or any other event.
//! Emit failures are ignored: they only happen while the window is closing.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mj_engine::api::{Event, EventSink};
use tauri::{AppHandle, Emitter};

/// Minimum spacing of non-final progress events per source.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// Per-source throttle for progress events (pure, testable).
#[derive(Default)]
pub struct Throttle {
    last: Mutex<HashMap<(u8, u64), Instant>>,
}

impl Throttle {
    /// True when `event` should be forwarded now.
    pub fn pass(&self, event: &Event, now: Instant) -> bool {
        let (key, is_final) = match event {
            Event::ScanProgress(p) => ((0u8, p.generation), p.total.is_some_and(|t| p.done >= t)),
            Event::JobProgress(p) => ((1u8, p.job_id.0), p.progress.total > 0 && p.progress.done >= p.progress.total),
            _ => return true,
        };
        let mut last = mj_core::lock(&self.last);
        let ok = is_final || last.get(&key).is_none_or(|t| now.duration_since(*t) >= PROGRESS_INTERVAL);
        if ok {
            last.insert(key, now);
        }
        ok
    }
}

/// Event sink backed by the Tauri app handle.
pub struct TauriSink {
    app: AppHandle,
    throttle: Throttle,
}

impl TauriSink {
    /// Sink for `app`.
    pub fn new(app: AppHandle) -> Self {
        TauriSink { app, throttle: Throttle::default() }
    }
}

impl EventSink for TauriSink {
    fn emit(&self, event: &Event) {
        if self.throttle.pass(event, Instant::now()) {
            let _ = self.app.emit(event.name(), event.payload());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_engine::api::{HistoryChanged, ScanProgressEvent};

    #[test]
    fn throttles_progress_per_source_and_keeps_finals() {
        let t = Throttle::default();
        let now = Instant::now();
        let p = |generation, done, total| {
            Event::ScanProgress(ScanProgressEvent { generation, done, total, ..Default::default() })
        };
        assert!(t.pass(&p(1, 1, None), now));
        assert!(!t.pass(&p(1, 2, None), now + Duration::from_millis(10)));
        assert!(t.pass(&p(2, 2, None), now + Duration::from_millis(10)), "another source");
        assert!(t.pass(&p(1, 5, Some(5)), now + Duration::from_millis(11)), "final always passes");
        assert!(t.pass(&p(1, 6, None), now + Duration::from_millis(80)));
        assert!(t.pass(&Event::HistoryChanged(HistoryChanged { run_id: None }), now));
    }
}
