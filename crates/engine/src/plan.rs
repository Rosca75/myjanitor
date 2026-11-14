//! Plan commands ("Your plan"). Owner: W3-D.
//!
//! Lock order (deadlock rule for the whole engine): `results` before `plan` before
//! `jobs`. Plan mutations lock the plan, change its structure and release it; then
//! [`Engine::refresh_plan`] reads the results and recomputes every step estimate, the
//! step safety classes and the "Before you apply" checklist from real state.

use std::collections::BTreeMap;

use mj_core::{
    lock, Action, Badges, ChecklistItem, ChecklistLevel, Confidence, Error, Estimate, OpportunityId, Plan, PlanStep,
    Result, SafetyClass, SelectionOverrides, StepId, Timestamp,
};

use crate::api::{Event, PlanAddStepRequest, PlanReorderRequest, PlanUpdateStepRequest, StepRequest};
use crate::results::{blocked_reason, candidate_safety, is_selected, severity, skip_badges, OpportunityRecord};
use crate::Engine;

/// Plan state of the session.
#[derive(Debug, Default)]
pub struct PlanState {
    pub plan: Plan,
    /// Exact estimates measured by the last dry run, valid while the step's action and
    /// selection are unchanged.
    pub measured: BTreeMap<StepId, (Action, SelectionOverrides, Estimate)>,
}

/// One-line summary of an action.
pub(crate) fn summary(action: &Action) -> String {
    match action {
        Action::Trash => {
            if cfg!(windows) {
                "To MyJanitor's quarantine (restorable)".into()
            } else {
                "To Trash".into()
            }
        }
        Action::Quarantine { purge_after_days } => format!("Quarantine {purge_after_days} days"),
        Action::PermanentDelete => "Delete permanently".into(),
        Action::Move { dest, .. } => format!("Move to {}", dest.display()),
        Action::Copy { dest, .. } => format!("Copy to {}", dest.display()),
        Action::Hardlink => "Replace with hard links".into(),
        Action::Reflink => "Replace with reflinks".into(),
        Action::Dehydrate => "Make online-only".into(),
        Action::Optimise { originals, .. } => format!("Optimise · originals {}", originals_label(originals)),
        Action::Archive { .. } => "Archive".into(),
        Action::FsCompress { .. } => "Compress transparently".into(),
        Action::FsDecompress => "Decompress".into(),
        Action::None => "Review only".into(),
    }
}

fn originals_label(p: &mj_core::OriginalsPolicy) -> String {
    match p {
        mj_core::OriginalsPolicy::RecycleBin => "to Recycle Bin".into(),
        mj_core::OriginalsPolicy::Quarantine => "to quarantine".into(),
        mj_core::OriginalsPolicy::BackupFolder { folder } => format!("to {}", folder.display()),
        mj_core::OriginalsPolicy::Delete => "deleted".into(),
    }
}

/// True for actions that remove the item from its place.
pub(crate) fn removes(a: &Action) -> bool {
    matches!(a, Action::Trash | Action::Quarantine { .. } | Action::PermanentDelete | Action::Move { .. })
}

/// Estimate of a step from its record's selection.
pub(crate) fn step_estimate(rec: &OpportunityRecord, skip: Badges) -> Estimate {
    let mut e = Estimate { confidence: Confidence::Exact, ..Estimate::default() };
    for c in &rec.candidates {
        if is_selected(rec, c, skip) {
            e.files += 1;
            if !c.badges.contains(Badges::ONLINE_ONLY) {
                e.bytes_before += c.size;
                e.bytes_after += c.size_after.min(c.size);
            }
            e.confidence = e.confidence.max(c.confidence);
        } else if blocked_reason(c, skip).is_some() && c.default_selected {
            e.skipped += 1;
        }
    }
    e
}

/// Safety class of a step: the action's class, escalated by any selected row whose own
/// class requires confirmation.
pub(crate) fn step_safety(action: &Action, lossy: bool, rec: Option<&OpportunityRecord>, skip: Badges) -> SafetyClass {
    let mut s = if matches!(action, Action::None) { SafetyClass::ReviewOnly } else { action.safety(lossy) };
    if let Some(rec) = rec {
        for c in &rec.candidates {
            let cs = candidate_safety(rec, c);
            if c.safety_override().is_some()
                && cs.requires_confirmation()
                && is_selected(rec, c, skip)
                && severity(cs) > severity(s)
            {
                s = cs;
            }
        }
    }
    s
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Engine {
    pub(crate) fn plan_changed(&self, plan: Plan) -> Result<Plan> {
        self.emit(Event::PlanChanged(plan.clone()));
        Ok(plan)
    }

    /// Recompute estimates, safety and the checklist; returns the new plan.
    pub(crate) fn refresh_plan(&self) -> Plan {
        let settings = lock(&self.settings).clone();
        let skip = skip_badges(&settings);
        let sync_known = self.platform().sync_roots().is_ok();
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let mut st = lock(&self.plan);
        let mut confidential = 0u64;
        let mut protected = 0u64;
        let mut synced = 0u64;
        let mut online_only = 0u64;
        let mut empty_steps = Vec::new();
        let measured = st.measured.clone();
        for step in &mut st.plan.steps {
            let rec = results.get(&step.opportunity);
            step.safety = step_safety(&step.action, step.lossy, rec, skip);
            step.summary = summary(&step.action);
            match rec {
                Some(rec) => {
                    step.selection = rec.selection.clone();
                    step.estimate = match measured.get(&step.id) {
                        Some((a, sel, est)) if *a == step.action && *sel == rec.selection => *est,
                        _ => step_estimate(rec, skip),
                    };
                    if !removes(&step.action) && !matches!(step.action, Action::None) {
                        // Copies free nothing; optimisations keep their estimate.
                        if matches!(step.action, Action::Copy { .. }) {
                            step.estimate.bytes_after = step.estimate.bytes_before;
                        }
                    }
                    for c in &rec.candidates {
                        let locked = c.badges & skip;
                        if !locked.is_empty() {
                            confidential += 1;
                        } else if c.badges.contains(Badges::PROTECTED) {
                            protected += 1;
                        }
                        if is_selected(rec, c, skip) {
                            if c.badges.contains(Badges::SYNCED) && removes(&step.action) {
                                synced += 1;
                            }
                            if c.badges.contains(Badges::ONLINE_ONLY) {
                                online_only += 1;
                            }
                        }
                    }
                    if step.estimate.files == 0 {
                        empty_steps.push(step.title.clone());
                    }
                }
                None => {
                    step.estimate = Estimate::default();
                    empty_steps.push(step.title.clone());
                }
            }
        }
        drop(results);
        let plan = &mut st.plan;
        plan.estimate = plan.steps.iter().fold(Estimate::default(), |acc, s| acc.add(&s.estimate));
        if plan.steps.is_empty() {
            plan.estimate.confidence = Confidence::Exact;
        }
        plan.updated = Timestamp::now();
        let mut list = Vec::new();
        if plan.steps.iter().any(|s| s.safety.is_undoable() && s.safety != SafetyClass::ReviewOnly) {
            list.push(ChecklistItem {
                level: ChecklistLevel::Ok,
                code: "undo_window".into(),
                text: format!(
                    "Every reversible step can be undone from History for {} days",
                    settings.general.undo_days
                ),
            });
        }
        let confirm = plan.steps.iter().filter(|s| s.safety.requires_confirmation()).count() as u64;
        if confirm > 0 {
            list.push(ChecklistItem {
                level: ChecklistLevel::Warning,
                code: "confirmation_steps".into(),
                text: format!("{} will ask for confirmation", plural(confirm, "step", "steps")),
            });
        }
        let permanent = plan.steps.iter().filter(|s| s.safety == SafetyClass::Permanent).count() as u64;
        if permanent > 0 {
            list.push(ChecklistItem {
                level: ChecklistLevel::Warning,
                code: "permanent_steps".into(),
                text: format!("{} cannot be undone", plural(permanent, "step", "steps")),
            });
        }
        if confidential > 0 {
            list.push(ChecklistItem {
                level: ChecklistLevel::Ok,
                code: "skipped_confidential".into(),
                text: format!(
                    "{} signed, sensitivity-labelled or encrypted {} skipped",
                    confidential,
                    if confidential == 1 { "file is" } else { "files are" }
                ),
            });
        }
        if protected > 0 {
            list.push(ChecklistItem {
                level: ChecklistLevel::Ok,
                code: "skipped_protected".into(),
                text: format!("{} in protected locations {} never touched", plural(protected, "item", "items"), {
                    if protected == 1 {
                        "is"
                    } else {
                        "are"
                    }
                }),
            });
        }
        if synced > 0 {
            list.push(ChecklistItem {
                level: ChecklistLevel::Warning,
                code: "synced_files".into(),
                text: format!(
                    "{} in synced folders: removing them also removes them in the cloud (you can make them online-only instead)",
                    plural(synced, "item is", "items are")
                ),
            });
        }
        if !sync_known && plan.steps.iter().any(|s| removes(&s.action)) {
            list.push(ChecklistItem {
                level: ChecklistLevel::Warning,
                code: "sync_unknown".into(),
                text: "Synced folders could not be detected on this computer: check that nothing you remove is synced"
                    .into(),
            });
        }
        if online_only > 0 {
            list.push(ChecklistItem {
                level: ChecklistLevel::Info,
                code: "online_only".into(),
                text: format!("{} online-only: they free no space here", plural(online_only, "item is", "items are")),
            });
        }
        if !empty_steps.is_empty() {
            list.push(ChecklistItem {
                level: ChecklistLevel::Info,
                code: "empty_steps".into(),
                text: format!("Nothing is selected in: {}", empty_steps.join(", ")),
            });
        }
        plan.checklist = list;
        plan.clone()
    }

    /// After a rescan: keep the steps whose opportunity still exists (their per-row
    /// selection resets with the new candidate ids), drop the others.
    pub(crate) fn plan_after_rescan(&self) {
        let ids: Vec<OpportunityId> = {
            let r = self.results.read().unwrap_or_else(|e| e.into_inner());
            r.opportunities
                .iter()
                .filter(|o| o.opportunity.availability.is_available())
                .map(|o| o.opportunity.id.clone())
                .collect()
        };
        {
            let mut st = lock(&self.plan);
            st.plan.steps.retain(|s| ids.contains(&s.opportunity));
            st.measured.clear();
        }
        let plan = self.refresh_plan();
        let _ = self.plan_changed(plan);
    }

    /// The plan.
    pub fn plan_get(&self) -> Result<Plan> {
        Ok(lock(&self.plan).plan.clone())
    }

    /// Add an opportunity as a step.
    pub fn plan_add_step(&self, req: PlanAddStepRequest) -> Result<Plan> {
        let (title, default_action, params, safety_default, available) = {
            let results = self.results.read().unwrap_or_else(|e| e.into_inner());
            let rec = results.get(&req.opportunity_id).ok_or_else(|| crate::results::unknown(&req.opportunity_id))?;
            let o = &rec.opportunity;
            if !o.availability.is_available() {
                return Err(Error::unsupported(o.title.clone(), "this opportunity is disabled here"));
            }
            (o.title.clone(), o.default_action.clone(), o.params.clone(), o.safety, o.available_actions.clone())
        };
        let action = req.action.unwrap_or(default_action);
        if !matches!(action, Action::None) && !available.contains(&action.kind()) {
            return Err(Error::InvalidInput(format!("{title} cannot use the action `{:?}`", action.kind())));
        }
        let params = req.params.map(|p| params.merged(&p)).unwrap_or(params);
        let lossy = safety_default == SafetyClass::LossyOptIn;
        {
            let mut st = lock(&self.plan);
            if st.plan.steps.iter().any(|s| s.opportunity == req.opportunity_id) {
                return Err(Error::Conflict(format!("{title} is already in the plan")));
            }
            if st.plan.id == 0 {
                st.plan.id = self.next_id();
                st.plan.name = "Your plan".into();
            }
            st.plan.steps.push(PlanStep {
                id: StepId(self.next_id()),
                opportunity: req.opportunity_id,
                title,
                summary: summary(&action),
                safety: action.safety(lossy),
                action,
                params,
                selection: Default::default(),
                estimate: Estimate::default(),
                lossy,
            });
        }
        let plan = self.refresh_plan();
        self.plan_changed(plan)
    }

    /// Change a step's action or parameters.
    pub fn plan_update_step(&self, req: PlanUpdateStepRequest) -> Result<Plan> {
        let opp = {
            let st = lock(&self.plan);
            st.plan.steps.iter().find(|s| s.id == req.step_id).map(|s| s.opportunity.clone())
        }
        .ok_or_else(|| unknown_step(req.step_id))?;
        if let Some(a) = &req.action {
            let results = self.results.read().unwrap_or_else(|e| e.into_inner());
            if let Some(rec) = results.get(&opp) {
                if !matches!(a, Action::None) && !rec.opportunity.available_actions.contains(&a.kind()) {
                    return Err(Error::InvalidInput(format!("this step cannot use the action `{:?}`", a.kind())));
                }
            }
        }
        {
            let mut st = lock(&self.plan);
            let step =
                st.plan.steps.iter_mut().find(|s| s.id == req.step_id).ok_or_else(|| unknown_step(req.step_id))?;
            if let Some(a) = req.action {
                step.action = a;
            }
            if let Some(p) = req.params {
                step.params = step.params.merged(&p);
            }
        }
        let plan = self.refresh_plan();
        self.plan_changed(plan)
    }

    /// Remove a step.
    pub fn plan_remove_step(&self, req: StepRequest) -> Result<Plan> {
        {
            let mut st = lock(&self.plan);
            let before = st.plan.steps.len();
            st.plan.steps.retain(|s| s.id != req.step_id);
            if st.plan.steps.len() == before {
                return Err(unknown_step(req.step_id));
            }
            st.measured.remove(&req.step_id);
        }
        let plan = self.refresh_plan();
        self.plan_changed(plan)
    }

    /// Reorder steps (every id exactly once).
    pub fn plan_reorder(&self, req: PlanReorderRequest) -> Result<Plan> {
        {
            let mut st = lock(&self.plan);
            let mut ids: Vec<StepId> = st.plan.steps.iter().map(|s| s.id).collect();
            let mut want = req.step_ids.clone();
            ids.sort();
            want.sort();
            if ids != want {
                return Err(Error::InvalidInput("step_ids must list every plan step exactly once".into()));
            }
            let mut steps = std::mem::take(&mut st.plan.steps);
            for id in &req.step_ids {
                if let Some(pos) = steps.iter().position(|s| s.id == *id) {
                    st.plan.steps.push(steps.remove(pos));
                }
            }
        }
        let plan = self.refresh_plan();
        self.plan_changed(plan)
    }

    /// Remove every step.
    pub fn plan_clear(&self) -> Result<Plan> {
        {
            let mut st = lock(&self.plan);
            st.plan.steps.clear();
            st.measured.clear();
        }
        let plan = self.refresh_plan();
        self.plan_changed(plan)
    }

    /// The plan step of an opportunity, if any.
    pub(crate) fn in_plan(&self, id: &OpportunityId) -> Option<StepId> {
        lock(&self.plan).plan.steps.iter().find(|s| &s.opportunity == id).map(|s| s.id)
    }
}

/// Error for an unknown step id.
pub(crate) fn unknown_step(id: StepId) -> Error {
    Error::InvalidInput(format!("unknown plan step {}", id.0))
}
