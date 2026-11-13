//! Cancellation and pause token shared between a job and its controller.
//!
//! Workers call [`CancelToken::checkpoint`] between items (never mid-write):
//! it blocks while the token is paused and returns `Err(Error::Cancelled)`
//! once it is cancelled. Cloning shares the same state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::error::{Error, Result};
use crate::lock;

#[derive(Debug, Default)]
struct Inner {
    cancelled: AtomicBool,
    paused: Mutex<bool>,
    cv: Condvar,
}

/// Shared cancel + pause flag.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<Inner>);

impl CancelToken {
    /// A fresh, running token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation (also wakes paused workers so they can exit).
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        let _g = lock(&self.0.paused);
        self.0.cv.notify_all();
    }

    /// True once cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Pause: workers block at their next checkpoint.
    pub fn pause(&self) {
        *lock(&self.0.paused) = true;
    }

    /// Resume paused workers.
    pub fn resume(&self) {
        *lock(&self.0.paused) = false;
        self.0.cv.notify_all();
    }

    /// True while paused.
    pub fn is_paused(&self) -> bool {
        *lock(&self.0.paused)
    }

    /// Block while paused; `Err(Cancelled)` once cancelled.
    pub fn checkpoint(&self) -> Result<()> {
        if self.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let mut paused = lock(&self.0.paused);
        while *paused && !self.is_cancelled() {
            paused = self.0.cv.wait(paused).unwrap_or_else(|e| e.into_inner());
        }
        drop(paused);
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cancel_and_pause() {
        let t = CancelToken::new();
        assert!(t.checkpoint().is_ok());
        t.pause();
        let t2 = t.clone();
        let h = std::thread::spawn(move || t2.checkpoint());
        std::thread::sleep(Duration::from_millis(30));
        assert!(!h.is_finished());
        t.resume();
        assert!(h.join().unwrap().is_ok());
        t.pause();
        let t3 = t.clone();
        let h = std::thread::spawn(move || t3.checkpoint());
        std::thread::sleep(Duration::from_millis(10));
        t.cancel();
        assert_eq!(h.join().unwrap(), Err(Error::Cancelled));
        assert!(t.is_cancelled());
    }
}
