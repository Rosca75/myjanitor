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

use std::path::Path;

use mj_core::{
    ActionItem, Archiver, CancelToken, Error, Estimate, ExternalTool, ItemOutcome, JobId, JobOutcome, Optimiser,
    ProgressSink, Result, RunId, Settings, StepId,
};
use mj_platform::Platform;

use crate::guard::Guard;
use crate::journal::Journal;

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

/// Apply `items`.
pub fn execute(ctx: &ExecContext<'_>, items: &[ActionItem]) -> Result<JobOutcome> {
    let _ = (ctx, items);
    Err(Error::not_implemented("action engine (W2-E)"))
}

/// Check and measure `items` without changing anything.
pub fn dry_run(ctx: &ExecContext<'_>, items: &[ActionItem]) -> Result<DryRunResult> {
    let _ = (ctx, items);
    Err(Error::not_implemented("dry run (W2-E)"))
}
