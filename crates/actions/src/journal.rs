//! The journal store. **Owner: W2-E.** Format (frozen, ARCHITECTURE.md §7):
//! `<data dir>/journal/entries-YYYY-MM.jsonl` (one `JournalEntry` snapshot per
//! line, last line per id wins, torn last line ignored, `sync_data` after every
//! `intent`) and `<data dir>/journal/runs.jsonl` (`RunRecord`s, same rules).
//!
//! Wave 1: the in-memory store is real (tests, dry runs); the file store is not.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use mj_core::{lock, EntryId, Error, JournalEntry, Result, RunId, RunRecord, Timestamp};

/// Journal backend.
#[derive(Debug)]
enum Backend {
    Memory,
}

/// The undo journal.
#[derive(Debug)]
pub struct Journal {
    backend: Backend,
    entries: Mutex<BTreeMap<EntryId, JournalEntry>>,
    runs: Mutex<BTreeMap<RunId, RunRecord>>,
}

impl Journal {
    /// Open (or create) the journal under `dir`, replaying it.
    pub fn open(dir: &Path) -> Result<Journal> {
        let _ = dir;
        Err(Error::not_implemented("persistent journal (W2-E)"))
    }

    /// A journal that is never persisted (tests, dry runs).
    pub fn in_memory() -> Journal {
        Journal { backend: Backend::Memory, entries: Mutex::new(BTreeMap::new()), runs: Mutex::new(BTreeMap::new()) }
    }

    /// True when backed by files.
    pub fn is_persistent(&self) -> bool {
        !matches!(self.backend, Backend::Memory)
    }

    /// Append an entry snapshot (durable before returning for file backends).
    pub fn append(&self, entry: &JournalEntry) -> Result<()> {
        lock(&self.entries).insert(entry.id, entry.clone());
        Ok(())
    }

    /// Append or update a run header.
    pub fn put_run(&self, run: &RunRecord) -> Result<()> {
        lock(&self.runs).insert(run.run_id, run.clone());
        Ok(())
    }

    /// Latest state of one entry.
    pub fn entry(&self, id: EntryId) -> Option<JournalEntry> {
        lock(&self.entries).get(&id).cloned()
    }

    /// Latest state of every entry of a run, in id order.
    pub fn entries_of(&self, run: RunId) -> Vec<JournalEntry> {
        lock(&self.entries).values().filter(|e| e.run_id == run).cloned().collect()
    }

    /// Run headers, newest first.
    pub fn runs(&self) -> Vec<RunRecord> {
        let mut v: Vec<RunRecord> = lock(&self.runs).values().cloned().collect();
        v.sort_by(|a, b| b.started.cmp(&a.started).then(b.run_id.cmp(&a.run_id)));
        v
    }

    /// A new run id: milliseconds of `now`, bumped past every existing id.
    pub fn next_run_id(&self, now: Timestamp) -> RunId {
        let max = lock(&self.runs).keys().next_back().map(|r| r.0).unwrap_or(0);
        RunId((now.millis().max(0) as u64).max(max + 1))
    }
}
