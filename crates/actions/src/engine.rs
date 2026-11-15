//! Executing action items. **Owner: W2-E.** Contract (frozen):
//!
//! - `execute` processes items in order, one at a time per volume, calling
//!   `ctx.cancel.checkpoint()` between items; a failure on one item is recorded
//!   and the batch continues; cancellation stops before the next item.
//! - Items failing the guard or the stamp check are `Skipped` (journal state
//!   `skipped`), never acted on.
//! - `Optimise` writes to a temp file in the same folder, applies
//!   keep-if-not-smaller, then replaces the original according to
//!   `OriginalsPolicy` (or writes a light copy), preserving timestamps and
//!   permissions per `Settings::optimise`.
//! - Lossy and irreversible items require the step to have been confirmed:
//!   the facade checks `StepConfirmation` *and* passes `ExecOptions::confirmed`;
//!   this layer does not trust its caller and refuses unconfirmed items itself
//!   (see below).
//! - `dry_run` performs every check without changing anything and returns
//!   exact numbers for removals and measured numbers for optimisations
//!   (outputs written to the temp dir, then deleted).
//!
//! W2-E additions (safety first, see docs/decisions/W2-E.md):
//!
//! - **SAF-11 is enforced here too.** [`execute`] treats every item as
//!   *unconfirmed*: permanent deletes, `originals: delete` and lossy optimiser
//!   outputs are skipped with `confirmation_required`. The facade passes the
//!   user's confirmation through [`execute_with`] and [`ExecOptions::confirmed`].
//! - Signed, labelled and encrypted files are skipped by default for every
//!   action except `copy` (per `Settings::safety::skip_*`), and reported.
//! - On Windows "to Recycle Bin" goes to the quarantine by default
//!   ([`TrashRoute`]), because the Recycle Bin cannot be restored exactly.
//!
//! Per item: guard + stamp check → confirmation and content-safety checks → plan
//! the paths → journal `intent` (synced) → re-stat (TOCTOU) → act → `done` /
//! `failed` / `skipped`. A journal write failure stops the batch (nothing is
//! ever done without its intent on disk). Each item runs inside `catch_unwind`;
//! a panic after the intent leaves the entry to the same settlement logic as
//! crash recovery, so the journal stays truthful.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use mj_core::{
    Action, ActionItem, ActionKind, Archiver, Badges, CancelToken, Confidence, EntryId, EntryKind, Error, Estimate,
    ExternalTool, ItemOutcome, ItemStatus, JobId, JobOutcome, JournalEntry, JournalState, NullProgress, Optimiser,
    OriginalsPolicy, OutputMode, Phase, Progress, ProgressSink, Result, RunId, Settings, StepId, TargetKind, Timestamp,
};
use mj_platform::Platform;

use crate::fsops;
use crate::guard::Guard;
use crate::journal::Journal;
pub use crate::retain::TrashRoute;
use crate::retain::{self, RetainPlan};
use crate::safety;

/// Optional engines the action engine delegates to.
#[derive(Clone, Copy)]
pub struct Services<'a> {
    pub optimisers: &'a [Box<dyn Optimiser>],
    pub archiver: Option<&'a dyn Archiver>,
    pub tools: &'a [ExternalTool],
}

/// Everything one execution needs.
pub struct ExecContext<'a> {
    pub platform: &'a dyn Platform,
    pub guard: &'a Guard,
    pub journal: &'a Journal,
    pub services: Services<'a>,
    pub settings: &'a Settings,
    /// Quarantine root for `Action::Quarantine` / `OriginalsPolicy::Quarantine`.
    pub quarantine_dir: &'a Path,
    /// Scratch directory for optimiser outputs during dry runs.
    pub temp_dir: &'a Path,
    pub run_id: RunId,
    pub job_id: JobId,
    pub step_id: Option<StepId>,
    pub cancel: &'a CancelToken,
    /// `Phase::Applying` progress, `done`/`total` in items, `bytes` = bytes saved so far.
    pub progress: &'a dyn ProgressSink,
}

/// Result of a dry run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DryRunResult {
    pub estimate: Estimate,
    /// Items that would be skipped, with the reason.
    pub skipped: Vec<ItemOutcome>,
}

/// Per-call options of [`execute_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecOptions {
    /// The user confirmed this step's lossy / irreversible consequences (SAF-11).
    pub confirmed: bool,
    /// Where "to Recycle Bin" goes.
    pub trash_route: TrashRoute,
    /// SHA-256 of files is computed for the audit log up to this size (bytes).
    pub audit_hash_limit: u64,
}

impl Default for ExecOptions {
    fn default() -> Self {
        ExecOptions { confirmed: false, trash_route: TrashRoute::default_for_os(), audit_hash_limit: 1 << 30 }
    }
}

/// Maximum number of failures kept in `JobOutcome::errors`.
const MAX_OUTCOME_ERRORS: usize = 50;

/// Apply `items` (unconfirmed: see the module docs).
pub fn execute(ctx: &ExecContext<'_>, items: &[ActionItem]) -> Result<JobOutcome> {
    execute_with(ctx, items, &ExecOptions::default())
}

/// True when the action needs the user's confirmation before it may run (SAF-11).
pub fn requires_confirmation(action: &Action) -> bool {
    matches!(
        action,
        Action::PermanentDelete
            // F-07: a hard link shares content with its keeper from then on.
            | Action::Hardlink
            | Action::Optimise { originals: OriginalsPolicy::Delete, output: OutputMode::Replace, .. }
            | Action::Archive { originals: OriginalsPolicy::Delete, keep_sources: false, .. }
    )
}

/// Apply `items` with explicit options.
pub fn execute_with(ctx: &ExecContext<'_>, items: &[ActionItem], opts: &ExecOptions) -> Result<JobOutcome> {
    let mut out = JobOutcome::default();
    if items.is_empty() {
        return Ok(out);
    }
    let first = ctx.journal.reserve_entry_ids(ctx.run_id, items.len() as u64)?;
    let total = items.len() as u64;
    for (i, item) in items.iter().enumerate() {
        if ctx.cancel.checkpoint().is_err() {
            break;
        }
        ctx.progress.report(&Progress {
            phase: Phase::Applying,
            done: i as u64,
            total: Some(total),
            bytes: out.bytes_saved,
            current: Some(item.path.clone()),
        });
        let id = EntryId(first.0 + i as u64);
        let res = catch_unwind(AssertUnwindSafe(|| process(ctx, opts, item, id)));
        let outcome = match res {
            Ok(Ok(o)) => o,
            // A journal failure: stop the batch; nothing further may act without its intent.
            Ok(Err(e)) => return Err(e),
            Err(panic) => {
                let msg = panic_message(&panic);
                if let Some(e) = ctx.journal.entry(id) {
                    if e.state == JournalState::Intent {
                        crate::undo::settle_intent(ctx, &e)?;
                    }
                }
                ItemOutcome {
                    path: item.path.clone(),
                    status: ItemStatus::Failed,
                    bytes_before: item.expected.size,
                    bytes_after: item.expected.size,
                    error: Some(Error::Internal(msg).info()),
                }
            }
        };
        match outcome.status {
            ItemStatus::Done => {
                out.succeeded += 1;
                out.bytes_saved = out.bytes_saved.saturating_add(saved(&item.action, &outcome));
            }
            ItemStatus::Skipped => out.skipped += 1,
            ItemStatus::Failed => out.failed += 1,
        }
        if outcome.status != ItemStatus::Done && out.errors.len() < MAX_OUTCOME_ERRORS {
            out.errors.push(outcome);
        }
    }
    let done = out.succeeded + out.skipped + out.failed;
    ctx.progress.report(&Progress {
        phase: Phase::Applying,
        done,
        total: Some(total),
        bytes: out.bytes_saved,
        current: None,
    });
    Ok(out)
}

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic while acting on an item".into())
}

/// Bytes an item freed: removals, moves and link replacements free their size; copies
/// and light copies free nothing.
fn saved(action: &Action, o: &ItemOutcome) -> u64 {
    match action {
        Action::Copy { .. } | Action::Optimise { output: OutputMode::LightCopy { .. }, .. } => 0,
        _ => o.bytes_before.saturating_sub(o.bytes_after),
    }
}

/// The journal kind of an item's action (trash may really be a quarantine move).
fn recorded_kind(action: &Action, route: TrashRoute) -> ActionKind {
    match action {
        Action::Trash => retain::trash_kind(route),
        a => a.kind(),
    }
}

/// A fresh journal entry for `item`.
fn new_entry(ctx: &ExecContext<'_>, item: &ActionItem, id: EntryId, kind: ActionKind, now: Timestamp) -> JournalEntry {
    JournalEntry {
        id,
        run_id: ctx.run_id,
        job_id: ctx.job_id,
        step_id: ctx.step_id,
        at: now,
        state: JournalState::Intent,
        action: kind,
        target: item.target,
        original: item.path.clone(),
        original_stamp: item.expected,
        original_sha256: None,
        bytes_before: item.expected.size,
        bytes_after: item.expected.size,
        retained_at: None,
        trash: None,
        created: Vec::new(),
        link_source: item.link_source.clone(),
        reversible: false,
        undo_deadline: None,
        lossy: false,
        error: None,
        original_mode: fsops::file_mode(&item.path),
        previous_compression: None,
        output_sha256: None,
        retention_days: None,
        original_link: std::fs::symlink_metadata(&item.path).is_ok_and(|m| m.file_type().is_symlink()),
        undo_of: None,
    }
}

fn record(ctx: &ExecContext<'_>, e: &mut JournalEntry, state: JournalState, err: Option<&Error>) -> Result<()> {
    e.state = state;
    e.at = Timestamp::now();
    e.error = err.map(Error::info);
    ctx.journal.append(e)
}

fn outcome(e: &JournalEntry, status: ItemStatus, err: Option<&Error>) -> ItemOutcome {
    ItemOutcome {
        path: e.original.clone(),
        status,
        bytes_before: e.bytes_before,
        bytes_after: if status == ItemStatus::Done { e.bytes_after } else { e.bytes_before },
        error: err.map(Error::info),
    }
}

/// Skip before acting: a `skipped` record (no intent was written).
fn skip(ctx: &ExecContext<'_>, mut e: JournalEntry, err: Error) -> Result<ItemOutcome> {
    e.reversible = false;
    record(ctx, &mut e, JournalState::Skipped, Some(&err))?;
    Ok(outcome(&e, ItemStatus::Skipped, Some(&err)))
}

/// Pre-flight checks shared by `execute` and `dry_run`. Returns fresh metadata.
fn preflight(ctx: &ExecContext<'_>, opts: &ExecOptions, item: &ActionItem) -> Result<mj_core::FileEntry> {
    let allow_link = matches!(item.action, Action::Trash | Action::Quarantine { .. } | Action::PermanentDelete);
    let md = ctx.guard.check_item(item, ctx.platform, allow_link)?;
    if matches!(item.action, Action::None) {
        return Err(Error::InvalidInput("this step has no action".into()));
    }
    // F-10: online-only state is re-read now, before any content is read (a read would
    // download the file). Removing or moving a placeholder out of its sync root deletes
    // the cloud copy and leaves nothing to restore; reading it downloads it.
    let touches_content = !matches!(item.action, Action::Dehydrate | Action::Copy { .. });
    if touches_content && md.kind == EntryKind::File && md.is_online_only() {
        return Err(online_only(&item.path, 1));
    }
    // F-01: a folder is acted on as a whole, so everything below it must be acceptable
    // too (protected paths, `.git`, `.ssh`, MyJanitor's own folders, mounted volumes).
    // A copy changes nothing at the source and is exempt.
    if md.kind == EntryKind::Dir && !matches!(item.action, Action::Copy { .. }) {
        let walk = ctx.guard.check_tree(&item.path, ctx.platform, item.owned_tree)?;
        if touches_content && walk.online_only > 0 {
            return Err(online_only(&item.path, walk.online_only));
        }
        // F-04: the folder must still hold exactly what the scan saw (the user reviewed
        // that); a folder item without a scan stamp of its subtree is not acted on.
        if item.expected_tree != Some(walk.stamp()) {
            return Err(Error::Changed(item.path.clone()));
        }
    }
    if requires_confirmation(&item.action) && !opts.confirmed {
        return Err(Error::ConfirmationRequired(format!("{} needs confirmation", item.action.kind().label_text())));
    }
    // Content safety (SAF-07/09): skip signed / labelled / encrypted files.
    if !matches!(item.action, Action::Copy { .. }) && md.kind == EntryKind::File {
        let skip_set = safety::skip_badges(ctx.settings);
        if !skip_set.is_empty() {
            let mut badges = safety::content_badges(&item.path).unwrap_or(Badges::empty());
            if md.attrs.contains(mj_core::Attributes::ENCRYPTED) {
                badges |= Badges::ENCRYPTED;
            }
            if badges.intersects(skip_set) {
                return Err(Error::Confidential(item.path.clone()));
            }
        }
    }
    Ok(md)
}

/// The refusal for online-only content (F-10).
fn online_only(path: &Path, n: u64) -> Error {
    Error::Conflict(format!(
        "skipped: {} holds {n} online-only file(s); removing or reading them would delete or download the \
         cloud copy (make them available offline first, or choose \"free up space\")",
        path.display()
    ))
}

/// True for actions after which the item's content is gone from the scanned tree (the
/// group invariant of F-02 applies to them).
fn removes_copy(action: &Action) -> bool {
    matches!(action, Action::Trash | Action::Quarantine { .. } | Action::PermanentDelete)
}

/// F-02: the group member that must survive this removal still exists and still matches.
/// `full` compares content byte for byte for an identical group (else the scan stamp);
/// without `full` (dry runs) the scan stamp and file id are compared only. Returns the
/// kept copy's fresh metadata.
fn check_keep(ctx: &ExecContext<'_>, item: &ActionItem, full: bool) -> Result<Option<mj_core::FileEntry>> {
    let Some(keep) = item.keep.as_ref().filter(|_| removes_copy(&item.action)) else {
        return Ok(None);
    };
    let gone = |why: &str| {
        Err(Error::Conflict(format!(
            "skipped: {} is the copy kept of this group and {why}; nothing was removed",
            keep.path.display()
        )))
    };
    if mj_core::pathutil::path_key(&keep.path) == mj_core::pathutil::path_key(&item.path) {
        return gone("cannot be removed itself");
    }
    let md = match ctx.platform.metadata(&keep.path) {
        Ok(m) => m,
        Err(_) => return gone("no longer exists"),
    };
    if md.kind != EntryKind::File {
        return gone("is no longer a regular file");
    }
    if let (Some(a), Some(b)) = (keep.expected_id, md.file_id) {
        if a != b {
            return gone("was replaced since the scan");
        }
    }
    let item_id = ctx.platform.metadata(&item.path).ok().and_then(|m| m.file_id).or(item.expected_id);
    if let (Some(a), Some(b)) = (item_id, md.file_id) {
        if a == b {
            return gone("is the same file (a hard link)");
        }
    }
    if keep.identical && full {
        match fsops::compare_files(&keep.path, &item.path) {
            Ok(Some(_)) => {}
            Ok(None) => return gone("no longer has the same content"),
            Err(_) => return gone("cannot be read"),
        }
    } else if !crate::guard::stamp_matches(keep.expected, &md) {
        return gone("changed since the scan");
    }
    Ok(Some(md))
}

/// Re-stat of the kept copy right before acting: still the file verified by [`check_keep`].
fn restat_keep(ctx: &ExecContext<'_>, item: &ActionItem, verified: &mj_core::FileEntry) -> Result<()> {
    let Some(keep) = &item.keep else { return Ok(()) };
    let fresh = ctx.platform.metadata(&keep.path).map_err(|_| {
        Error::Conflict(format!("skipped: {} (the copy kept) disappeared; nothing was removed", keep.path.display()))
    })?;
    if fresh.kind != EntryKind::File
        || fresh.size != verified.size
        || fresh.modified != verified.modified
        || fresh.file_id != verified.file_id
    {
        return Err(Error::Conflict(format!(
            "skipped: {} (the copy kept) changed; nothing was removed",
            keep.path.display()
        )));
    }
    Ok(())
}

trait KindLabel {
    fn label_text(self) -> &'static str;
}

impl KindLabel for ActionKind {
    fn label_text(self) -> &'static str {
        match self {
            ActionKind::PermanentDelete => "Permanent delete",
            ActionKind::Optimise => "Deleting the originals of an optimisation",
            ActionKind::Archive => "Deleting the archived sources",
            ActionKind::Hardlink => "Replacing duplicates with hard links",
            _ => "This action",
        }
    }
}

/// One item. `Err` only for journal failures.
fn process(ctx: &ExecContext<'_>, opts: &ExecOptions, item: &ActionItem, id: EntryId) -> Result<ItemOutcome> {
    let now = Timestamp::now();
    let mut e = new_entry(ctx, item, id, recorded_kind(&item.action, opts.trash_route), now);
    let md = match preflight(ctx, opts, item) {
        Ok(md) => md,
        Err(err) => return skip(ctx, e, err),
    };
    if md.kind == EntryKind::Dir {
        e.bytes_before = fsops::tree_size(&item.path);
        e.bytes_after = e.bytes_before;
    }
    // F-02: never remove the last copy of a group; the copy kept is verified now (full
    // comparison) and re-stat'ed right before acting.
    let kept = match check_keep(ctx, item, true) {
        Ok(k) => k,
        Err(err) => return skip(ctx, e, err),
    };
    // Audit hash (before acting; link actions compute it while verifying).
    if md.kind == EntryKind::File
        && md.size <= opts.audit_hash_limit
        && !matches!(item.action, Action::Hardlink | Action::Reflink)
    {
        match fsops::hash_file(&item.path) {
            Ok(h) => e.original_sha256 = Some(h.sha256),
            Err(err) => return skip(ctx, e, err),
        }
    }
    let undo_until = Some(now.add_days(i64::from(ctx.settings.general.undo_days)));
    match &item.action {
        Action::Trash => match opts.trash_route {
            TrashRoute::Platform => act_retain(ctx, item, e, RetainPlan::Trash, undo_until, kept.as_ref()),
            TrashRoute::Quarantine => {
                let days = ctx.settings.general.quarantine_days;
                quarantine_item(ctx, item, e, days, now, kept.as_ref())
            }
        },
        Action::Quarantine { purge_after_days } => quarantine_item(ctx, item, e, *purge_after_days, now, kept.as_ref()),
        Action::PermanentDelete => act_retain(ctx, item, e, RetainPlan::Delete, None, kept.as_ref()),
        Action::Move { dest, keep_structure } => move_or_copy(ctx, item, e, dest, *keep_structure, true, undo_until),
        Action::Copy { dest, keep_structure } => move_or_copy(ctx, item, e, dest, *keep_structure, false, undo_until),
        Action::Hardlink | Action::Reflink => link_replace(ctx, item, e, undo_until),
        Action::Dehydrate => dehydrate(ctx, item, e, undo_until),
        Action::FsCompress { algorithm } => fs_compress(ctx, item, e, *algorithm, undo_until),
        Action::FsDecompress => fs_decompress(ctx, item, e, undo_until),
        Action::Optimise { optimiser, params, output, originals, .. } => {
            optimise(ctx, opts, item, e, *optimiser, params, output, originals, undo_until)
        }
        Action::Archive { format, level, dest, originals, keep_sources } => {
            archive(ctx, opts, item, e, *format, *level, dest.as_deref(), originals, *keep_sources, undo_until)
        }
        Action::None => skip(ctx, e, Error::InvalidInput("this step has no action".into())),
    }
}

/// Write the intent, re-stat (TOCTOU) and run `act`; record the result.
fn run_acted(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    mut e: JournalEntry,
    act: impl FnOnce(&mut JournalEntry) -> Result<()>,
) -> Result<ItemOutcome> {
    record(ctx, &mut e, JournalState::Intent, None)?;
    // F-12 (narrowed, see the fixes notes): the ancestor walk (no link, junction or mount
    // point between the root and the item, canonical parent inside the root) is repeated
    // right before acting, not only at pre-flight.
    let allow_link = matches!(item.action, Action::Trash | Action::Quarantine { .. } | Action::PermanentDelete);
    if let Err(err) =
        ctx.guard.check(&item.path, ctx.platform, allow_link).and_then(|_| ctx.guard.restat(item, ctx.platform))
    {
        e.reversible = false;
        record(ctx, &mut e, JournalState::Skipped, Some(&err))?;
        return Ok(outcome(&e, ItemStatus::Skipped, Some(&err)));
    }
    e.error = None;
    match act(&mut e) {
        Ok(()) => {
            // A completed action may carry a note (F-09: a cross-volume move that left
            // part of its source behind); it is journalled with the `done` record.
            let note = e.error.take();
            e.state = JournalState::Done;
            e.at = Timestamp::now();
            e.error = note;
            ctx.journal.append(&e)?;
            Ok(ItemOutcome { error: e.error.clone(), ..outcome(&e, ItemStatus::Done, None) })
        }
        Err(err) => {
            let err = if safety::needs_elevation(&err, ctx.guard.protected()) {
                Error::NeedsElevation(item.path.clone())
            } else {
                err
            };
            let status = if matches!(err, Error::ConfirmationRequired(_) | Error::Changed(_)) || is_skip(&err) {
                JournalState::Skipped
            } else {
                JournalState::Failed
            };
            e.reversible = false;
            record(ctx, &mut e, status, Some(&err))?;
            let st = if status == JournalState::Skipped { ItemStatus::Skipped } else { ItemStatus::Failed };
            Ok(outcome(&e, st, Some(&err)))
        }
    }
}

/// Errors that mean "nothing was done, by design" rather than a failure.
fn is_skip(err: &Error) -> bool {
    matches!(err, Error::Conflict(m) if m.starts_with("skipped:")) || matches!(err, Error::Unsupported { .. })
}

fn act_retain(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    mut e: JournalEntry,
    plan: RetainPlan,
    undo_until: Option<Timestamp>,
    kept: Option<&mj_core::FileEntry>,
) -> Result<ItemOutcome> {
    retain::note_plan(&mut e, &plan);
    run_acted(ctx, item, e, |e| {
        if let Some(k) = kept {
            restat_keep(ctx, item, k)?;
        }
        recheck_tree(ctx, item)?;
        e.reversible = !matches!(plan, RetainPlan::Delete);
        if matches!(plan, RetainPlan::Delete) && is_empty_folder_item(item) {
            // F-04: an "empty folder" is removed with `remove_dir`, which fails rather
            // than delete anything that appeared in it since the check.
            std::fs::remove_dir(&item.path).map_err(|er| Error::io(&item.path, er))?;
            if let Some(parent) = item.path.parent() {
                let _ = mj_platform::action::sync_dir(parent);
            }
        } else {
            retain::retain(ctx.platform, ctx.quarantine_dir, e, &item.path, &plan, Timestamp::now())?;
        }
        e.bytes_after = 0;
        e.undo_deadline = if e.reversible { undo_until } else { None };
        Ok(())
    })
}

/// F-05: a backup folder for originals is a destination like any other: guarded (not
/// protected, not one of MyJanitor's folders, not inside the item, also through links),
/// and never inside the quarantine or the scratch folder, whose contents are purged.
fn check_retain_plan(ctx: &ExecContext<'_>, item: &ActionItem, plan: &RetainPlan) -> Result<()> {
    let RetainPlan::Backup { dest, .. } = plan else { return Ok(()) };
    let parent = dest.parent().ok_or_else(|| Error::InvalidInput(format!("no folder: {}", dest.display())))?;
    ctx.guard.check_destination(parent, &item.path)?;
    for own in [ctx.quarantine_dir, ctx.temp_dir] {
        if mj_core::pathutil::is_within(own, dest) {
            return Err(Error::Protected(dest.clone()));
        }
        if let (Ok(a), Some(b)) = (std::fs::canonicalize(own), existing_canonical(parent)) {
            if mj_core::pathutil::is_within(&a, &b) {
                return Err(Error::Protected(dest.clone()));
            }
        }
    }
    Ok(())
}

/// The canonical form of `p`, resolved through its nearest existing ancestor.
fn existing_canonical(p: &Path) -> Option<PathBuf> {
    let mut cur = Some(p);
    while let Some(c) = cur {
        if let Ok(canon) = std::fs::canonicalize(c) {
            return Some(canon.join(p.strip_prefix(c).unwrap_or(Path::new(""))));
        }
        cur = c.parent();
    }
    None
}

/// True for a folder item the scan saw empty.
fn is_empty_folder_item(item: &ActionItem) -> bool {
    item.target == TargetKind::Folder && item.expected_tree.is_some_and(|t| t.entries == 0)
}

/// F-04: right before acting on a folder, its subtree must still be what the scan saw.
fn recheck_tree(ctx: &ExecContext<'_>, item: &ActionItem) -> Result<()> {
    if matches!(item.action, Action::Copy { .. }) {
        return Ok(());
    }
    // G-09: a file item is re-stat'ed against its scan stamp and file id the same way,
    // so a write between (for example) an archive's verification and the removal of its
    // source is never lost.
    if item.target != TargetKind::Folder {
        return ctx.guard.restat(item, ctx.platform).map(|_| ());
    }
    let walk = ctx.guard.check_tree(&item.path, ctx.platform, item.owned_tree)?;
    if item.expected_tree != Some(walk.stamp()) {
        return Err(Error::Changed(item.path.clone()));
    }
    Ok(())
}

fn quarantine_item(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    mut e: JournalEntry,
    days: u32,
    now: Timestamp,
    kept: Option<&mj_core::FileEntry>,
) -> Result<ItemOutcome> {
    e.action = ActionKind::Quarantine;
    // F-15: the purge honours this item's own retention.
    e.retention_days = Some(days);
    let plan = match retain::plan(
        &OriginalsPolicy::Quarantine,
        TrashRoute::Quarantine,
        ctx.quarantine_dir,
        &e,
        &item.path,
        &item.root,
    ) {
        Ok(p) => p,
        Err(err) => return skip(ctx, e, err),
    };
    // Undoable while the item is held: the shorter of the retention and the undo window.
    let until = now.add_days(i64::from(days.min(ctx.settings.general.undo_days)));
    act_retain(ctx, item, e, plan, Some(until), kept)
}

/// Destination of a move / copy, keeping the structure below the scan root when asked.
fn destination(item: &ActionItem, dest: &Path, keep_structure: bool) -> Result<PathBuf> {
    let name =
        item.path.file_name().ok_or_else(|| Error::InvalidInput(format!("no file name: {}", item.path.display())))?;
    let rel = if keep_structure && !item.root.as_os_str().is_empty() {
        item.path.strip_prefix(&item.root).map(Path::to_path_buf).unwrap_or_else(|_| PathBuf::from(name))
    } else {
        PathBuf::from(name)
    };
    if rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err(Error::InvalidInput(format!("cannot place {} under {}", item.path.display(), dest.display())));
    }
    Ok(fsops::unique_path(&dest.join(rel)))
}

fn missing_dirs(p: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut cur = Some(p);
    while let Some(d) = cur {
        if fsops::exists(d) {
            break;
        }
        v.push(d.to_path_buf());
        cur = d.parent();
    }
    v.reverse();
    v
}

fn move_or_copy(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    mut e: JournalEntry,
    dest: &Path,
    keep_structure: bool,
    is_move: bool,
    undo_until: Option<Timestamp>,
) -> Result<ItemOutcome> {
    let target = match destination(item, dest, keep_structure)
        .and_then(|t| ctx.guard.check_destination(t.parent().unwrap_or(dest), &item.path).map(|_| t))
    {
        Ok(t) => t,
        Err(err) => return skip(ctx, e, err),
    };
    let new_dirs = missing_dirs(target.parent().unwrap_or(dest));
    if is_move {
        e.retained_at = Some(target.clone());
        e.created = new_dirs.clone();
    } else {
        e.created = std::iter::once(target.clone()).chain(new_dirs.iter().cloned()).collect();
    }
    run_acted(ctx, item, e, |e| {
        if is_move {
            recheck_tree(ctx, item)?;
        }
        e.reversible = true;
        if let Some(parent) = target.parent() {
            fsops::create_dirs(parent)?;
        }
        let r = if is_move {
            fsops::move_item(&item.path, &target).map(|how| {
                if let fsops::MoveHow::CopiedSourceLeft { left } = how {
                    e.error = Some(fsops::left_behind_note(&item.path, &target, left).info());
                }
            })
        } else {
            fsops::copy_item_verified(&item.path, &target)
        };
        if let Err(err) = r {
            fsops::remove_empty_dirs(&new_dirs);
            return Err(err);
        }
        e.bytes_after = if is_move { 0 } else { e.bytes_before };
        e.undo_deadline = undo_until;
        Ok(())
    })
}

/// The temporary name a link / optimised output is created under, beside `path`.
pub fn temp_beside(path: &Path, id: EntryId, keep_ext: bool) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|n| n.to_string_lossy().into_owned());
    let file = match (keep_ext, ext) {
        (true, Some(ext)) => format!(".{stem}.mj-{}.tmp.{ext}", id.0),
        _ => format!(".{name}.mj-{}.tmp", id.0),
    };
    path.with_file_name(file)
}

fn link_replace(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    mut e: JournalEntry,
    undo_until: Option<Timestamp>,
) -> Result<ItemOutcome> {
    let is_hard = matches!(item.action, Action::Hardlink);
    let Some(keeper) = item.link_source.clone() else {
        return skip(ctx, e, Error::InvalidInput("no file to link to (the group has no keeper)".into()));
    };
    if let Err(err) = ctx.guard.check(&keeper, ctx.platform, false) {
        return skip(ctx, e, err);
    }
    let (kmd, dmd) = match (ctx.platform.metadata(&keeper), ctx.platform.metadata(&item.path)) {
        (Ok(k), Ok(d)) => (k, d),
        (Err(err), _) | (_, Err(err)) => return skip(ctx, e, err),
    };
    if kmd.kind != EntryKind::File || dmd.kind != EntryKind::File {
        return skip(ctx, e, Error::InvalidInput("hard links and reflinks replace regular files only".into()));
    }
    if let (Some(a), Some(b)) = (kmd.file_id, dmd.file_id) {
        if a == b {
            return skip(ctx, e, Error::Conflict("skipped: the files are already hard links of each other".into()));
        }
        if is_hard && a.volume != b.volume {
            return skip(ctx, e, Error::unsupported("Hard link", "the files are on different volumes"));
        }
    }
    // Verify content equality right before acting (full byte comparison).
    match fsops::compare_files(&keeper, &item.path) {
        Ok(Some(sha)) => e.original_sha256 = Some(sha),
        Ok(None) => return skip(ctx, e, Error::Changed(item.path.clone())),
        Err(err) => return skip(ctx, e, err),
    }
    let temp = temp_beside(&item.path, e.id, false);
    if fsops::exists(&temp) {
        return skip(ctx, e, Error::Conflict(format!("{} exists; nothing was overwritten", temp.display())));
    }
    e.created = vec![temp.clone()];
    e.link_source = Some(keeper.clone());
    run_acted(ctx, item, e, |e| {
        let made = if is_hard { ctx.platform.hardlink(&keeper, &temp) } else { ctx.platform.reflink(&keeper, &temp) };
        made?;
        let result = (|| {
            if !is_hard {
                // A reflink is a new inode: give it the duplicate's permissions and mtime.
                let perms =
                    std::fs::symlink_metadata(&item.path).map_err(|er| Error::io(&item.path, er))?.permissions();
                std::fs::set_permissions(&temp, perms).map_err(|er| Error::io(&temp, er))?;
                fsops::set_mtime(&temp, item.expected.mtime)?;
            }
            // TOCTOU: the duplicate must still be what was verified.
            ctx.guard.restat(item, ctx.platform)?;
            std::fs::rename(&temp, &item.path).map_err(|er| Error::io(&item.path, er))
        })();
        if let Err(err) = result {
            let _ = std::fs::remove_file(&temp);
            return Err(err);
        }
        let _ = mj_platform::action::sync_dir(item.path.parent().unwrap_or(Path::new("/")));
        e.created.clear();
        e.reversible = true;
        e.bytes_after = 0;
        e.undo_deadline = undo_until;
        Ok(())
    })
}

fn simple_platform(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    e: JournalEntry,
    undo_until: Option<Timestamp>,
    f: impl FnOnce(&dyn Platform, &Path) -> Result<()>,
    bytes_after: u64,
) -> Result<ItemOutcome> {
    run_acted(ctx, item, e, |e| {
        f(ctx.platform, &item.path)?;
        e.reversible = undo_until.is_some();
        e.undo_deadline = undo_until;
        e.bytes_after = bytes_after;
        Ok(())
    })
}

/// F-20: a dehydration frees space only once the content is really no longer local; the
/// fresh attributes say so (the platform already refuses when nothing was freed).
fn dehydrate(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    e: JournalEntry,
    undo_until: Option<Timestamp>,
) -> Result<ItemOutcome> {
    run_acted(ctx, item, e, |e| {
        ctx.platform.dehydrate(&item.path)?;
        let online = ctx.platform.metadata(&item.path).map(|m| m.is_online_only()).unwrap_or(false);
        e.reversible = undo_until.is_some();
        e.undo_deadline = undo_until;
        e.bytes_after = if online { 0 } else { e.bytes_before };
        Ok(())
    })
}

fn fs_compress(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    e: JournalEntry,
    algo: mj_core::FsCompressAlgo,
    undo_until: Option<Timestamp>,
) -> Result<ItemOutcome> {
    match ctx.platform.fs_compression(&item.path) {
        Ok(Some(_)) => return skip(ctx, e, Error::Conflict("skipped: the file is already compressed".into())),
        Ok(None) => {}
        Err(err) => return skip(ctx, e, err),
    }
    run_acted(ctx, item, e, |e| {
        ctx.platform.fs_compress(&item.path, algo)?;
        e.reversible = true;
        e.undo_deadline = undo_until;
        e.bytes_after = ctx.platform.metadata(&item.path).ok().and_then(|m| m.allocated).unwrap_or(e.bytes_before);
        Ok(())
    })
}

fn fs_decompress(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    mut e: JournalEntry,
    undo_until: Option<Timestamp>,
) -> Result<ItemOutcome> {
    // The previous algorithm is journalled so undo can compress again with it.
    match ctx.platform.fs_compression(&item.path) {
        Ok(Some(a)) => e.previous_compression = Some(a),
        Ok(None) => return skip(ctx, e, Error::Conflict("skipped: the file is not compressed".into())),
        Err(err) => return skip(ctx, e, err),
    }
    let size = item.expected.size;
    simple_platform(ctx, item, e, undo_until, |p, path| p.fs_decompress(path), size)
}

#[allow(clippy::too_many_arguments)]
fn optimise(
    ctx: &ExecContext<'_>,
    opts: &ExecOptions,
    item: &ActionItem,
    mut e: JournalEntry,
    id: mj_core::OptimiserId,
    params: &mj_core::ParamValues,
    output: &OutputMode,
    originals: &OriginalsPolicy,
    undo_until: Option<Timestamp>,
) -> Result<ItemOutcome> {
    let Some(opt) = ctx.services.optimisers.iter().find(|o| o.info().id == id) else {
        return skip(ctx, e, Error::not_implemented(format!("optimiser {id:?}")));
    };
    if item.target != TargetKind::File {
        return skip(ctx, e, Error::InvalidInput("optimisers work on files".into()));
    }
    let temp = temp_beside(&item.path, e.id, true);
    if fsops::exists(&temp) {
        return skip(ctx, e, Error::Conflict(format!("{} exists; nothing was overwritten", temp.display())));
    }
    let replace = matches!(output, OutputMode::Replace);
    let plan = if replace {
        match retain::plan(originals, opts.trash_route, ctx.quarantine_dir, &e, &item.path, &item.root)
            .and_then(|p| check_retain_plan(ctx, item, &p).map(|_| p))
        {
            Ok(p) => Some(p),
            Err(err) => return skip(ctx, e, err),
        }
    } else {
        None
    };
    if let Some(p) = &plan {
        retain::note_plan(&mut e, p);
    }
    e.created = vec![temp.clone()];
    let preserve_times = ctx.settings.optimise.preserve_timestamps;
    let preserve_perms = ctx.settings.optimise.preserve_permissions;
    run_acted(ctx, item, e, |e| {
        // F-03: the optimiser's intermediate files go to a folder this item created for
        // itself (never a name shared with another job, a dry run or another process).
        let scratch = mj_core::scratch::ScratchDir::new(ctx.temp_dir, &format!("apply-{}", e.id.0))?;
        let job = mj_core::OptimiseJob {
            src: &item.path,
            dst: &temp,
            params,
            tools: ctx.services.tools,
            temp_dir: scratch.path(),
            cancel: ctx.cancel,
            progress: &NullProgress,
        };
        // F-16: the temp name must still be free right before the optimiser runs.
        if fsops::exists(&temp) {
            return Err(Error::Conflict(format!("{} exists; nothing was overwritten", temp.display())));
        }
        let res = opt.optimise(&job);
        drop(scratch);
        let cleanup = || {
            let _ = std::fs::remove_file(&temp);
        };
        let out = match res {
            Ok(o) => o,
            // A `create_new` / no-replace refusal of the temp name itself means someone else
            // took it between the check and the write: that file is not ours to delete. Any
            // other failure (G-08: a `Conflict` for another reason included) leaves no
            // output of ours beside the user's file.
            Err(err) if err.is_destination_taken(&temp) => return Err(err),
            Err(err) => {
                cleanup();
                return Err(err);
            }
        };
        if let Some(reason) = out.skipped {
            cleanup();
            return Err(Error::Conflict(format!("skipped: {reason}")));
        }
        let new_size = match std::fs::symlink_metadata(&temp) {
            Ok(m) if m.is_file() => m.len(),
            _ => {
                cleanup();
                return Err(Error::Internal("the optimiser reported success but wrote no file".into()));
            }
        };
        // F-03: the output must be a new, independent file this job wrote: not another
        // name of the original (or of anything else) through a hard link.
        if let Err(err) = output_is_independent(ctx, &temp, &item.path) {
            cleanup();
            return Err(err);
        }
        // OPT-ENG-04: keep the original unless the output is smaller (cannot be disabled).
        // A privacy rewrite (GPS / metadata strip) may be the same size, never larger.
        if !keeps_output(&out, e.bytes_before, new_size) {
            cleanup();
            return Err(Error::Conflict("skipped: the optimised file was not smaller; the original was kept".into()));
        }
        // A conversion (WebP, Opus, MP4...) is named with its own extension, never with the
        // source's: a `.wav` holding Opus data would no longer open.
        let orig_ext = item.path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let new_ext = out.converted_extension().filter(|x| !x.eq_ignore_ascii_case(&orig_ext));
        let converted_path = new_ext.as_ref().map(|x| fsops::unique_path(&item.path.with_extension(x)));
        e.lossy = out.lossy;
        if out.lossy && !opts.confirmed {
            cleanup();
            return Err(Error::ConfirmationRequired("lossy output needs confirmation".into()));
        }
        if let Err(err) = ctx.guard.restat(item, ctx.platform) {
            cleanup();
            return Err(err);
        }
        let orig_md = std::fs::symlink_metadata(&item.path).map_err(|er| Error::io(&item.path, er))?;
        if preserve_perms {
            let _ = std::fs::set_permissions(&temp, orig_md.permissions());
        }
        if preserve_times {
            let _ = fsops::set_mtime(&temp, item.expected.mtime);
        }
        // F-11: whatever wrote it (an in-house rewriter or ffmpeg, cwebp, avifenc...), the
        // output is on disk before the original moves or is replaced.
        if let Err(err) = fsops::sync_output(&temp) {
            cleanup();
            return Err(err);
        }
        e.bytes_after = new_size;
        match &plan {
            None => {
                // Light copy beside the original; nothing is removed.
                let suffix = match output {
                    OutputMode::LightCopy { suffix } => suffix.clone(),
                    OutputMode::Replace => String::new(),
                };
                let stem = item.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = match &new_ext {
                    Some(x) => format!(".{x}"),
                    None => item.path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default(),
                };
                let fin = fsops::unique_path(&item.path.with_file_name(format!("{stem}{suffix}{ext}")));
                if let Err(err) = mj_platform::action::rename_no_replace(&temp, &fin) {
                    cleanup();
                    return Err(err);
                }
                e.created = vec![fin];
                e.reversible = true;
            }
            Some(RetainPlan::Delete) => {
                // Irreversible (confirmed): the output atomically replaces the original, or
                // (conversion) takes its new name and then the original is deleted.
                match &converted_path {
                    None => {
                        if let Err(er) = std::fs::rename(&temp, &item.path) {
                            cleanup();
                            return Err(Error::io(&item.path, er));
                        }
                        e.created = vec![item.path.clone()];
                    }
                    Some(target) => {
                        if let Err(err) = mj_platform::action::rename_no_replace(&temp, target) {
                            cleanup();
                            return Err(err);
                        }
                        if let Err(er) = std::fs::remove_file(&item.path) {
                            // Keep the user's original; take the new file back.
                            let _ = std::fs::remove_file(target);
                            return Err(Error::io(&item.path, er));
                        }
                        e.created = vec![target.clone()];
                    }
                }
                e.reversible = false;
            }
            Some(p) => {
                if let Err(err) = retain::retain(ctx.platform, ctx.quarantine_dir, e, &item.path, p, Timestamp::now()) {
                    cleanup();
                    return Err(err);
                }
                let target = converted_path.clone().unwrap_or_else(|| item.path.clone());
                if let Err(err) = mj_platform::action::rename_no_replace(&temp, &target) {
                    // Roll back: the original goes back to its place, the output is removed.
                    let back = retain::restore(ctx.platform, ctx.quarantine_dir, e, &item.path);
                    cleanup();
                    return Err(match back {
                        Ok(()) => err,
                        Err(b) => Error::Internal(format!("{err}; the original could not be put back: {b}")),
                    });
                }
                e.created = vec![target];
                e.reversible = true;
            }
        }
        // F-11: the directory entries of the new name (and of the moved original) too.
        if let Some(parent) = item.path.parent() {
            let _ = mj_platform::action::sync_dir(parent);
        }
        // F-06: undo discards this output only while it still has this content.
        e.output_sha256 = e.created.first().and_then(|p| fsops::hash_file(p).ok()).map(|h| h.sha256);
        e.undo_deadline = if e.reversible { undo_until } else { None };
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
fn archive(
    ctx: &ExecContext<'_>,
    opts: &ExecOptions,
    item: &ActionItem,
    mut e: JournalEntry,
    format: mj_core::ArchiveFormat,
    level: u8,
    dest: Option<&Path>,
    originals: &OriginalsPolicy,
    keep_sources: bool,
    undo_until: Option<Timestamp>,
) -> Result<ItemOutcome> {
    let Some(archiver) = ctx.services.archiver else {
        return skip(ctx, e, Error::not_implemented("archiver"));
    };
    let base = item.path.parent().map(Path::to_path_buf).unwrap_or_default();
    let folder = dest.map(Path::to_path_buf).unwrap_or_else(|| base.clone());
    if let Err(err) = ctx.guard.check_destination(&folder, &item.path) {
        return skip(ctx, e, err);
    }
    let ext = match format {
        mj_core::ArchiveFormat::Zip => "zip",
        mj_core::ArchiveFormat::SevenZip => "7z",
        mj_core::ArchiveFormat::TarZstd => "tar.zst",
    };
    let name = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dst = fsops::unique_path(&folder.join(format!("{name}.{ext}")));
    let plan = if keep_sources {
        None
    } else {
        match retain::plan(originals, opts.trash_route, ctx.quarantine_dir, &e, &item.path, &item.root)
            .and_then(|p| check_retain_plan(ctx, item, &p).map(|_| p))
        {
            Ok(p) => Some(p),
            Err(err) => return skip(ctx, e, err),
        }
    };
    // G-14: an archive stores no links (they are only listed in INDEX.csv) and a 7z
    // archive no empty folders; when the sources would go, those would exist nowhere.
    if plan.is_some() && item.target == TargetKind::Folder {
        let (links, empty_dirs) = fsops::links_and_empty_dirs(&item.path);
        let empty = if format == mj_core::ArchiveFormat::SevenZip { empty_dirs } else { 0 };
        if links > 0 || empty > 0 {
            let what = match (links, empty) {
                (l, 0) => format!("{l} link(s)"),
                (0, d) => format!("{d} empty folder(s)"),
                (l, d) => format!("{l} link(s) and {d} empty folder(s)"),
            };
            return skip(
                ctx,
                e,
                Error::Conflict(format!(
                    "skipped: {} holds {what} the archive cannot store, so its sources were kept; \
                     archive it with \"keep the originals\" or remove them first",
                    item.path.display()
                )),
            );
        }
    }
    if let Some(p) = &plan {
        retain::note_plan(&mut e, p);
    }
    e.created = vec![dst.clone()];
    run_acted(ctx, item, e, |e| {
        let sources = [item.path.clone()];
        let job = mj_core::ArchiveJob {
            sources: &sources,
            base: &base,
            dst: &dst,
            format,
            level,
            tools: ctx.services.tools,
            cancel: ctx.cancel,
            progress: &NullProgress,
        };
        // F-16: a failed `create` never wrote `dst` (the archiver builds elsewhere and moves
        // in without replacing); if it failed because `dst` appeared meanwhile, that file is
        // someone else's and is left alone. Only an archive this run created is removed.
        let size = archiver.create(&job)?;
        let size = archiver
            .verify(&dst, &sources, &base, ctx.cancel)
            .map(|_| size)
            // F-04: the sources go only if they still are what was scanned and archived.
            .and_then(|s| if plan.is_some() { recheck_tree(ctx, item).map(|_| s) } else { Ok(s) });
        let size = match size {
            Ok(s) => s,
            Err(err) => {
                let _ = std::fs::remove_file(&dst);
                return Err(err);
            }
        };
        // F-11: the archive is on disk before its sources go.
        if let Err(err) = fsops::sync_output(&dst) {
            let _ = std::fs::remove_file(&dst);
            return Err(err);
        }
        if let Some(parent) = dst.parent() {
            let _ = mj_platform::action::sync_dir(parent);
        }
        e.bytes_after = size;
        e.reversible = true;
        // F-06: undo removes the archive only while it still has this content.
        e.output_sha256 = fsops::hash_file(&dst).ok().map(|h| h.sha256);
        if let Some(p) = &plan {
            if let Err(err) = retain::retain(ctx.platform, ctx.quarantine_dir, e, &item.path, p, Timestamp::now()) {
                // ACT-1: removing a folder is not atomic (a permanent delete or a platform
                // trash can stop part-way). The archive is removed only while the sources are
                // provably whole; otherwise it is the only complete copy of what is gone.
                if recheck_tree(ctx, item).is_ok() {
                    let _ = std::fs::remove_file(&dst);
                    return Err(err);
                }
                return Err(Error::Conflict(format!(
                    "{err}; part of {} was already removed, so the complete verified archive was kept at {}",
                    item.path.display(),
                    dst.display()
                )));
            }
        } else {
            e.bytes_after = e.bytes_before.saturating_add(size);
        }
        e.undo_deadline = if e.reversible { undo_until } else { None };
        Ok(())
    })
}

/// Check and measure `items` without changing anything.
pub fn dry_run(ctx: &ExecContext<'_>, items: &[ActionItem]) -> Result<DryRunResult> {
    dry_run_with(ctx, items, &ExecOptions { confirmed: true, ..ExecOptions::default() })
}

/// [`dry_run`] with explicit options (`confirmed: false` reports unconfirmed items as skipped).
pub fn dry_run_with(ctx: &ExecContext<'_>, items: &[ActionItem], opts: &ExecOptions) -> Result<DryRunResult> {
    let mut r = DryRunResult {
        estimate: Estimate { confidence: Confidence::Exact, ..Estimate::default() },
        ..Default::default()
    };
    let total = items.len() as u64;
    for (i, item) in items.iter().enumerate() {
        ctx.cancel.checkpoint()?;
        ctx.progress.report(&Progress::new(Phase::Estimating, i as u64, Some(total)));
        let res = catch_unwind(AssertUnwindSafe(|| measure(ctx, opts, item)))
            .unwrap_or_else(|p| Err(Error::Internal(panic_message(&p))));
        match res {
            Ok((before, after, conf)) => {
                r.estimate.files += 1;
                r.estimate.bytes_before = r.estimate.bytes_before.saturating_add(before);
                r.estimate.bytes_after = r.estimate.bytes_after.saturating_add(after);
                if conf != Confidence::Exact {
                    r.estimate.confidence = conf;
                }
            }
            Err(err) => {
                r.estimate.skipped += 1;
                r.skipped.push(ItemOutcome {
                    path: item.path.clone(),
                    status: ItemStatus::Skipped,
                    bytes_before: item.expected.size,
                    bytes_after: item.expected.size,
                    error: Some(err.info()),
                });
            }
        }
    }
    ctx.progress.report(&Progress::new(Phase::Estimating, total, Some(total)));
    Ok(r)
}

/// F-03: an optimiser output is replaced into place only when it is a regular file with
/// a single name that is not the original itself.
fn output_is_independent(ctx: &ExecContext<'_>, output: &Path, original: &Path) -> Result<()> {
    let md = ctx.platform.metadata(output)?;
    let orig = ctx.platform.metadata(original)?;
    let shared = md.links.is_some_and(|n| n > 1) || (md.file_id.is_some() && md.file_id == orig.file_id);
    if md.kind != EntryKind::File || shared {
        return Err(Error::Conflict(format!(
            "{} is not a new file written by this job; nothing was replaced",
            output.display()
        )));
    }
    Ok(())
}

/// Keep-if-not-smaller (OPT-ENG-04): an output is kept only when strictly smaller, or the
/// same size for a privacy rewrite (GPS / metadata strip).
fn keeps_output(out: &mj_core::OptimiseOutput, before: u64, after: u64) -> bool {
    after < before || (out.privacy && after == before)
}

/// (bytes before, bytes after, confidence) of one item, read-only.
fn measure(ctx: &ExecContext<'_>, opts: &ExecOptions, item: &ActionItem) -> Result<(u64, u64, Confidence)> {
    let md = preflight(ctx, opts, item)?;
    check_keep(ctx, item, false)?;
    let before = if md.kind == EntryKind::Dir { fsops::tree_size(&item.path) } else { md.reclaimable() };
    match &item.action {
        Action::Copy { dest, keep_structure } => {
            let t = destination(item, dest, *keep_structure)?;
            ctx.guard.check_destination(t.parent().unwrap_or(dest), &item.path)?;
            Ok((before, before, Confidence::Exact))
        }
        Action::Move { dest, keep_structure } => {
            let t = destination(item, dest, *keep_structure)?;
            ctx.guard.check_destination(t.parent().unwrap_or(dest), &item.path)?;
            Ok((before, 0, Confidence::Exact))
        }
        Action::Hardlink | Action::Reflink => {
            let keeper = item.link_source.as_ref().ok_or_else(|| Error::InvalidInput("no keeper".into()))?;
            ctx.guard.check(keeper, ctx.platform, false)?;
            if fsops::compare_files(keeper, &item.path)?.is_none() {
                return Err(Error::Changed(item.path.clone()));
            }
            Ok((before, 0, Confidence::Exact))
        }
        Action::Optimise { optimiser, params, .. } => {
            let opt = ctx
                .services
                .optimisers
                .iter()
                .find(|o| o.info().id == *optimiser)
                .ok_or_else(|| Error::not_implemented(format!("optimiser {optimiser:?}")))?;
            // F-03: the measured output and the optimiser's own intermediate files live in a
            // folder this dry run created for itself; it is removed with everything in it.
            let scratch = mj_core::scratch::ScratchDir::new(ctx.temp_dir, &format!("dry-run-{}", ctx.job_id.0))?;
            let ext = item.path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
            let dst = scratch.join(format!("output{ext}"));
            let work = mj_core::scratch::ScratchDir::new(scratch.path(), "work")?;
            let job = mj_core::OptimiseJob {
                src: &item.path,
                dst: &dst,
                params,
                tools: ctx.services.tools,
                temp_dir: work.path(),
                cancel: ctx.cancel,
                progress: &NullProgress,
            };
            let res = opt.optimise(&job);
            let size = std::fs::symlink_metadata(&dst).ok().filter(|m| m.is_file()).map(|m| m.len());
            drop(work);
            drop(scratch);
            let out = res?;
            if let Some(reason) = &out.skipped {
                return Err(Error::Conflict(format!("skipped: {reason}")));
            }
            let after = size.ok_or_else(|| Error::Internal("the optimiser wrote no file".into()))?;
            // Compare file lengths, as `execute` does (allocated sizes would let a dry run
            // promise a saving that the real run then refuses).
            let before = if md.kind == EntryKind::File { md.size } else { before };
            if !keeps_output(&out, before, after) {
                return Err(Error::Conflict("skipped: the optimised file would not be smaller".into()));
            }
            if out.lossy && !opts.confirmed {
                return Err(Error::ConfirmationRequired("lossy output needs confirmation".into()));
            }
            Ok((before, after, Confidence::Exact))
        }
        Action::Archive { .. } | Action::FsCompress { .. } | Action::FsDecompress | Action::Dehydrate => {
            Ok((before, 0, Confidence::Estimated))
        }
        _ => Ok((before, 0, Confidence::Exact)),
    }
}

#[cfg(test)]
mod tests {
    use super::keeps_output;
    use mj_core::OptimiseOutput;

    #[test]
    fn keep_if_not_smaller_exempts_only_equal_size_privacy_rewrites() {
        let plain = OptimiseOutput::default();
        let privacy = OptimiseOutput { privacy: true, ..OptimiseOutput::default() };
        assert!(keeps_output(&plain, 100, 99));
        assert!(!keeps_output(&plain, 100, 100));
        assert!(keeps_output(&privacy, 100, 100), "a GPS strip of the same size is kept");
        assert!(!keeps_output(&privacy, 100, 101), "never a larger file");
    }
}
