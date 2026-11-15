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
    lock, Action, ActionItem, Badges, CancelToken, Confidence, Error, Estimate, FileStamp, GroupKind, ItemStatus, Job,
    JobId, JobKind, JobOutcome, JournalState, KeepCopy, OriginalsPolicy, PlanStep, ProgressSink, Result, RunId,
    RunKind, RunRecord, SafetyClass, StepConfirmation, StepId, TargetKind, Timestamp,
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
        let (mut online, mut no_keeper, mut outside, mut last_copy) = (0u64, 0u64, 0u64, 0u64);
        // F-02: removing group members (duplicates, similar images, versions...) always
        // leaves one member of each group; a group whose every member is selected is
        // skipped whole and reported.
        let survivors = if crate::results::removes_copy(&action) {
            crate::results::group_survivors(rec, skip)
        } else {
            Default::default()
        };
        let mut warned: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        let locked = rec.candidates.iter().filter(|c| !(c.badges & skip).is_empty()).count();
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
            let keep = match c.group.and_then(|g| survivors.get(&g).map(|s| (g, s))) {
                Some((_, None)) => {
                    last_copy += 1;
                    continue;
                }
                Some((g, Some(k))) => {
                    let exact = rec.groups.iter().find(|x| x.id == g).is_some_and(|x| x.kind == GroupKind::Exact);
                    // A survivor the index does not know cannot be verified: skip.
                    let Some(ke) = index.get(k) else {
                        last_copy += 1;
                        continue;
                    };
                    Some(KeepCopy { path: k.clone(), expected: ke.stamp(), expected_id: ke.file_id, identical: exact })
                }
                None => None,
            };
            let mut item_action = action.clone();
            let synced = c.badges.contains(Badges::SYNCED)
                || inspector_roots.iter().any(|r| mj_core::pathutil::is_within(&r.path, &c.path));
            if conf.is_some_and(|x| x.synced_online_only) && synced && removes(&action) {
                item_action = Action::Dehydrate;
            }
            if let Some(w) = c.columns.get("warning").filter(|w| !w.is_empty()) {
                *warned.entry(w.clone()).or_default() += 1;
            }
            let entry = index.get(&c.path);
            let mut item = ActionItem::new(
                c.path.clone(),
                c.target,
                item_action,
                root.clone(),
                if c.target == TargetKind::Folder { FileStamp { size: 0, mtime: Timestamp(0) } } else { c.stamp },
            );
            item.link_source = link_source;
            item.keep = keep;
            item.expected_id = entry.and_then(|e| e.file_id);
            // F-04: a folder carries its subtree as scanned; the action engine refuses it
            // when anything below it changed (or when the scan did not see it whole).
            if c.target == TargetKind::Folder {
                item.expected_tree = index.position(&c.path).map(|i| index.tree_stamp(i));
            }
            // F-01: only a regenerable artefact folder (dev artefacts, caches) owns the
            // version-control metadata inside it.
            item.owned_tree =
                c.target == TargetKind::Folder && crate::results::candidate_safety(rec, c) == SafetyClass::Rebuildable;
            work.items.push(item);
            work.sizes.push((
                if c.badges.contains(Badges::ONLINE_ONLY) { 0 } else { c.size },
                entry.and_then(|e| e.file_id.zip(e.links)),
            ));
        }
        let mut warn = |n: u64, what: &str| {
            if n > 0 {
                work.warnings.push(format!("{n} {what}"));
            }
        };
        warn(locked as u64, "signed, labelled or encrypted file(s) skipped");
        warn(online, "online-only item(s) skipped: removing them would delete the cloud copy");
        warn(no_keeper, "item(s) skipped: no copy to link to");
        warn(last_copy, "item(s) skipped: every copy of their group was selected, so one is always kept");
        warn(outside, "item(s) skipped: outside the scanned locations");
        for (text, n) in warned {
            work.warnings.push(format!("{n} item(s): {text}"));
        }
        Ok(work)
    }

    /// Queue a dry run of the plan ("Estimate exactly").
    pub fn plan_dry_run(self: &Arc<Self>) -> Result<Job> {
        let plan = lock(&self.plan).plan.clone();
        if plan.steps.is_empty() {
            return Err(Error::InvalidInput("the plan is empty".into()));
        }
        let mut works: Vec<StepWork> = plan.steps.iter().map(|s| self.build_step(s, None)).collect::<Result<_>>()?;
        skip_kept_copies(works.iter_mut());
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
                        // Removals: the scan's allocated sizes (folders = subtree totals) of the
                        // items that pass every check, minus what other hard links keep.
                        let mut before = 0u64;
                        for (item, (size, _)) in w.items.iter().zip(&w.sizes) {
                            if !r.skipped.iter().any(|s| s.path == item.path) {
                                before += size;
                            }
                        }
                        est.bytes_before = before;
                        est.bytes_after = hardlink_not_freed(w, &r.skipped).min(before);
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
        let plan = self.refresh_plan();
        let _ = self.plan_changed(plan);
        self.emit(Event::DryRun(report));
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
            || (!step.lossy && !matches!(step.action, Action::Hardlink) && !step.action.safety(false).is_undoable());
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
        let hardlink = matches!(step.action, Action::Hardlink);
        let (title, body, ack, label) = if hardlink {
            // F-07: why a hard link needs a confirmation, in the user's words.
            (
                format!("Replace {files} duplicate {noun} with hard links?"),
                "Each duplicate becomes a second name of the copy that is kept: they share one content from now on. \
                 Editing either one changes both, and the duplicate's own content cannot be brought back after that \
                 (Restore refuses once they differ). The duplicate's owner and permissions become the kept copy's."
                    .to_string(),
                "I understand linked files share every later change".to_string(),
                format!("Link {files} {noun}"),
            )
        } else if permanent {
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
            undo_text: if hardlink {
                format!(
                    "History → Restore separates the files again for {} days, as long as neither was edited.",
                    settings.general.undo_days
                )
            } else if permanent {
                "Every change is logged in History, but permanently deleted files cannot be restored.".into()
            } else {
                format!(
                    "Every change is logged; History → Restore brings the originals back for {} days.",
                    settings.general.undo_days
                )
            },
            action_label: label,
            permanent,
            selection_digest: selection_digest(&work),
        })
    }

    /// Apply the plan: one job per step, in plan order.
    pub fn plan_apply(self: &Arc<Self>, req: PlanApplyRequest) -> Result<ApplyStarted> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(Error::Conflict("MyJanitor is shutting down".into()));
        }
        self.require_writable()?;
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
            let work = self.build_step(s, conf)?;
            // G-04: the acknowledgement covers what the dialog showed, nothing else.
            if confirm && conf.and_then(|c| c.selection_digest.as_deref()) != Some(selection_digest(&work).as_str()) {
                return Err(Error::ConfirmationRequired(format!(
                    "step {} ({}) changed since it was confirmed (a new scan or a selection change): \
                     review it and confirm it again",
                    s.id.0, s.title
                )));
            }
            works.push((work, confirm));
        }
        // G-03: the copy one step keeps is kept by every other step of the run too.
        skip_kept_copies(works.iter_mut().map(|(w, _)| w));
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
        let done = self.after_apply_step(run.run_id, &w);
        // Reclaimed bytes: for removals the scan's allocated sizes of the items done
        // (the same figures the cards and the dry run show), else what the engine saved.
        let saved = if removes(&w.step.action) {
            w.items.iter().zip(&w.sizes).filter(|(i, _)| done.contains(&i.path)).map(|(_, s)| s.0).sum()
        } else {
            outcome.as_ref().map(|o| o.bytes_saved).unwrap_or(0)
        };
        self.reclaimed.fetch_add(saved, Ordering::Relaxed);
        self.emit(Event::HistoryChanged(HistoryChanged { run_id: Some(run.run_id) }));
        JobResult { outcome: outcome.or_else(|| Some(JobOutcome::default())), error, report: true }
    }

    /// Applied items leave their cards (and the index); the applied step leaves the plan.
    fn after_apply_step(&self, run_id: RunId, w: &StepWork) -> Vec<PathBuf> {
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
        done
    }
}

/// G-04: a digest of what a step would act on: its action and every item's path, kind,
/// scan stamp, subtree stamp and file id, in a stable order. The originals choice and the
/// synced-folder choice are the dialog's own answers and are not part of it.
pub(crate) fn selection_digest(w: &StepWork) -> String {
    let mut lines: Vec<String> = w
        .items
        .iter()
        .map(|i| {
            format!(
                "{}\t{:?}\t{}\t{}\t{:?}\t{:?}",
                mj_core::pathutil::path_key(&i.path),
                i.target,
                i.expected.size,
                i.expected.mtime.0,
                i.expected_tree,
                i.expected_id
            )
        })
        .collect();
    lines.sort();
    let mut h = mj_core::hash::Sha256::new();
    let action = with_originals(&w.step.action, Some(&OriginalsPolicy::RecycleBin));
    h.update(format!("{action:?}\n{}\n", w.step.id.0).as_bytes());
    for l in &lines {
        h.update(l.as_bytes());
        h.update(b"\n");
    }
    h.finish_hex()
}

/// True when `action`, run on a kept copy (or a folder holding one), could leave no copy of
/// its group where the user can reach it: removals, moves, archives that take their
/// sources away, and optimisations or archives that delete the originals (G-03).
fn takes_copy_away(action: &Action) -> bool {
    match action {
        Action::Trash | Action::Quarantine { .. } | Action::PermanentDelete | Action::Move { .. } => true,
        Action::Archive { keep_sources, originals, .. } => !keep_sources || *originals == OriginalsPolicy::Delete,
        Action::Optimise { originals, .. } => *originals == OriginalsPolicy::Delete,
        _ => false,
    }
}

/// G-03: "one copy is always kept" across the whole run. Every copy a step keeps (the
/// survivor of a removed duplicate, the source of a link) is collected over every step;
/// any item of any step that is such a copy, or a folder holding one, and whose action
/// would take it away, is dropped from its step and reported, whatever the step order.
pub(crate) fn skip_kept_copies<'a>(works: impl Iterator<Item = &'a mut StepWork>) {
    use mj_core::pathutil::path_key;
    let mut works: Vec<&mut StepWork> = works.collect();
    // The key of every kept copy and of each of its ancestors: an item is refused when its
    // own key is in the set (it is a kept copy, or holds one).
    let mut held: std::collections::HashSet<String> = std::collections::HashSet::new();
    for w in works.iter() {
        for i in &w.items {
            for k in i.keep.as_ref().map(|k| &k.path).into_iter().chain(i.link_source.as_ref()) {
                let mut cur = Some(k.as_path());
                while let Some(p) = cur {
                    if !held.insert(path_key(p)) {
                        break;
                    }
                    cur = p.parent();
                }
            }
        }
    }
    if held.is_empty() {
        return;
    }
    for w in works.iter_mut() {
        let mut skipped = 0u64;
        let mut items = Vec::with_capacity(w.items.len());
        let mut sizes = Vec::with_capacity(w.sizes.len());
        for (item, size) in w.items.drain(..).zip(w.sizes.drain(..)) {
            if takes_copy_away(&item.action) && held.contains(&path_key(&item.path)) {
                skipped += 1;
            } else {
                items.push(item);
                sizes.push(size);
            }
        }
        w.items = items;
        w.sizes = sizes;
        if skipped > 0 {
            w.warnings.push(format!(
                "{skipped} item(s) skipped: they are, or hold, the copy another step keeps, so one copy is always kept"
            ));
        }
    }
}

/// Bytes a removal does not free because other hard links to the same file stay
/// (only some of a file's links are in the step), or counted once per link when all are.
fn hardlink_not_freed(w: &StepWork, skipped: &[mj_core::ItemOutcome]) -> u64 {
    let mut by_id: HashMap<mj_core::FileId, (u32, u32, u64)> = HashMap::new();
    for (item, (size, id)) in w.items.iter().zip(&w.sizes) {
        if item.target != TargetKind::File
            || skipped.iter().any(|s| s.path == item.path && s.status != ItemStatus::Done)
        {
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
