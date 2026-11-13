//! Quarantine (CLN-ACT-04). **Owner: W2-E.** Layout:
//! `<quarantine dir>/<run id>/<entry id>/<original file name>`; the journal
//! entry's `retained_at` points at it. `purge` permanently deletes items older
//! than `Settings::general::quarantine_days` and marks their entries `purged`.

use std::path::Path;

use mj_core::{Error, Result, Timestamp};

use crate::journal::Journal;

/// Where an entry's item is stored.
pub fn slot(
    quarantine_dir: &Path,
    run: mj_core::RunId,
    entry: mj_core::EntryId,
    name: &std::ffi::OsStr,
) -> std::path::PathBuf {
    quarantine_dir.join(run.0.to_string()).join(entry.0.to_string()).join(name)
}

/// Purge expired items. Returns bytes freed.
pub fn purge(journal: &Journal, quarantine_dir: &Path, now: Timestamp, days: u32) -> Result<u64> {
    let _ = (journal, quarantine_dir, now, days);
    Err(Error::not_implemented("quarantine purge (W2-E)"))
}
