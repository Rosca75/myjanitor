//! One read per file and stage across concurrent detectors (DUP-7).
//!
//! The engine runs the content detectors (exact duplicates, similar images,
//! attachment copies, duplicate fonts) at the same time, and each runs the exact
//! lane over its own files. A detector reads the cache once at the start of a
//! stage, so two of them starting together both missed it and both read the
//! same files: 2-3x the I/O on a first scan. [`shared`] makes the read of a
//! (file, stamp, stage) single-flight within the process: the first caller
//! reads and stores the result in the fingerprint cache before it lets the
//! others go; a caller that finds the read in flight waits for it and takes the
//! result from the cache. When the first read fails, a waiter reads for itself
//! (and reports its own error), so a failure is never shared silently.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use mj_core::pathutil::path_key;
use mj_core::{lock, CancelToken, FileEntry, FileStamp, FingerprintStore, Fingerprints};

use super::adaptive::ReadError;

/// What a read computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Stage {
    Partial,
    Full,
}

type Key = (String, FileStamp, Stage);

#[derive(Default)]
struct Flight {
    done: Mutex<bool>,
    cv: Condvar,
}

fn registry() -> &'static Mutex<HashMap<Key, Arc<Flight>>> {
    static R: OnceLock<Mutex<HashMap<Key, Arc<Flight>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Ends a flight, also when the read panics: removed from the registry, then waiters woken.
struct Landing<'a>(&'a Key, Arc<Flight>);

impl Drop for Landing<'_> {
    fn drop(&mut self) {
        lock(registry()).remove(self.0);
        *lock(&self.1.done) = true;
        self.1.cv.notify_all();
    }
}

/// The result of `stage` for `e`: from the cache (`cached`), from a read in flight
/// elsewhere, or from `read`, whose fingerprints are stored before anyone waiting
/// is let go.
pub(crate) fn shared<R>(
    store: &dyn FingerprintStore,
    cancel: &CancelToken,
    e: &FileEntry,
    stage: Stage,
    cached: impl Fn(&Fingerprints) -> Option<R>,
    read: impl FnOnce() -> Result<(R, Fingerprints), ReadError>,
) -> Result<R, ReadError> {
    let key: Key = (path_key(&e.path), e.stamp(), stage);
    let mut waited = false;
    loop {
        if let Some(r) = store.get(&e.path, e.stamp()).as_ref().and_then(&cached) {
            return Ok(r);
        }
        let inflight = {
            let mut m = lock(registry());
            match m.get(&key) {
                // After one wait, a failed or uncached read is redone here, unregistered.
                Some(f) if !waited => Some(f.clone()),
                Some(_) => None,
                None => {
                    let f = Arc::new(Flight::default());
                    m.insert(key.clone(), f.clone());
                    drop(m);
                    let _landing = Landing(&key, f);
                    let (r, fp) = read()?;
                    store.put(&e.path, e.stamp(), fp);
                    return Ok(r);
                }
            }
        };
        match inflight {
            Some(f) => {
                let mut done = lock(&f.done);
                while !*done {
                    if cancel.is_cancelled() {
                        return Err(ReadError::Io(std::io::Error::other("cancelled")));
                    }
                    done =
                        f.cv.wait_timeout(done, Duration::from_millis(50))
                            .map(|r| r.0)
                            .unwrap_or_else(|e| e.into_inner().0);
                }
                waited = true;
            }
            None => {
                let (r, fp) = read()?;
                store.put(&e.path, e.stamp(), fp);
                return Ok(r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::{EntryKind, MemoryFingerprintStore};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn concurrent_callers_read_once_and_failures_are_not_shared() {
        let store = MemoryFingerprintStore::new();
        let cancel = CancelToken::new();
        let e = FileEntry::new("/flight/a.bin", EntryKind::File, 10);
        let reads = AtomicUsize::new(0);
        let barrier = std::sync::Barrier::new(4);
        let got: Vec<u64> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..4)
                .map(|_| {
                    s.spawn(|| {
                        barrier.wait();
                        shared(
                            &store,
                            &cancel,
                            &e,
                            Stage::Partial,
                            |f| f.partial,
                            || {
                                reads.fetch_add(1, Ordering::SeqCst);
                                std::thread::sleep(Duration::from_millis(100));
                                Ok((7, Fingerprints { partial: Some(7), ..Default::default() }))
                            },
                        )
                        .unwrap()
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(got, [7, 7, 7, 7]);
        assert_eq!(reads.load(Ordering::SeqCst), 1);

        // A failed read: each caller reads for itself and gets its own error.
        let e = FileEntry::new("/flight/b.bin", EntryKind::File, 10);
        let reads = AtomicUsize::new(0);
        let errs = std::thread::scope(|s| {
            let hs: Vec<_> = (0..3)
                .map(|_| {
                    s.spawn(|| {
                        shared(
                            &store,
                            &cancel,
                            &e,
                            Stage::Full,
                            |f| f.full,
                            || {
                                reads.fetch_add(1, Ordering::SeqCst);
                                std::thread::sleep(Duration::from_millis(20));
                                Err::<(u64, Fingerprints), _>(ReadError::Changed)
                            },
                        )
                        .is_err()
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).filter(|&x| x).count()
        });
        assert_eq!(errs, 3);
        assert_eq!(reads.load(Ordering::SeqCst), 3);
        assert!(lock(registry()).keys().all(|k| !k.0.contains("flight")), "flights land");
    }
}
