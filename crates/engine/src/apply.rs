//! Dry run, confirmation dialog data and apply (OPT-ENG-03, SAF-10/11). Owner: W3-D.
//!
//! Safety rules enforced here (and again inside `mj-actions`):
//!
//! - `plan_apply` refuses with `confirmation_required`, before anything runs, unless
//!   every step whose safety class requires it (`lossy_opt_in`, `permanent`, also when
//!   the dialog's originals choice makes it permanent) has a `StepConfirmation` with
//!   `acknowledged: true`; only those steps run with `ExecOptions::confirmed`.
//! - Items are the selected, actionable rows only, with their scan stamp and file id;
//!   online-only rows are never removed (that would delete the cloud copy).
//! - The guard's allowed roots are the roots of the scanned index; MyJanitor's own
//!   folders are protected.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use mj_actions::{ExecContext, ExecOptions, Guard, ProtectedPaths, Services};
use mj_core::{
    lock, Action, ActionItem, Badges, CancelToken, Confidence, Error, Estimate, FileStamp, ItemStatus, Job, JobId,
    JobKind, JobOutcome, JournalState, OriginalsPolicy, PlanStep, ProgressSink, Result, RunId, RunKind, RunRecord,
    SafetyClass, StepConfirmation, StepId, TargetKind, Timestamp,
};

use crate::api::{
    ApplyStarted, ConfirmationInfo, DryRunReport, DryRunStep, Event, HistoryChanged, OpportunitiesChanged,
    OriginalsOption, PlanApplyRequest, StepRequest, SyncedWarning,
};
use crate::jobs::{JobResult, Work};
use crate::plan::removes;
use crate::results::{is_selected, skip_badges};
use crate::Engine;

/// A step expanded into action items.
#[derive(Debug, Clone)]
pub(crate) struct StepWork {
    pub step: PlanStep,
    pub items: Vec<ActionItem>,
    pub roots: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// Bytes the index says each item occupies, and its (file id, link count).
    pub sizes: Vec<(u64, Option<(mj_core::FileId, u32)>)>,
}

/// The action a step runs with the dialog's originals choice applied.
fn with_originals(action: &Action, originals: Option<&OriginalsPolicy>) -> Action {
    let mut a = action.clone();
    if let Some(o) = originals {
        match &mut a {
            Action::Optimise { originals, .. } | Action::Archive { originals, .. } => *originals = o.clone(),
            _ => {}
        }
    }
    a
}

/// True when the step needs the user's confirmation with this action.
fn needs_confirmation(step: &PlanStep, action: &Action) -> bool {
    step.safety.requires_confirmation()
        || action.safety(step.lossy).requires_confirmation()
        || mj_actions::engine::requires_confirmation(action)
}

impl Engine {
    /// Run `f` with an execution context for `roots`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_exec<R>(
        &self,
        roots: Vec<PathBuf>,
        run_id: RunId,
        job_id: JobId,
        step_id: Option<StepId>,
        cancel: &CancelToken,
        progress: &dyn ProgressSink,
        f: impl FnOnce(&ExecContext<'_>) -> R,
    ) -> R {
        let settings = lock(&self.settings).clone();
        let protected = ProtectedPaths::new(&settings.safety.protected_paths).with_app_dirs(&self.cfg.app_dirs());
        let guard = Guard::new(roots, protected);
        let tools = lock(&self.tools).clone();
        let _ = std::fs::create_dir_all(&self.cfg.temp_dir);
        let ctx = ExecContext {
            platform: self.platform(),
            guard: &guard,
            journal: &self.journal,
            services: Services { optimisers: &self.optimisers, archiver: Some(self.archiver.as_ref()), tools: &tools },
            settings: &settings,
            quarantine_dir: &self.cfg.quarantine_dir,
            temp_dir: &self.cfg.temp_dir,
            run_id,
            job_id,
            step_id,
            cancel,
            progress,
        };
        f(&ctx)
    }

    pub(crate) fn exec_options(&self, confirmed: bool) -> ExecOptions {
        ExecOptions { confirmed, trash_route: self.cfg.trash_route, ..ExecOptions::default() }
    }

    /// Expand a plan step into action items from the current selection.
    pub(crate) fn build_step(&self, step: &PlanStep, conf: Option<&StepConfirmation>) -> Result<StepWork> {
        let skip = skip_badges(&lock(&self.settings));
        let inspector_roots = self.platform().sync_roots().unwrap_or_default();
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let index = results.index.clone().ok_or_else(|| Error::InvalidInput("scan first".into()))?;
        let rec = results.get(&step.opportunity).ok_or_else(|| crate::results::unknown(&step.opportunity))?;
        let action = with_originals(&step.action, conf.map(|c| &c.originals));
        let mut work = StepWork {
            step: step.clone(),
            items: Vec::new(),
            roots: index.roots.clone(),
            warnings: Vec::new(),
            sizes: Vec::new(),
        };
        work.step.action = action.clone();
        if matches!(action, Action::None) {
            work.warnings.push("Review-only step: nothing is changed".into());
            return Ok(work);
        }
        let (mut online, mut no_keeper, mut outside) = (0u64, 0u64, 0u64);
        let locked = rec.candidates.iter().filter(|c| c.default_selected && !(c.badges & skip).is_empty()).count();
        for c in rec.candidates.iter().filter(|c| is_selected(rec, c, skip)) {
            if c.badges.contains(Badges::ONLINE_ONLY) && !matches!(action, Action::Copy { .. }) {
                online += 1;
                continue;
            }
            let Some(root) = index.roots.iter().find(|r| mj_core::pathutil::is_within(r, &c.path)) else {
                outside += 1;
                continue;
            };
            let link_source = if matches!(action, Action::Hardlink | Action::Reflink) {
                match c.group.and_then(|g| rec.groups.iter().find(|x| x.id == g)).and_then(|g| g.keeper.clone()) {
                    Some(k) if k != c.path => Some(k),
                    _ => {
                        no_keeper += 1;
                        continue;
                    }
                }
            } else {
                None
            };
            let mut item_action = action.clone();
            let synced = c.badges.contains(Badges::SYNCED)
                || inspector_roots.iter().any(|r| mj_core::pathutil::is_within(&r.path, &c.path));
            if conf.is_some_and(|x| x.synced_online_only) && synced && removes(&action) {
                item_action = Action::Dehydrate;
            }
            let entry = index.get(&c.path);
            work.items.push(ActionItem {
                path: c.path.clone(),
                target: c.target,
                action: item_action,
                link_source,
                root: root.clone(),
                expected: if c.target == TargetKind::Folder {
                    FileStamp { size: 0, mtime: Timestamp(0) }
                } else {
                    c.stamp
                },
                expected_id: entry.and_then(|e| e.file_id),
            });
            work.sizes
                .push((entry.map(|e| e.reclaimable()).unwrap_or(c.size), entry.and_then(|e| e.file_id.zip(e.links))));
        }
        let mut warn = |n: u64, what: &str| {
            if n > 0 {
                work.warnings.push(format!("{n} {what}"));
            }
        };
        warn(locked as u64, "signed, labelled or encrypted file(s) skipped");
        warn(online, "online-only item(s) skipped: removing them would delete the cloud copy");
        warn(no_keeper, "item(s) skipped: no copy to link to");
        warn(outside, "item(s) skipped: outside the scanned locations");
        Ok(work)
    }

    /// Queue a dry run of the plan ("Estimate exactly").
    pub fn plan_dry_run(self: &Arc<Self>) -> Result<Job> {
        let plan = lock(&self.plan).plan.clone();
        if plan.steps.is_empty() {
            return Err(Error::InvalidInput("the plan is empty".into()));
        }
        let works: Vec<StepWork> = plan.steps.iter().map(|s| self.build_step(s, None)).collect::<Result<_>>()?;
        let total: u64 = works.iter().map(|w| w.items.len() as u64).sum();
        let job = self.new_job(JobKind::DryRun, None, None, "Dry run", total, plan.estimate.bytes_before);
        let plan_id = plan.id;
        let work: Work = Box::new(move |engine, ctx| engine.dry_run_body(plan_id, works, ctx));
        self.spawn_detached(job, work)
    }

    fn dry_run_body(&self, plan_id: u64, works: Vec<StepWork>, ctx: &crate::jobs::JobCtx<'_>) -> JobResult {
        let mut steps = Vec::new();
        let mut total = Estimate { confidence: Confidence::Exact, ..Estimate::default() };
        let mut error = None;
        for w in &works {
            if ctx.cancel.checkpoint().is_err() {
                error = Some(Error::Cancelled);
                break;
            }
            let res = self.with_exec(
                w.roots.clone(),
                RunId(0),
                ctx.job_id,
                Some(w.step.id),
                ctx.cancel,
                ctx.progress,
                |ec| mj_actions::engine::dry_run_with(ec, &w.items, &self.exec_options(true)),
            );
            match res {
                Ok(r) => {
                    let mut est = r.estimate;
                    if removes(&w.step.action) {
                        est.bytes_after =
                            est.bytes_after.saturating_add(hardlink_not_freed(w, &r.skipped)).min(est.bytes_before);
                    }
                    if matches!(w.step.action, Action::Copy { .. }) {
                        est.bytes_after = est.bytes_before;
                    }
                    total = total.add(&est);
                    let sel = self
                        .results
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&w.step.opportunity)
                        .map(|x| x.selection.clone())
                        .unwrap_or_default();
                    lock(&self.plan).measured.insert(w.step.id, (w.step.action.clone(), sel, est));
                    steps.push(DryRunStep {
                        step_id: w.step.id,
                        title: w.step.title.clone(),
                        estimate: est,
                        skipped_count: r.skipped.len() as u64,
                        skipped: r.skipped.into_iter().take(100).collect(),
                        warnings: w.warnings.clone(),
                    });
                }
                Err(Error::Cancelled) => {
                    error = Some(Error::Cancelled);
                    break;
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        let report = DryRunReport { plan_id, job_id: ctx.job_id, steps, total, error: error.as_ref().map(Error::info) };
        self.emit(Event::DryRun(report));
        let plan = self.refresh_plan();
        let _ = self.plan_changed(plan);
        JobResult { outcome: None, error, report: false }
    }

    /// Content of the confirmation dialog for one step (SAF-10/11).
    pub fn plan_confirmation_info(&self, req: StepRequest) -> Result<ConfirmationInfo> {
        let step = lock(&self.plan)
            .plan
            .steps
            .iter()
            .find(|s| s.id == req.step_id)
            .cloned()
            .ok_or_else(|| crate::plan::unknown_step(req.step_id))?;
        let settings = lock(&self.settings).clone();
        let work = self.build_step(&step, None)?;
        let e = step.estimate;
        let files = e.files;
        let permanent = step.safety == SafetyClass::Permanent
            || matches!(step.action, Action::PermanentDelete)
            || (!step.lossy && !step.action.safety(false).is_undoable());
        let roots = self.platform().sync_roots().ok();
        let mut synced_count = 0u64;
        let mut provider = String::new();
        let mut can_online_only = false;
        for it in &work.items {
            if let Some(r) = roots
                .iter()
                .flatten()
                .filter(|r| mj_core::pathutil::is_within(&r.path, &it.path))
                .max_by_key(|r| r.path.components().count())
            {
                synced_count += 1;
                if provider.is_empty() {
                    provider = r.name.clone();
                }
                can_online_only |= r.can_dehydrate && cfg!(windows);
            }
        }
        let recycle = if cfg!(windows) && self.cfg.trash_route == mj_actions::TrashRoute::Quarantine {
            "Keep in MyJanitor's quarantine (restorable, default)"
        } else {
            "Move to Recycle Bin / Trash (default)"
        };
        let has_originals = matches!(step.action, Action::Optimise { .. } | Action::Archive { .. });
        let originals_options = if has_originals {
            vec![
                OriginalsOption { policy: OriginalsPolicy::RecycleBin, label: recycle.into(), reversible: true },
                OriginalsOption {
                    policy: OriginalsPolicy::Quarantine,
                    label: format!("Keep in quarantine for {} days", settings.general.quarantine_days),
                    reversible: true,
                },
                OriginalsOption {
                    policy: OriginalsPolicy::Delete,
                    label: "Delete permanently".into(),
                    reversible: false,
                },
            ]
        } else {
            Vec::new()
        };
        let noun = if files == 1 { "file" } else { "files" };
        let (title, body, ack, label) = if permanent {
            (
                format!("Delete {files} {noun} permanently?"),
                "This cannot be undone: the files do not go to the Recycle Bin or the quarantine.".to_string(),
                "I understand these files cannot be restored".to_string(),
                format!("Delete {files} {noun}"),
            )
        } else {
            (
                format!("Compress {files} {noun}?"),
                "This is lossy: the new files are smaller but cannot be turned back into the originals.".to_string(),
                "I understand the new files are lower quality than the originals".to_string(),
                format!("Compress {files} {noun}"),
            )
        };
        Ok(ConfirmationInfo {
            step_id: step.id,
            title,
            body,
            files,
            bytes_now: e.bytes_before,
            bytes_after: e.bytes_after,
            saving: e.saving(),
            confidence: e.confidence,
            originals_options,
            default_originals: settings.general.default_originals.clone(),
            synced: (synced_count > 0).then_some(SyncedWarning { count: synced_count, provider, can_online_only }),
            acknowledge_text: ack,
            undo_text: if permanent {
                "Every change is logged in History, but permanently deleted files cannot be restored.".into()
            } else {
                format!(
                    "Every change is logged; History → Restore brings the originals back for {} days.",
                    settings.general.undo_days
                )
            },
            action_label: label,
            permanent,
        })
    }

    /// Apply the plan: one job per step, in plan order.
    pub fn plan_apply(self: &Arc<Self>, req: PlanApplyRequest) -> Result<ApplyStarted> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(Error::Conflict("MyJanitor is shutting down".into()));
        }
        let plan = lock(&self.plan).plan.clone();
        if plan.steps.is_empty() {
            return Err(Error::InvalidInput("the plan is empty".into()));
        }
        if lock(&self.jobs).jobs.iter().any(|j| j.kind == JobKind::Apply && !j.state.is_finished()) {
            return Err(Error::Conflict("a plan is already being applied".into()));
        }
        // Validate everything before anything runs.
        let mut works = Vec::new();
        for s in &plan.steps {
            let conf = req.confirmations.iter().find(|c| c.step_id == s.id && c.acknowledged);
            let action = with_originals(&s.action, conf.map(|c| &c.originals));
            let confirm = needs_confirmation(s, &action);
            if confirm && conf.is_none() {
                return Err(Error::ConfirmationRequired(format!("step {} ({}) needs confirmation", s.id.0, s.title)));
            }
            works.push((self.build_step(s, conf)?, confirm));
        }
        let now = Timestamp::now();
        let run_id = self.journal.next_run_id(now);
        let titles: Vec<String> = plan.steps.iter().map(|s| s.title.clone()).collect();
        let run = RunRecord {
            run_id,
            kind: RunKind::Apply,
            title: format!(
                "Plan · {}",
                if titles.len() == 1 { "1 step".to_string() } else { format!("{} steps", titles.len()) }
            ),
            started: now,
            finished: None,
            steps: titles,
            undo_of: None,
        };
        self.journal.put_run(&run)?;
        let roots = works.first().map(|(w, _)| w.roots.clone()).unwrap_or_default();
        self.remember_run(run_id, roots, plan.steps.iter().map(|s| (s.id.0, s.title.clone())).collect());
        let mut jobs = Vec::new();
        for (w, confirmed) in works {
            let bytes: u64 = w.sizes.iter().map(|s| s.0).sum();
            let job =
                self.new_job(JobKind::Apply, Some(run_id), Some(w.step.id), &w.step.title, w.items.len() as u64, bytes);
            let run = run.clone();
            let body: Work = Box::new(move |engine, ctx| engine.apply_body(&run, w, confirmed, ctx));
            jobs.push((job, body));
        }
        let listed: Vec<Job> = jobs.iter().map(|(j, _)| j.clone()).collect();
        self.enqueue(jobs)?;
        Ok(ApplyStarted { run_id, jobs: listed })
    }

    fn apply_body(&self, run: &RunRecord, w: StepWork, confirmed: bool, ctx: &crate::jobs::JobCtx<'_>) -> JobResult {
        let opts = self.exec_options(confirmed);
        let res =
            self.with_exec(w.roots.clone(), run.run_id, ctx.job_id, Some(w.step.id), ctx.cancel, ctx.progress, |ec| {
                mj_actions::execute_with(ec, &w.items, &opts)
            });
        let mut finished = run.clone();
        finished.finished = Some(Timestamp::now());
        let _ = self.journal.put_run(&finished);
        let (outcome, error) = match res {
            Ok(o) => (Some(o), None),
            Err(e) => (None, Some(e)),
        };
        let saved = outcome.as_ref().map(|o| o.bytes_saved).unwrap_or(0);
        self.reclaimed.fetch_add(saved, Ordering::Relaxed);
        self.after_apply_step(run.run_id, &w);
        self.emit(Event::HistoryChanged(HistoryChanged { run_id: Some(run.run_id) }));
        JobResult { outcome: outcome.or_else(|| Some(JobOutcome::default())), error, report: true }
    }

    /// Applied items leave their cards (and the index); the applied step leaves the plan.
    fn after_apply_step(&self, run_id: RunId, w: &StepWork) {
        let done: Vec<PathBuf> = self
            .journal
            .entries_of(run_id)
            .into_iter()
            .filter(|e| e.step_id == Some(w.step.id) && e.state == JournalState::Done)
            .map(|e| e.original)
            .collect();
        let skip = skip_badges(&lock(&self.settings));
        let generation = {
            let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
            if !done.is_empty() {
                let gone = |p: &Path| done.iter().any(|d| mj_core::pathutil::is_within(d, p));
                for rec in &mut results.opportunities {
                    let before = rec.candidates.len();
                    rec.candidates.retain(|c| !gone(&c.path));
                    if rec.candidates.len() != before {
                        for g in &mut rec.groups {
                            g.members.retain(|m| !gone(m));
                        }
                        crate::opportunities::recompute(rec, skip);
                    }
                }
            }
            results.generation
        };
        {
            let mut st = lock(&self.plan);
            st.plan.steps.retain(|s| s.id != w.step.id);
            st.measured.remove(&w.step.id);
        }
        let plan = self.refresh_plan();
        let _ = self.plan_changed(plan);
        self.emit(Event::OpportunitiesChanged(OpportunitiesChanged { generation }));
    }
}

/// Bytes a removal does not free because other hard links to the same file stay
/// (only some of a file's links are in the step), or counted once per link when all are.
fn hardlink_not_freed(w: &StepWork, skipped: &[mj_core::ItemOutcome]) -> u64 {
    let mut by_id: HashMap<mj_core::FileId, (u32, u32, u64)> = HashMap::new();
    for (item, (size, id)) in w.items.iter().zip(&w.sizes) {
        if skipped.iter().any(|s| s.path == item.path && s.status != ItemStatus::Done) {
            continue;
        }
        if let Some((id, links)) = id {
            if *links > 1 {
                let e = by_id.entry(*id).or_insert((*links, 0, *size));
                e.1 += 1;
            }
        }
    }
    by_id
        .values()
        .map(|(links, n, size)| if n < links { size * u64::from(*n) } else { size * u64::from(n.saturating_sub(1)) })
        .sum()
}
