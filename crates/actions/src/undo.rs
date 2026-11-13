//! Undo / restore and crash recovery. **Owner: W2-E.**
//!
//! Contract: `undo` restores each entry to its `original` path (failing, not
//! overwriting, when something now exists there), removes files the action
//! `created`, writes `undone` / `undo_failed` records, and runs as its own run
//! (`RunKind::Undo`, `undo_of`). Entries are undone in reverse order.
//! `recover` settles `intent` records left by a crash at start-up.
//!
//! How each action is undone:
//!
//! | action | undo |
//! |---|---|
//! | trash (Linux) | read the `.trashinfo` back, rename the item to its original path |
//! | trash (Windows, opt-in route) | best effort through the `$I` records (see `mj_platform::action::trash`) |
//! | quarantine, move | move `retained_at` back (rename, or copy + verify + delete across volumes); remove folders the move created, if empty |
//! | copy | remove the copy if it still matches the original (hash / tree comparison), then the folders it created |
//! | hardlink, reflink | replace the link with an independent copy whose content hash equals the journalled one, mtime restored |
//! | dehydrate | hydrate |
//! | fs_compress | decompress |
//! | optimise, archive | set the output aside, restore the original, then delete the output (only if its size is still what was written) |
//! | permanent delete, fs_decompress | not reversible |
//!
//! Every undo writes its own entry in the undo run (`intent` before acting, then
//! `done` / `failed`), so an interrupted undo is settled by [`recover`] too.
//!
//! **Recovery never deletes or moves user data.** It inspects both locations and
//! records what it finds (`done` when the action completed, `failed` otherwise,
//! with a note naming any leftover). The one exception is removing a temporary
//! link name or optimiser output MyJanitor itself created beside an original
//! that is still intact, which cannot lose data.

use std::path::{Path, PathBuf};

use mj_core::{
    ActionKind, EntryId, EntryKind, Error, ItemOutcome, ItemStatus, JobOutcome, JournalEntry, JournalState, Phase,
    Progress, Result, RunId, RunKind, RunRecord, Timestamp,
};

use crate::engine::{temp_beside, ExecContext};
use crate::fsops;
use crate::retain;

/// Undo the given entries (newest first).
pub fn undo(ctx: &ExecContext<'_>, entries: &[EntryId]) -> Result<JobOutcome> {
    let mut ids: Vec<EntryId> = entries.to_vec();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids.dedup();
    let mut out = JobOutcome::default();
    if ids.is_empty() {
        return Ok(out);
    }
    // Undo entries are recognised by their run's kind (crash recovery relies on it).
    match ctx.journal.run(ctx.run_id) {
        Some(r) if r.kind != RunKind::Undo => {
            return Err(Error::InvalidInput(format!("run {} is not an undo run", ctx.run_id.0)));
        }
        Some(_) => {}
        None => ctx.journal.put_run(&RunRecord {
            run_id: ctx.run_id,
            kind: RunKind::Undo,
            title: "Undo".into(),
            started: Timestamp::now(),
            finished: None,
            steps: Vec::new(),
            undo_of: None,
        })?,
    }
    let first = ctx.journal.reserve_entry_ids(ctx.run_id, ids.len() as u64)?;
    let total = ids.len() as u64;
    for (k, id) in ids.iter().enumerate() {
        if ctx.cancel.checkpoint().is_err() {
            break;
        }
        ctx.progress.report(&Progress::new(Phase::Undoing, k as u64, Some(total)));
        let uid = EntryId(first.0 + k as u64);
        let o = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| undo_one(ctx, *id, uid))) {
            Ok(r) => r?,
            Err(_) => {
                if let Some(u) = ctx.journal.entry(uid).filter(|u| u.state == JournalState::Intent) {
                    settle_intent(ctx, &u)?;
                }
                ItemOutcome {
                    path: ctx.journal.entry(*id).map(|e| e.original).unwrap_or_default(),
                    status: ItemStatus::Failed,
                    bytes_before: 0,
                    bytes_after: 0,
                    error: Some(Error::Internal("panic during undo".into()).info()),
                }
            }
        };
        match o.status {
            ItemStatus::Done => out.succeeded += 1,
            ItemStatus::Skipped => out.skipped += 1,
            ItemStatus::Failed => out.failed += 1,
        }
        if o.status != ItemStatus::Done && out.errors.len() < 50 {
            out.errors.push(o);
        }
    }
    ctx.progress.report(&Progress::new(Phase::Undoing, total, Some(total)));
    Ok(out)
}

/// Entries of `run` that can be undone now (state `done` or `undo_failed`, reversible,
/// within the deadline), newest first.
pub fn undoable_entries(journal: &crate::Journal, run: RunId, now: Timestamp) -> Vec<EntryId> {
    let mut v: Vec<EntryId> =
        journal.entries_of(run).into_iter().filter(|e| is_undoable(e, now)).map(|e| e.id).collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v
}

/// True when `e` can be undone at `now`.
pub fn is_undoable(e: &JournalEntry, now: Timestamp) -> bool {
    matches!(e.state, JournalState::Done | JournalState::UndoFailed)
        && e.reversible
        && e.undo_deadline.is_none_or(|d| now <= d)
}

/// Create and store the header of an undo run.
pub fn start_undo_run(journal: &crate::Journal, of: Option<RunId>, title: &str, now: Timestamp) -> Result<RunRecord> {
    let r = RunRecord {
        run_id: journal.next_run_id(now),
        kind: RunKind::Undo,
        title: title.to_string(),
        started: now,
        finished: None,
        steps: Vec::new(),
        undo_of: of,
    };
    journal.put_run(&r)?;
    Ok(r)
}

fn write(ctx: &ExecContext<'_>, e: &mut JournalEntry, state: JournalState, err: Option<&Error>) -> Result<()> {
    e.state = state;
    e.at = Timestamp::now();
    e.error = err.map(Error::info);
    ctx.journal.append(e)
}

fn undo_one(ctx: &ExecContext<'_>, id: EntryId, uid: EntryId) -> Result<ItemOutcome> {
    let Some(mut src) = ctx.journal.entry(id) else {
        let err = Error::InvalidInput(format!("no journal entry {}", id.0));
        return Ok(ItemOutcome {
            path: PathBuf::new(),
            status: ItemStatus::Skipped,
            bytes_before: 0,
            bytes_after: 0,
            error: Some(err.info()),
        });
    };
    let (path, before, after) = (src.original.clone(), src.bytes_after, src.bytes_before);
    let mk = |status, err: Option<&Error>| ItemOutcome {
        path: path.clone(),
        status,
        bytes_before: before,
        bytes_after: after,
        error: err.map(Error::info),
    };
    if !is_undoable(&src, Timestamp::now()) {
        let why = if !src.reversible {
            "this action cannot be undone".to_string()
        } else if !matches!(src.state, JournalState::Done | JournalState::UndoFailed) {
            format!("entry is {:?}, not done", src.state)
        } else {
            "the undo window has passed".to_string()
        };
        return Ok(mk(ItemStatus::Skipped, Some(&Error::InvalidInput(why))));
    }
    // The original location must be acceptable to the guard (lexically: inside the allowed
    // roots and not protected) and no existing ancestor may be a link: restoring through a
    // link that now points elsewhere would write outside the root.
    if let Err(err) = check_restore_target(ctx, &src.original) {
        return Ok(mk(ItemStatus::Skipped, Some(&err)));
    }
    let mut u = src.clone();
    u.id = uid;
    u.run_id = ctx.run_id;
    u.job_id = ctx.job_id;
    u.step_id = ctx.step_id;
    u.reversible = false;
    u.undo_deadline = None;
    write(ctx, &mut u, JournalState::Intent, None)?;
    match reverse(ctx, &src) {
        Ok(()) => {
            write(ctx, &mut src, JournalState::Undone, None)?;
            write(ctx, &mut u, JournalState::Done, None)?;
            Ok(mk(ItemStatus::Done, None))
        }
        Err(err) => {
            write(ctx, &mut src, JournalState::UndoFailed, Some(&err))?;
            write(ctx, &mut u, JournalState::Failed, Some(&err))?;
            Ok(mk(ItemStatus::Failed, Some(&err)))
        }
    }
}

/// Lexical guard checks plus "no existing ancestor between the root and the path is a link".
fn check_restore_target(ctx: &ExecContext<'_>, original: &Path) -> Result<()> {
    ctx.guard.check_lexical(original)?;
    let root = ctx.guard.root_of(original).map(Path::to_path_buf).unwrap_or_default();
    let mut cur = original.parent();
    while let Some(p) = cur {
        if p.components().count() <= root.components().count() {
            break;
        }
        if let Ok(md) = ctx.platform.metadata(p) {
            if md.kind == EntryKind::Link {
                return Err(Error::LinkNotFollowed(p.to_path_buf()));
            }
        }
        cur = p.parent();
    }
    Ok(())
}

/// Carry out the reverse operation of a `done` entry.
fn reverse(ctx: &ExecContext<'_>, e: &JournalEntry) -> Result<()> {
    let orig = &e.original;
    match e.action {
        ActionKind::Trash | ActionKind::Quarantine => retain::restore(ctx.platform, ctx.quarantine_dir, e, orig),
        ActionKind::Move => {
            retain::restore(ctx.platform, ctx.quarantine_dir, e, orig)?;
            fsops::remove_empty_dirs(&e.created);
            Ok(())
        }
        ActionKind::Copy => {
            let copy = e.created.first().ok_or_else(|| Error::InvalidInput("no copy recorded".into()))?;
            if !fsops::exists(copy) {
                fsops::remove_empty_dirs(&e.created[1..]);
                return Ok(());
            }
            verify_copy(e, copy)?;
            fsops::remove_item(copy)?;
            fsops::remove_empty_dirs(&e.created[1..]);
            Ok(())
        }
        ActionKind::Hardlink | ActionKind::Reflink => unlink_replacement(ctx, e),
        ActionKind::Dehydrate => ctx.platform.hydrate(orig),
        ActionKind::FsCompress => ctx.platform.fs_decompress(orig),
        ActionKind::Optimise | ActionKind::Archive => undo_output(ctx, e),
        ActionKind::PermanentDelete | ActionKind::FsDecompress | ActionKind::None => {
            Err(Error::InvalidInput("this action cannot be undone".into()))
        }
    }
}

/// A copy may only be removed while it still is what was copied.
fn verify_copy(e: &JournalEntry, copy: &Path) -> Result<()> {
    let md = std::fs::symlink_metadata(copy).map_err(|er| Error::io(copy, er))?;
    if md.is_file() {
        let h = fsops::hash_file(copy)?;
        let same = match &e.original_sha256 {
            Some(s) => *s == h.sha256,
            None => h.size == e.bytes_before,
        };
        if !same {
            return Err(Error::Conflict(format!("{} was modified after the copy; it was kept", copy.display())));
        }
        return Ok(());
    }
    if md.is_dir() {
        let a = tree_digest(copy)?;
        let b = tree_digest(&e.original)?;
        if a != b {
            return Err(Error::Conflict(format!(
                "{} no longer matches {}; it was kept",
                copy.display(),
                e.original.display()
            )));
        }
    }
    Ok(())
}

/// Sorted (relative path, kind, size, xxh64) of a tree, for comparisons.
pub fn tree_digest(root: &Path) -> Result<Vec<(String, u8, u64, u64)>> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((p, depth)) = stack.pop() {
        if depth > 4096 {
            return Err(Error::InvalidInput("folder nesting too deep".into()));
        }
        for e in std::fs::read_dir(&p).map_err(|er| Error::io(&p, er))? {
            let e = e.map_err(|er| Error::io(&p, er))?;
            let path = e.path();
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let ft = e.file_type().map_err(|er| Error::io(&path, er))?;
            if ft.is_symlink() {
                let t = std::fs::read_link(&path).map_err(|er| Error::io(&path, er))?;
                out.push((rel, 2, 0, mj_core::hash::xxh64(t.to_string_lossy().as_bytes(), 0)));
            } else if ft.is_dir() {
                out.push((rel, 1, 0, 0));
                stack.push((path, depth + 1));
            } else {
                let h = fsops::hash_file(&path)?;
                out.push((rel, 0, h.size, h.xxh64));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Replace a hard link / reflink with an independent copy of the same content.
fn unlink_replacement(ctx: &ExecContext<'_>, e: &JournalEntry) -> Result<()> {
    let orig = &e.original;
    let md = ctx.platform.metadata(orig)?;
    if md.kind != EntryKind::File {
        return Err(Error::Conflict(format!("{} is no longer a file; nothing was changed", orig.display())));
    }
    let temp = temp_beside(orig, e.id, false);
    let temp = if fsops::exists(&temp) { fsops::unique_path(&temp) } else { temp };
    let h = fsops::copy_file_verified(orig, &temp)?;
    let ok = match &e.original_sha256 {
        Some(s) => *s == h.sha256,
        None => h.size == e.original_stamp.size,
    };
    if !ok {
        let _ = std::fs::remove_file(&temp);
        return Err(Error::Conflict(format!("{} was modified after linking; nothing was changed", orig.display())));
    }
    if let Err(err) = fsops::set_mtime(&temp, e.original_stamp.mtime) {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    // Replacing the link itself (verified to hold the journalled content) is intended here.
    if let Err(er) = std::fs::rename(&temp, orig) {
        let _ = std::fs::remove_file(&temp);
        return Err(Error::io(orig, er));
    }
    Ok(())
}

/// Undo an optimisation or an archive: output aside, original back, output removed.
fn undo_output(ctx: &ExecContext<'_>, e: &JournalEntry) -> Result<()> {
    let orig = &e.original;
    let Some(output) = e.created.first().cloned() else {
        return Err(Error::InvalidInput("no output recorded".into()));
    };
    let retained = e.retained_at.is_some() || e.trash.is_some();
    // The output must still be what MyJanitor wrote (size check).
    if fsops::exists(&output) {
        let size = std::fs::symlink_metadata(&output).map(|m| m.len()).unwrap_or(0);
        if size != e.bytes_after && !(e.action == ActionKind::Archive && !retained) {
            return Err(Error::Conflict(format!(
                "{} changed after it was written; nothing was changed",
                output.display()
            )));
        }
    }
    if !retained {
        // Light copy, or an archive whose sources were kept: just remove the new file.
        if fsops::exists(&output) {
            fsops::remove_item(&output)?;
        }
        return Ok(());
    }
    let aside = if output == *orig && fsops::exists(&output) {
        let a = fsops::unique_path(&temp_beside(orig, e.id, false));
        mj_platform::action::rename_no_replace(&output, &a)?;
        Some(a)
    } else {
        None
    };
    match retain::restore(ctx.platform, ctx.quarantine_dir, e, orig) {
        Ok(()) => {
            if let Some(a) = aside {
                fsops::remove_item(&a)?;
            } else if output != *orig && fsops::exists(&output) {
                fsops::remove_item(&output)?;
            }
            Ok(())
        }
        Err(err) => {
            if let Some(a) = aside {
                let _ = mj_platform::action::rename_no_replace(&a, orig);
            }
            Err(err)
        }
    }
}

/// Settle `intent` records from an interrupted run. Returns the entries changed.
pub fn recover(ctx: &ExecContext<'_>) -> Result<Vec<EntryId>> {
    let mut changed = Vec::new();
    for e in ctx.journal.pending_intents() {
        settle_intent(ctx, &e)?;
        changed.push(e.id);
    }
    Ok(changed)
}

/// Inspect the filesystem for one dangling `intent` and record what happened.
pub fn settle_intent(ctx: &ExecContext<'_>, e: &JournalEntry) -> Result<()> {
    let is_undo_run = ctx.journal.run(e.run_id).is_some_and(|r| r.kind == RunKind::Undo);
    let mut rec = e.clone();
    if is_undo_run {
        return settle_undo_intent(ctx, rec);
    }
    let (state, note, reversible) = inspect_action(ctx, e);
    rec.reversible = reversible && state == JournalState::Done;
    if state == JournalState::Done {
        match e.action {
            ActionKind::Hardlink | ActionKind::Reflink => rec.created.clear(),
            ActionKind::Quarantine | ActionKind::Trash | ActionKind::Move | ActionKind::PermanentDelete => {
                rec.bytes_after = 0
            }
            _ => {}
        }
    } else {
        rec.undo_deadline = None;
    }
    let err = note.map(Error::Conflict);
    write(ctx, &mut rec, state, err.as_ref())
}

/// (state, note, still reversible) for an interrupted action.
fn inspect_action(ctx: &ExecContext<'_>, e: &JournalEntry) -> (JournalState, Option<String>, bool) {
    let orig_exists = fsops::exists(&e.original);
    let failed = |n: &str| (JournalState::Failed, Some(format!("interrupted: {n}")), false);
    match e.action {
        ActionKind::Trash | ActionKind::Quarantine | ActionKind::Move if e.retained_at.is_some() => {
            let r = e.retained_at.as_deref().unwrap_or(Path::new(""));
            match (orig_exists, fsops::exists(r)) {
                (true, false) => failed("nothing was moved"),
                (false, true) => (JournalState::Done, Some("completed before the interruption".into()), true),
                (true, true) => {
                    failed(&format!("the item is still in place; an incomplete copy may remain at {}", r.display()))
                }
                (false, false) => failed("the item was found at neither location"),
            }
        }
        ActionKind::Trash | ActionKind::Quarantine | ActionKind::Move => {
            if orig_exists {
                failed("nothing was moved")
            } else {
                (
                    JournalState::Done,
                    Some("moved to the Trash before the interruption; restore it from the Trash".into()),
                    false,
                )
            }
        }
        ActionKind::Copy => {
            let Some(copy) = e.created.first() else { return failed("no copy recorded") };
            if !fsops::exists(copy) {
                return failed("no copy was made");
            }
            if verify_copy(e, copy).is_ok() {
                (JournalState::Done, Some("completed before the interruption".into()), true)
            } else {
                failed(&format!("an incomplete copy may remain at {}", copy.display()))
            }
        }
        ActionKind::PermanentDelete => {
            if orig_exists {
                failed("the item still exists (it may be partly deleted if it is a folder)")
            } else {
                (JournalState::Done, None, false)
            }
        }
        ActionKind::Hardlink | ActionKind::Reflink => {
            let temp = e.created.first().cloned();
            let keeper = e.link_source.clone().unwrap_or_default();
            if let Some(t) = temp.filter(|t| fsops::exists(t)) {
                // The rename did not happen: the duplicate is intact. The temp name is a link
                // MyJanitor made to the keeper (or a clone of it), so removing it loses nothing.
                let ours = match e.action {
                    ActionKind::Hardlink => same_file(ctx, &t, &keeper),
                    _ => fsops::compare_files(&t, &keeper).ok().flatten().is_some(),
                };
                if ours {
                    let _ = std::fs::remove_file(&t);
                    return failed("the duplicate was not replaced");
                }
                return failed(&format!("a temporary file remains at {}", t.display()));
            }
            let linked = match e.action {
                ActionKind::Hardlink => same_file(ctx, &e.original, &keeper),
                _ => {
                    e.original_sha256
                        .as_ref()
                        .is_some_and(|s| fsops::hash_file(&e.original).map(|h| h.sha256 == *s).unwrap_or(false))
                        && fsops::stamp_of(&e.original).map(|s| s.mtime == e.original_stamp.mtime).unwrap_or(false)
                }
            };
            if linked {
                (JournalState::Done, Some("completed before the interruption".into()), true)
            } else {
                failed("the duplicate was not replaced")
            }
        }
        ActionKind::Dehydrate => {
            let online = ctx.platform.metadata(&e.original).map(|m| m.is_online_only()).unwrap_or(false);
            if online {
                (JournalState::Done, None, true)
            } else {
                failed("the file is still local")
            }
        }
        ActionKind::FsCompress => match ctx.platform.fs_compression(&e.original) {
            Ok(Some(_)) => (JournalState::Done, None, true),
            _ => failed("the file is not compressed"),
        },
        ActionKind::Optimise | ActionKind::Archive => {
            let intact = orig_exists
                && fsops::stamp_of(&e.original)
                    .map(|s| s.size == e.original_stamp.size && s.mtime == e.original_stamp.mtime)
                    .unwrap_or(false);
            if intact {
                // The original was never touched: a temp output beside it is ours to remove.
                if let Some(t) = e.created.first() {
                    let is_temp = t.file_name().is_some_and(|n| n.to_string_lossy().contains(".mj-"));
                    if is_temp && t != &e.original && fsops::exists(t) {
                        let _ = std::fs::remove_file(t);
                    }
                }
                return failed("the original was kept");
            }
            let where_ = e.retained_at.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "the Trash".into());
            failed(&format!(
                "check {} by hand; the original may be at {where_}, the output at {}",
                e.original.display(),
                e.created.first().map(|p| p.display().to_string()).unwrap_or_default()
            ))
        }
        ActionKind::FsDecompress | ActionKind::None => failed("state unknown"),
    }
}

fn same_file(ctx: &ExecContext<'_>, a: &Path, b: &Path) -> bool {
    match (ctx.platform.metadata(a), ctx.platform.metadata(b)) {
        (Ok(x), Ok(y)) => x.file_id.is_some() && x.file_id == y.file_id,
        _ => false,
    }
}

/// The entry an undo-run entry reverses: same original, action and retained locations.
fn find_undo_source(ctx: &ExecContext<'_>, u: &JournalEntry) -> Option<JournalEntry> {
    ctx.journal
        .all_entries()
        .into_iter()
        .filter(|s| s.id < u.id && s.run_id != u.run_id)
        .filter(|s| matches!(s.state, JournalState::Done | JournalState::UndoFailed))
        .filter(|s| s.original == u.original && s.action == u.action)
        .filter(|s| s.retained_at == u.retained_at && s.trash == u.trash && s.link_source == u.link_source)
        .max_by_key(|s| s.id)
}

fn settle_undo_intent(ctx: &ExecContext<'_>, mut u: JournalEntry) -> Result<()> {
    let Some(mut src) = find_undo_source(ctx, &u) else {
        let err = Error::Conflict("interrupted undo: the entry it reversed was not found".into());
        return write(ctx, &mut u, JournalState::Failed, Some(&err));
    };
    let orig = fsops::exists(&src.original);
    let happened = match src.action {
        ActionKind::Trash | ActionKind::Quarantine | ActionKind::Move => {
            let r = src.retained_at.as_deref().or(src.trash.as_ref().and_then(|t| t.location.as_deref()));
            orig && r.is_none_or(|r| !fsops::exists(r))
        }
        ActionKind::Copy => src.created.first().is_some_and(|c| !fsops::exists(c)),
        ActionKind::Hardlink => {
            orig && !same_file(ctx, &src.original, src.link_source.as_deref().unwrap_or(Path::new("")))
        }
        ActionKind::Dehydrate => ctx.platform.metadata(&src.original).map(|m| !m.is_online_only()).unwrap_or(false),
        ActionKind::FsCompress => matches!(ctx.platform.fs_compression(&src.original), Ok(None)),
        ActionKind::Optimise | ActionKind::Archive => {
            orig && fsops::stamp_of(&src.original).map(|s| s.size == src.original_stamp.size).unwrap_or(false)
                && src.retained_at.as_deref().is_none_or(|r| !fsops::exists(r))
        }
        _ => false,
    };
    if happened {
        write(ctx, &mut src, JournalState::Undone, None)?;
        write(ctx, &mut u, JournalState::Done, None)
    } else {
        let err = Error::Conflict("interrupted undo: nothing was restored; undo it again".into());
        write(ctx, &mut u, JournalState::Failed, Some(&err))
    }
}
