//! Throttled walk progress shared by the walker's worker threads (SCAN-11).
//!
//! Workers bump atomic counters per entry; at most one update per
//! [`PROGRESS_INTERVAL`] is forwarded (a compare-and-swap on the last report
//! time elects the reporting thread), so the sink is never flooded and no lock
//! is taken per entry. [`Reporter::finish`] always forwards a final update.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use mj_core::progress::PROGRESS_INTERVAL;
use mj_core::{Phase, Progress, ProgressSink};

/// Shared progress state of one scan.
pub struct Reporter<'a> {
    sink: &'a dyn ProgressSink,
    start: Instant,
    /// Milliseconds since `start` of the last forwarded update (+1, 0 = never).
    last: AtomicU64,
    entries: AtomicU64,
    bytes: AtomicU64,
    interval_ms: u64,
}

impl<'a> Reporter<'a> {
    /// A reporter forwarding to `sink`.
    pub fn new(sink: &'a dyn ProgressSink) -> Self {
        Reporter {
            sink,
            start: Instant::now(),
            last: AtomicU64::new(0),
            entries: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            interval_ms: PROGRESS_INTERVAL.as_millis() as u64,
        }
    }

    /// Count one entry of `bytes`; maybe forward an update naming `current`.
    pub fn tick(&self, bytes: u64, current: &Path) {
        self.entries.fetch_add(1, Ordering::Relaxed);
        if bytes > 0 {
            self.bytes.fetch_add(bytes, Ordering::Relaxed);
        }
        let now = self.start.elapsed().as_millis() as u64 + 1;
        let last = self.last.load(Ordering::Relaxed);
        if (last == 0 || now >= last + self.interval_ms)
            && self.last.compare_exchange(last, now, Ordering::AcqRel, Ordering::Relaxed).is_ok()
        {
            self.sink.report(&Progress {
                phase: Phase::Walking,
                done: self.entries.load(Ordering::Relaxed),
                total: None,
                bytes: self.bytes.load(Ordering::Relaxed),
                current: Some(current.to_path_buf()),
            });
        }
    }

    /// Entries counted so far.
    #[cfg(test)]
    pub fn entries(&self) -> u64 {
        self.entries.load(Ordering::Relaxed)
    }

    /// Forward the final update (`total == done`, so throttles let it through).
    pub fn finish(&self, entries: u64, bytes: u64) {
        self.sink.report(&Progress {
            phase: Phase::Walking,
            done: entries,
            total: Some(entries),
            bytes,
            current: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn throttles_ticks_and_always_sends_final() {
        let got: Mutex<Vec<Progress>> = Mutex::new(Vec::new());
        let sink = |p: &Progress| got.lock().unwrap().push(p.clone());
        let r = Reporter::new(&sink);
        for _ in 0..10_000 {
            r.tick(10, Path::new("/x"));
        }
        assert_eq!(r.entries(), 10_000);
        let n = got.lock().unwrap().len();
        assert!((1..10).contains(&n), "{n} updates for an instant burst");
        r.finish(10_000, 100_000);
        let all = got.lock().unwrap();
        let last = all.last().unwrap();
        assert_eq!((last.done, last.total, last.bytes), (10_000, Some(10_000), 100_000));
        assert_eq!(all[0].current.as_deref(), Some(Path::new("/x")));
    }
}
