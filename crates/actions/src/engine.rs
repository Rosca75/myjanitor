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
//! - Lossy items require the step to have been confirmed (the engine facade
//!   checks `StepConfirmation`; this layer trusts its caller on that).
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
                return Err(Error::Protected(item.path.clone()));
            }
        }
    }
    Ok(md)
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
            TrashRoute::Platform => act_retain(ctx, item, e, RetainPlan::Trash, undo_until),
            TrashRoute::Quarantine => {
                let days = ctx.settings.general.quarantine_days;
                quarantine_item(ctx, item, e, days, now)
            }
        },
        Action::Quarantine { purge_after_days } => quarantine_item(ctx, item, e, *purge_after_days, now),
        Action::PermanentDelete => act_retain(ctx, item, e, RetainPlan::Delete, None),
        Action::Move { dest, keep_structure } => move_or_copy(ctx, item, e, dest, *keep_structure, true, undo_until),
        Action::Copy { dest, keep_structure } => move_or_copy(ctx, item, e, dest, *keep_structure, false, undo_until),
        Action::Hardlink | Action::Reflink => link_replace(ctx, item, e, undo_until),
        Action::Dehydrate => simple_platform(ctx, item, e, undo_until, |p, path| p.dehydrate(path), 0),
        Action::FsCompress { algorithm } => fs_compress(ctx, item, e, *algorithm, undo_until),
        Action::FsDecompress => {
            let size = item.expected.size;
            simple_platform(ctx, item, e, None, |p, path| p.fs_decompress(path), size)
        }
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
    if let Err(err) = ctx.guard.restat(item, ctx.platform) {
        e.reversible = false;
        record(ctx, &mut e, JournalState::Skipped, Some(&err))?;
        return Ok(outcome(&e, ItemStatus::Skipped, Some(&err)));
    }
    match act(&mut e) {
        Ok(()) => {
            record(ctx, &mut e, JournalState::Done, None)?;
            Ok(outcome(&e, ItemStatus::Done, None))
        }
        Err(err) => {
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
) -> Result<ItemOutcome> {
    retain::note_plan(&mut e, &plan);
    run_acted(ctx, item, e, |e| {
        e.reversible = !matches!(plan, RetainPlan::Delete);
        retain::retain(ctx.platform, ctx.quarantine_dir, e, &item.path, &plan, Timestamp::now())?;
        e.bytes_after = 0;
        e.undo_deadline = if e.reversible { undo_until } else { None };
        Ok(())
    })
}

fn quarantine_item(
    ctx: &ExecContext<'_>,
    item: &ActionItem,
    mut e: JournalEntry,
    days: u32,
    now: Timestamp,
) -> Result<ItemOutcome> {
    e.action = ActionKind::Quarantine;
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
    act_retain(ctx, item, e, plan, Some(until))
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
        e.reversible = true;
        if let Some(parent) = target.parent() {
            fsops::create_dirs(parent)?;
        }
        let r = if is_move {
            fsops::move_item(&item.path, &target).map(|_| ())
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
        match retain::plan(originals, opts.trash_route, ctx.quarantine_dir, &e, &item.path, &item.root) {
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
        let job = mj_core::OptimiseJob {
            src: &item.path,
            dst: &temp,
            params,
            tools: ctx.services.tools,
            temp_dir: ctx.temp_dir,
            cancel: ctx.cancel,
            progress: &NullProgress,
        };
        let res = opt.optimise(&job);
        let cleanup = || {
            let _ = std::fs::remove_file(&temp);
        };
        let out = match res {
            Ok(o) => o,
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
        // OPT-ENG-04: keep the original unless the output is smaller (cannot be disabled).
        if new_size >= e.bytes_before {
            cleanup();
            return Err(Error::Conflict("skipped: the optimised file was not smaller; the original was kept".into()));
        }
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
        e.bytes_after = new_size;
        match &plan {
            None => {
                // Light copy beside the original; nothing is removed.
                let suffix = match output {
                    OutputMode::LightCopy { suffix } => suffix.clone(),
                    OutputMode::Replace => String::new(),
                };
                let stem = item.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = item.path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
                let fin = fsops::unique_path(&item.path.with_file_name(format!("{stem}{suffix}{ext}")));
                if let Err(err) = mj_platform::action::rename_no_replace(&temp, &fin) {
                    cleanup();
                    return Err(err);
                }
                e.created = vec![fin];
                e.reversible = true;
            }
            Some(RetainPlan::Delete) => {
                // Irreversible (confirmed): the output atomically replaces the original.
                if let Err(er) = std::fs::rename(&temp, &item.path) {
                    cleanup();
                    return Err(Error::io(&item.path, er));
                }
                e.created = vec![item.path.clone()];
                e.reversible = false;
            }
            Some(p) => {
                if let Err(err) = retain::retain(ctx.platform, ctx.quarantine_dir, e, &item.path, p, Timestamp::now()) {
                    cleanup();
                    return Err(err);
                }
                if let Err(err) = mj_platform::action::rename_no_replace(&temp, &item.path) {
                    // Roll back: the original goes back to its place, the output is removed.
                    let back = retain::restore(ctx.platform, ctx.quarantine_dir, e, &item.path);
                    cleanup();
                    return Err(match back {
                        Ok(()) => err,
                        Err(b) => Error::Internal(format!("{err}; the original could not be put back: {b}")),
                    });
                }
                e.created = vec![item.path.clone()];
                e.reversible = true;
            }
        }
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
        match retain::plan(originals, opts.trash_route, ctx.quarantine_dir, &e, &item.path, &item.root) {
            Ok(p) => Some(p),
            Err(err) => return skip(ctx, e, err),
        }
    };
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
        let size = archiver.create(&job).and_then(|s| archiver.verify(&dst, &sources, &base, ctx.cancel).map(|_| s));
        let size = match size {
            Ok(s) => s,
            Err(err) => {
                let _ = std::fs::remove_file(&dst);
                return Err(err);
            }
        };
        e.bytes_after = size;
        e.reversible = true;
        if let Some(p) = &plan {
            if let Err(err) = retain::retain(ctx.platform, ctx.quarantine_dir, e, &item.path, p, Timestamp::now()) {
                let _ = std::fs::remove_file(&dst);
                return Err(err);
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

/// (bytes before, bytes after, confidence) of one item, read-only.
fn measure(ctx: &ExecContext<'_>, opts: &ExecOptions, item: &ActionItem) -> Result<(u64, u64, Confidence)> {
    let md = preflight(ctx, opts, item)?;
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
            std::fs::create_dir_all(ctx.temp_dir).map_err(|e| Error::io(ctx.temp_dir, e))?;
            let ext = item.path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
            let dst = fsops::unique_path(&ctx.temp_dir.join(format!("dry-run-{}{ext}", ctx.job_id.0)));
            let job = mj_core::OptimiseJob {
                src: &item.path,
                dst: &dst,
                params,
                tools: ctx.services.tools,
                temp_dir: ctx.temp_dir,
                cancel: ctx.cancel,
                progress: &NullProgress,
            };
            let res = opt.optimise(&job);
            let size = std::fs::metadata(&dst).map(|m| m.len()).ok();
            let _ = std::fs::remove_file(&dst);
            let out = res?;
            if let Some(reason) = out.skipped {
                return Err(Error::Conflict(format!("skipped: {reason}")));
            }
            let after = size.ok_or_else(|| Error::Internal("the optimiser wrote no file".into()))?;
            if after >= before {
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
