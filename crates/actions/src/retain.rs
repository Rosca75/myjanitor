//! Retaining and restoring originals: the Recycle Bin / Trash, the quarantine and
//! backup folders. Shared by the action engine (trash, quarantine, optimise and
//! archive originals) and by undo. Owner: W2-E.
//!
//! **Windows decision (docs/decisions/W2-E.md, D-2E01):** the documented Recycle
//! Bin API (`SHFileOperationW`) does not say where an item went, so an exact
//! restore from History is not possible through it. On Windows every
//! "to Recycle Bin" request is therefore carried out as a quarantine move by
//! default ([`TrashRoute::Quarantine`]), journalled as `quarantine`, so History
//! can always restore it. [`TrashRoute::Platform`] (the real Recycle Bin, with a
//! best-effort restore) stays available to callers that opt in.

use std::path::{Path, PathBuf};

use mj_core::pathutil::is_within;
use mj_core::{ActionKind, Error, JournalEntry, OriginalsPolicy, Result, Timestamp};
use mj_platform::Platform;

use crate::fsops;
use crate::quarantine::{self, Manifest};

/// Where "move to Recycle Bin / Trash" really goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashRoute {
    /// The platform trash (freedesktop Trash on Linux, Recycle Bin on Windows).
    Platform,
    /// MyJanitor's quarantine (exactly restorable from History, auto-purged).
    Quarantine,
}

impl TrashRoute {
    /// The default of this OS: the platform trash on Linux, the quarantine on Windows.
    pub fn default_for_os() -> TrashRoute {
        if cfg!(windows) {
            TrashRoute::Quarantine
        } else {
            TrashRoute::Platform
        }
    }
}

/// Paths a retention will use, decided before the journal `intent` is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetainPlan {
    Trash,
    Quarantine { slot: PathBuf },
    Backup { dest: PathBuf, new_dirs: Vec<PathBuf> },
    Delete,
}

/// Decide where `path` goes for `policy`. `rel_root` is the scan root (backup folders keep
/// the structure below it).
pub fn plan(
    policy: &OriginalsPolicy,
    route: TrashRoute,
    quarantine_dir: &Path,
    entry: &JournalEntry,
    path: &Path,
    rel_root: &Path,
) -> Result<RetainPlan> {
    let name = path.file_name().ok_or_else(|| Error::InvalidInput(format!("no file name: {}", path.display())))?;
    let qslot = || quarantine::slot(quarantine_dir, entry.run_id, entry.id, name);
    Ok(match policy {
        OriginalsPolicy::RecycleBin if route == TrashRoute::Platform => RetainPlan::Trash,
        OriginalsPolicy::RecycleBin | OriginalsPolicy::Quarantine => {
            if !quarantine_dir.is_absolute() {
                return Err(Error::InvalidInput("the quarantine folder must be an absolute path".into()));
            }
            if is_within(path, quarantine_dir) {
                return Err(Error::InvalidInput(format!("the quarantine folder is inside {}", path.display())));
            }
            RetainPlan::Quarantine { slot: qslot() }
        }
        OriginalsPolicy::BackupFolder { folder } => {
            let rel = if !rel_root.as_os_str().is_empty() && is_within(rel_root, path) {
                path.strip_prefix(rel_root).map(Path::to_path_buf).unwrap_or_else(|_| PathBuf::from(name))
            } else {
                PathBuf::from(name)
            };
            let dest = fsops::unique_path(&folder.join(rel));
            let mut new_dirs = Vec::new();
            let mut cur = dest.parent();
            while let Some(p) = cur {
                if fsops::exists(p) {
                    break;
                }
                new_dirs.push(p.to_path_buf());
                cur = p.parent();
            }
            new_dirs.reverse();
            RetainPlan::Backup { dest, new_dirs }
        }
        OriginalsPolicy::Delete => RetainPlan::Delete,
    })
}

/// Record the plan in the entry (before the intent is written).
pub fn note_plan(entry: &mut JournalEntry, plan: &RetainPlan) {
    match plan {
        RetainPlan::Quarantine { slot } => entry.retained_at = Some(slot.clone()),
        RetainPlan::Backup { dest, .. } => entry.retained_at = Some(dest.clone()),
        RetainPlan::Trash | RetainPlan::Delete => {}
    }
}

/// Carry out a retention plan: move `path` away (it no longer exists at its place on
/// success). Updates `entry` (`trash`, `retained_at`, `reversible`).
pub fn retain(
    platform: &dyn Platform,
    quarantine_dir: &Path,
    entry: &mut JournalEntry,
    path: &Path,
    plan: &RetainPlan,
    now: Timestamp,
) -> Result<()> {
    match plan {
        RetainPlan::Trash => {
            let ticket = platform.trash(path)?;
            entry.trash = Some(ticket);
        }
        RetainPlan::Quarantine { slot } => {
            let parent = slot.parent().ok_or_else(|| Error::Internal("quarantine slot without parent".into()))?;
            fsops::create_dirs(parent)?;
            let md = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
            let hash = if md.is_file() { fsops::hash_file(path).ok() } else { None };
            let m = Manifest {
                entry_id: entry.id,
                run_id: entry.run_id,
                original: path.to_path_buf(),
                size: if md.is_file() { md.len() } else { fsops::tree_size(path) },
                mtime: md.modified().map(Timestamp::from_system_time).unwrap_or_default(),
                xxh64: hash.as_ref().map(|h| fsops::hex64(h.xxh64)),
                sha256: hash.map(|h| h.sha256),
                folder: md.is_dir(),
                quarantined_at: now,
            };
            quarantine::write_manifest(quarantine_dir, &m)?;
            match fsops::move_item(path, slot) {
                Err(e) => {
                    quarantine::cleanup_slot(quarantine_dir, entry.run_id, entry.id);
                    return Err(e);
                }
                // F-09: the slot holds the complete verified copy: the entry stays
                // restorable (manifest kept) and names what is left at the source.
                Ok(fsops::MoveHow::CopiedSourceLeft { left }) => {
                    entry.error = Some(fsops::left_behind_note(path, slot, left).info());
                }
                Ok(_) => {}
            }
            entry.retained_at = Some(slot.clone());
        }
        RetainPlan::Backup { dest, .. } => {
            if let Some(parent) = dest.parent() {
                fsops::create_dirs(parent)?;
            }
            if let fsops::MoveHow::CopiedSourceLeft { left } = fsops::move_item(path, dest)? {
                entry.error = Some(fsops::left_behind_note(path, dest, left).info());
            }
            entry.retained_at = Some(dest.clone());
        }
        RetainPlan::Delete => {
            fsops::remove_item(path)?;
            entry.reversible = false;
        }
    }
    Ok(())
}

/// Put a retained original back at `original` (never overwriting). Used by undo and by
/// the engine's own rollback.
pub fn restore(platform: &dyn Platform, quarantine_dir: &Path, entry: &JournalEntry, original: &Path) -> Result<()> {
    if fsops::exists(original) {
        return Err(Error::Conflict(format!("{} exists again; nothing was overwritten", original.display())));
    }
    if let Some(retained) = &entry.retained_at {
        if !fsops::exists(retained) {
            return Err(Error::NotFound(retained.clone()));
        }
        verify_retained(entry, retained)?;
        if let Some(parent) = original.parent() {
            fsops::create_dirs(parent)?;
        }
        fsops::move_item(retained, original)?;
        if is_within(quarantine_dir, retained) {
            quarantine::cleanup_slot(quarantine_dir, entry.run_id, entry.id);
        }
        return Ok(());
    }
    if let Some(ticket) = &entry.trash {
        if let Some(parent) = original.parent() {
            fsops::create_dirs(parent)?;
        }
        if let Some(loc) = ticket.location.as_deref().filter(|l| fsops::exists(l)) {
            verify_retained(entry, loc)?;
        }
        return platform.restore_trashed(ticket, original);
    }
    Err(Error::InvalidInput(format!("nothing retained for {}", original.display())))
}

/// F-18: what is about to be moved back must be what was moved away: the recorded
/// SHA-256 when there is one, else the recorded size (files; folders carry no hash).
/// A mismatch (the user replaced the file at a Move destination, a trash name reused)
/// is a `conflict` and nothing is restored.
fn verify_retained(entry: &JournalEntry, retained: &Path) -> Result<()> {
    let Ok(md) = std::fs::symlink_metadata(retained) else { return Ok(()) };
    if !md.is_file() || entry.target != mj_core::TargetKind::File {
        return Ok(());
    }
    let same = match &entry.original_sha256 {
        Some(want) => fsops::hash_file(retained)?.sha256 == *want,
        None => md.len() == entry.original_stamp.size,
    };
    if !same {
        return Err(Error::Conflict(format!(
            "{} is not the item that was moved there (its content differs); nothing was restored",
            retained.display()
        )));
    }
    Ok(())
}

/// The journal kind a trash request is recorded as under `route`.
pub fn trash_kind(route: TrashRoute) -> ActionKind {
    match route {
        TrashRoute::Platform => ActionKind::Trash,
        TrashRoute::Quarantine => ActionKind::Quarantine,
    }
}
