//! Adaptive concurrency for reads on flaky filesystems (research §2.3).
//!
//! Some network mounts (GVFS/SMB) fail whole-file reads with `EINVAL` as soon
//! as more than one is in flight. Pass 1 runs the work in parallel and watches
//! the failure ratio: once at least [`PROBE`] items finished and more than half
//! failed, it stops handing out work. Pass 2 then runs every item that was not
//! attempted or failed **serially**, with up to [`SERIAL_ATTEMPTS`] attempts and
//! a short back-off. On a healthy filesystem pass 2 never runs. Errors that
//! cannot change on retry (not found, permission denied, changed during the
//! scan) are not retried. Every item that still fails is returned as an error,
//! so the caller can report it as unreadable: a failed read never looks like
//! "no duplicate".

use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use mj_core::{lock, CancelToken, Error, Result};

/// Items that must finish before the failure ratio is judged.
pub const PROBE: usize = 12;
/// Serial attempts per item in pass 2.
pub const SERIAL_ATTEMPTS: usize = 2;

/// Why one read failed.
#[derive(Debug)]
pub enum ReadError {
    /// The OS reported an error.
    Io(io::Error),
    /// The file's size no longer matches the scan (being written, replaced).
    Changed,
    /// The work panicked on this item (hostile content); never retried.
    Panicked,
}

impl ReadError {
    fn retryable(&self) -> bool {
        match self {
            ReadError::Io(e) => !matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied),
            ReadError::Changed | ReadError::Panicked => false,
        }
    }
}

impl From<io::Error> for ReadError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            ReadError::Changed
        } else {
            ReadError::Io(e)
        }
    }
}

/// Result of [`run`].
pub struct Adaptive<R> {
    /// One result per item, index-aligned.
    pub results: Vec<std::result::Result<R, ReadError>>,
    /// Pass 1 was abandoned because concurrent reads failed.
    pub fell_back: bool,
}

/// Run `work` over `items` with the parallel-then-serial strategy.
/// `Err(Cancelled)` when the token is cancelled (results are discarded).
pub fn run<T, R, F>(items: &[T], workers: usize, cancel: &CancelToken, work: F) -> Result<Adaptive<R>>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> std::result::Result<R, ReadError> + Sync,
{
    let n = items.len();
    let workers = if workers == 0 { mj_core::pool::default_workers() } else { workers }.clamp(1, n.max(1));
    let call = |i: usize| catch_unwind(AssertUnwindSafe(|| work(&items[i]))).unwrap_or(Err(ReadError::Panicked));
    let slots: Vec<Mutex<Option<std::result::Result<R, ReadError>>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let mut fell_back = false;

    if workers > 1 {
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let abort = AtomicBool::new(false);
        let worker = || loop {
            if abort.load(Ordering::Relaxed) || cancel.checkpoint().is_err() {
                return;
            }
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= n {
                return;
            }
            let r = call(i);
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            let f = if r.is_err() { failed.fetch_add(1, Ordering::Relaxed) + 1 } else { failed.load(Ordering::Relaxed) };
            *lock(&slots[i]) = Some(r);
            if d >= PROBE && f * 2 > d {
                abort.store(true, Ordering::Relaxed);
            }
        };
        std::thread::scope(|s| {
            for w in 0..workers {
                let spawned = std::thread::Builder::new()
                    .name(format!("mj-dupes-io-{w}"))
                    .stack_size(mj_core::pool::LARGE_STACK)
                    .spawn_scoped(s, worker);
                if spawned.is_err() && w == 0 {
                    // No thread at all: everything falls through to the serial pass.
                    abort.store(true, Ordering::Relaxed);
                }
            }
        });
        fell_back = abort.load(Ordering::Relaxed);
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    // Pass 2: serial, for items not attempted or failed with a retryable error.
    let mut results = Vec::with_capacity(n);
    for (i, slot) in slots.into_iter().enumerate() {
        let prev = slot.into_inner().unwrap_or_else(|e| e.into_inner());
        let mut cur = match prev {
            Some(Ok(r)) => {
                results.push(Ok(r));
                continue;
            }
            Some(Err(e)) if !e.retryable() => {
                results.push(Err(e));
                continue;
            }
            Some(Err(e)) => Some(e),
            None => None,
        };
        // Items pass 1 already tried get SERIAL_ATTEMPTS more tries; untried items the same.
        let mut out = None;
        for attempt in 0..SERIAL_ATTEMPTS {
            cancel.checkpoint()?;
            if attempt > 0 || cur.is_some() {
                std::thread::sleep(Duration::from_millis(25 * (attempt as u64 + 1)));
            }
            match call(i) {
                Ok(r) => {
                    out = Some(Ok(r));
                    break;
                }
                Err(e) => {
                    let retry = e.retryable();
                    cur = Some(e);
                    if !retry {
                        break;
                    }
                }
            }
        }
        results.push(out.unwrap_or_else(|| Err(cur.unwrap_or(ReadError::Panicked))));
    }
    Ok(Adaptive { results, fell_back })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_work_runs_once_in_parallel() {
        let calls = AtomicUsize::new(0);
        let items: Vec<u32> = (0..100).collect();
        let r = run(&items, 4, &CancelToken::new(), |v| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ReadError>(*v * 2)
        })
        .unwrap();
        assert!(!r.fell_back);
        assert_eq!(calls.load(Ordering::SeqCst), 100);
        assert!(r.results.iter().enumerate().all(|(i, x)| matches!(x, Ok(v) if *v == i as u32 * 2)));
    }

    #[test]
    fn falls_back_to_serial_when_concurrent_reads_fail() {
        // Simulates a GVFS mount: a read fails whenever another one is in flight.
        let in_flight = AtomicUsize::new(0);
        let items: Vec<u32> = (0..60).collect();
        let r = run(&items, 4, &CancelToken::new(), |v| {
            let before = in_flight.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(2));
            in_flight.fetch_sub(1, Ordering::SeqCst);
            if before > 0 {
                Err(ReadError::Io(io::Error::other("EINVAL")))
            } else {
                Ok(*v)
            }
        })
        .unwrap();
        assert!(r.results.iter().all(|x| x.is_ok()), "every item succeeds serially");
    }

    #[test]
    fn permanent_failures_are_reported_not_retried() {
        let calls = AtomicUsize::new(0);
        let items = [0u32, 1, 2];
        let r = run(&items, 1, &CancelToken::new(), |v| -> std::result::Result<(), ReadError> {
            calls.fetch_add(1, Ordering::SeqCst);
            match v {
                0 => Err(ReadError::Io(io::Error::from(io::ErrorKind::NotFound))),
                1 => Err(ReadError::Io(io::Error::other("flaky"))),
                _ => panic!("hostile"),
            }
        })
        .unwrap();
        assert!(r.results.iter().all(|x| x.is_err()));
        // NotFound once, flaky twice, panic once.
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn cancel_is_an_error() {
        let t = CancelToken::new();
        t.cancel();
        assert!(matches!(run(&[1u8, 2], 2, &t, |_| Ok::<_, ReadError>(())), Err(Error::Cancelled)));
    }
}
