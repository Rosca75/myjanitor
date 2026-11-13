//! In-house parallelism with `std::thread` + channels (no `rayon`).
//!
//! - [`parallel_map`]: scoped, index-aligned fan-out for many small items
//!   (stat, hash, decode). An atomic counter hands out indexes; each worker
//!   writes into its own result slot, so no lock is held per item. Checks the
//!   cancel token before every item.
//! - [`ThreadPool`]: persistent workers fed by a channel, for background jobs
//!   whose lifetime outlives a call. Workers run with [`LARGE_STACK`] and catch
//!   panics so one bad file cannot kill a worker.
//! - [`spawn_large`]: one dedicated thread with a large stack for a whole
//!   long-running operation (scan, apply), as the lessons file requires.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::cancel::CancelToken;
use crate::lock;

/// Stack size for worker threads: deep recursion on hostile input must not overflow.
pub const LARGE_STACK: usize = 16 * 1024 * 1024;

/// Number of logical CPUs (at least 1).
pub fn default_workers() -> usize {
    thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// Spawn a named thread with [`LARGE_STACK`].
pub fn spawn_large<F, T>(name: &str, f: F) -> std::io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    thread::Builder::new().name(name.to_string()).stack_size(LARGE_STACK).spawn(f)
}

/// Map `f` over `items` on `workers` threads. Result `i` belongs to item `i`;
/// it is `None` when the token was cancelled before the item started or when
/// `f` panicked on it. `workers == 0` means [`default_workers`].
pub fn parallel_map<T, R, F>(items: &[T], workers: usize, cancel: &CancelToken, f: F) -> Vec<Option<R>>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &T) -> R + Sync,
{
    let n = items.len();
    let workers = if workers == 0 { default_workers() } else { workers }.clamp(1, n.max(1));
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let run = || loop {
        if cancel.checkpoint().is_err() {
            return;
        }
        let i = next.fetch_add(1, Ordering::Relaxed);
        if i >= n {
            return;
        }
        if let Ok(r) = catch_unwind(AssertUnwindSafe(|| f(i, &items[i]))) {
            *lock(&slots[i]) = Some(r);
        }
    };
    if workers == 1 {
        run();
    } else {
        thread::scope(|s| {
            for w in 0..workers {
                let _ = thread::Builder::new().name(format!("mj-par-{w}")).stack_size(LARGE_STACK).spawn_scoped(s, run);
            }
        });
    }
    slots.into_iter().map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner())).collect()
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A fixed set of worker threads fed by a channel.
pub struct ThreadPool {
    sender: Option<mpsc::Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl ThreadPool {
    /// Start `threads` workers (0 = [`default_workers`]) named `name-N`.
    pub fn new(name: &str, threads: usize) -> std::io::Result<ThreadPool> {
        let threads = if threads == 0 { default_workers() } else { threads };
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let mut workers = Vec::with_capacity(threads);
        for i in 0..threads {
            let rx = Arc::clone(&rx);
            workers.push(thread::Builder::new().name(format!("{name}-{i}")).stack_size(LARGE_STACK).spawn(
                move || loop {
                    let job = lock(&rx).recv();
                    match job {
                        Ok(job) => {
                            let _ = catch_unwind(AssertUnwindSafe(job));
                        }
                        Err(_) => return,
                    }
                },
            )?);
        }
        Ok(ThreadPool { sender: Some(tx), workers })
    }

    /// Number of workers.
    pub fn threads(&self) -> usize {
        self.workers.len()
    }

    /// Queue a job. Returns false if the pool is shutting down.
    pub fn execute<F: FnOnce() + Send + 'static>(&self, f: F) -> bool {
        match &self.sender {
            Some(tx) => tx.send(Box::new(f)).is_ok(),
            None => false,
        }
    }
}

impl Drop for ThreadPool {
    /// Finish queued jobs, then join every worker.
    fn drop(&mut self) {
        self.sender.take();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn parallel_map_is_index_aligned() {
        let items: Vec<u64> = (0..1000).collect();
        let out = parallel_map(&items, 4, &CancelToken::new(), |i, v| (i as u64) * 2 + v);
        assert!(out.iter().enumerate().all(|(i, r)| *r == Some(i as u64 * 3)));
    }

    #[test]
    fn parallel_map_survives_panics_and_cancel() {
        let items: Vec<u32> = (0..10).collect();
        let out = parallel_map(&items, 2, &CancelToken::new(), |_, v| {
            assert!(*v != 5, "boom");
            *v
        });
        assert_eq!(out[5], None);
        assert_eq!(out[6], Some(6));
        let t = CancelToken::new();
        t.cancel();
        let out = parallel_map(&items, 2, &t, |_, v| *v);
        assert!(out.iter().all(Option::is_none));
    }

    #[test]
    fn pool_runs_all_jobs_before_drop_returns() {
        let sum = Arc::new(AtomicU64::new(0));
        {
            let pool = ThreadPool::new("t", 3).unwrap();
            for i in 1..=100 {
                let s = sum.clone();
                assert!(pool.execute(move || {
                    s.fetch_add(i, Ordering::SeqCst);
                }));
            }
            pool.execute(|| panic!("ignored"));
        }
        assert_eq!(sum.load(Ordering::SeqCst), 5050);
    }
}
