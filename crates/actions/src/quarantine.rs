//! Quarantine (CLN-ACT-04). **Owner: W2-E.** Layout:
//! `<quarantine dir>/<run id>/<entry id>/<original file name>`; the journal
//! entry's `retained_at` points at it, and a manifest
//! `<quarantine dir>/<run id>/<entry id>.manifest.json` records the original
//! path, size, mtime and content hash (so a quarantine folder is understandable
//! even without the journal). `purge` permanently deletes items older than
//! `days` and marks their entries `purged`.
//!
//! The move is a same-volume rename when possible, else copy + verify (size and
//! xxh64) + fsync + delete ([`crate::fsops::move_item`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mj_core::pathutil::is_within;
use mj_core::{EntryId, Error, JournalEntry, JournalState, Result, RunId, Timestamp};

use crate::fsops;
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

/// The manifest beside a slot.
pub fn manifest_path(quarantine_dir: &Path, run: RunId, entry: EntryId) -> PathBuf {
    quarantine_dir.join(run.0.to_string()).join(format!("{}.manifest.json", entry.0))
}

/// What the manifest records about one quarantined item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub entry_id: EntryId,
    pub run_id: RunId,
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub original: PathBuf,
    pub size: u64,
    pub mtime: Timestamp,
    /// xxh64 (hex) of the content, for files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xxh64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub folder: bool,
    pub quarantined_at: Timestamp,
}

/// Write a manifest (synced).
pub fn write_manifest(quarantine_dir: &Path, m: &Manifest) -> Result<()> {
    let p = manifest_path(quarantine_dir, m.run_id, m.entry_id);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let text = serde_json::to_vec_pretty(m).map_err(|e| Error::Internal(format!("manifest encode: {e}")))?;
    let tmp = p.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| Error::io(&tmp, e))?;
        f.write_all(&text).and_then(|_| f.sync_all()).map_err(|e| Error::io(&tmp, e))?;
    }
    std::fs::rename(&tmp, &p).map_err(|e| Error::io(&p, e))
}

/// Read a manifest.
pub fn read_manifest(quarantine_dir: &Path, run: RunId, entry: EntryId) -> Result<Manifest> {
    let p = manifest_path(quarantine_dir, run, entry);
    let data = std::fs::read(&p).map_err(|e| Error::io(&p, e))?;
    serde_json::from_slice(&data).map_err(|e| Error::parse("quarantine manifest", e.to_string()))
}

/// Remove an entry's manifest and its now-empty slot and run folders.
pub fn cleanup_slot(quarantine_dir: &Path, run: RunId, entry: EntryId) {
    let _ = std::fs::remove_file(manifest_path(quarantine_dir, run, entry));
    let entry_dir = quarantine_dir.join(run.0.to_string()).join(entry.0.to_string());
    let _ = std::fs::remove_dir(&entry_dir);
    let _ = std::fs::remove_dir(quarantine_dir.join(run.0.to_string()));
}

/// True when `retained` is a slot inside `quarantine_dir` with no link on the way.
pub fn is_quarantined_path(quarantine_dir: &Path, retained: &Path) -> bool {
    if !is_within(quarantine_dir, retained)
        || mj_core::pathutil::path_key(quarantine_dir) == mj_core::pathutil::path_key(retained)
    {
        return false;
    }
    let depth = quarantine_dir.components().count();
    let mut cur = retained.parent();
    while let Some(p) = cur {
        if p.components().count() <= depth {
            break;
        }
        match std::fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => return false,
            _ => {}
        }
        cur = p.parent();
    }
    true
}

/// The quarantine's contents according to the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuarantineSummary {
    pub items: u64,
    pub bytes: u64,
    pub oldest: Option<Timestamp>,
}

/// Items currently held (entries `done` or `undo_failed` whose `retained_at` is in the quarantine).
pub fn summary(journal: &Journal, quarantine_dir: &Path) -> QuarantineSummary {
    let mut s = QuarantineSummary::default();
    // XC-4: counted in place under the journal lock; nothing is cloned.
    journal.for_each_entry(|e| {
        if is_held(quarantine_dir, e) {
            s.items += 1;
            s.bytes = s.bytes.saturating_add(e.bytes_before);
            s.oldest = Some(s.oldest.map_or(e.at, |o| o.min(e.at)));
        }
    });
    s
}

/// The entries [`summary`] counts (held items in their own quarantine slot).
pub fn held_entries(journal: &Journal, quarantine_dir: &Path) -> Vec<JournalEntry> {
    held(journal, quarantine_dir)
}

fn held(journal: &Journal, quarantine_dir: &Path) -> Vec<JournalEntry> {
    // XC-4: only held entries are cloned, not the whole history.
    journal.entries_where(|e| is_held(quarantine_dir, e))
}

fn is_held(quarantine_dir: &Path, e: &JournalEntry) -> bool {
    // G-13: an item whose undo was refused (`undo_failed`, retryable) is still in its
    // slot: it is counted and purged like any other held item.
    matches!(e.state, JournalState::Done | JournalState::UndoFailed)
        // Undo records copy their source's `retained_at`; they hold nothing.
        && e.undo_of.is_none()
        // F-05: only the slot the quarantine itself assigned to this entry is held (and
        // purged); anything else that happens to lie below the folder (a backup folder
        // pointed there by mistake) is the user's and never purged.
        && e.retained_at.as_deref().is_some_and(|r| is_own_slot(quarantine_dir, e, r))
}

fn is_own_slot(quarantine_dir: &Path, e: &JournalEntry, retained: &Path) -> bool {
    let Some(name) = e.original.file_name() else { return false };
    is_within(quarantine_dir, retained)
        && mj_core::pathutil::path_key(&slot(quarantine_dir, e.run_id, e.id, name))
            == mj_core::pathutil::path_key(retained)
}

/// Purge items older than `days` (an explicit "empty the quarantine" with that age).
/// Returns bytes freed. See [`purge_expired`] for the automatic purge.
pub fn purge(journal: &Journal, quarantine_dir: &Path, now: Timestamp, days: u32) -> Result<u64> {
    purge_by(journal, quarantine_dir, now, &|_| days)
}

/// The automatic purge (F-15): each item expires after its own retention (the
/// `purge_after_days` of the step that quarantined it), else `default_days`.
pub fn purge_expired(journal: &Journal, quarantine_dir: &Path, now: Timestamp, default_days: u32) -> Result<u64> {
    purge_by(journal, quarantine_dir, now, &|e| e.retention_days.unwrap_or(default_days))
}

/// An item expires `days_of(entry)` after it was quarantined (its `done` record), compared
/// with the caller's `now`. Each purged entry gets a `purged` record (no longer undoable).
/// Items that cannot be removed stay held and are retried by the next purge.
fn purge_by(
    journal: &Journal,
    quarantine_dir: &Path,
    now: Timestamp,
    days_of: &dyn Fn(&JournalEntry) -> u32,
) -> Result<u64> {
    let mut freed = 0u64;
    for e in held(journal, quarantine_dir) {
        if e.at.add_days(i64::from(days_of(&e))) > now {
            continue;
        }
        let Some(retained) = e.retained_at.clone() else { continue };
        if !is_quarantined_path(quarantine_dir, &retained) {
            continue;
        }
        let size = if fsops::exists(&retained) { fsops::tree_size(&retained) } else { 0 };
        if fsops::exists(&retained) && fsops::remove_item(&retained).is_err() {
            continue;
        }
        freed = freed.saturating_add(size);
        let mut rec = e.clone();
        rec.state = JournalState::Purged;
        rec.at = now;
        rec.reversible = false;
        journal.append(&rec)?;
        cleanup_slot(quarantine_dir, e.run_id, e.id);
    }
    Ok(freed)
}
