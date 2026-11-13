//! Undo / restore and crash recovery. **Owner: W2-E.**
//!
//! Contract: `undo` restores each entry to its `original` path (failing, not
//! overwriting, when something now exists there), removes files the action
//! `created`, writes `undone` / `undo_failed` records, and runs as its own run
//! (`RunKind::Undo`, `undo_of`). Entries are undone in reverse order.
//! `recover` settles `intent` records left by a crash at start-up.

use mj_core::{EntryId, Error, JobOutcome, Result};

use crate::engine::ExecContext;

/// Undo the given entries (newest first).
pub fn undo(ctx: &ExecContext<'_>, entries: &[EntryId]) -> Result<JobOutcome> {
    let _ = (ctx, entries);
    Err(Error::not_implemented("undo (W2-E)"))
}

/// Settle `intent` records from an interrupted run. Returns the entries changed.
pub fn recover(ctx: &ExecContext<'_>) -> Result<Vec<EntryId>> {
    let _ = ctx;
    Err(Error::not_implemented("journal recovery (W2-E)"))
}
