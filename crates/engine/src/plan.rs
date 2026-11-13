//! Plan commands ("Your plan"). Owner: W3-D.
//! Wave 1: in-memory CRUD with estimates taken from the opportunity cards.

use mj_core::{
    lock, Action, ChecklistItem, ChecklistLevel, Error, Estimate, OpportunityId, Plan, PlanStep, Result, SafetyClass,
    StepId, Timestamp,
};

use crate::api::{Event, PlanAddStepRequest, PlanReorderRequest, PlanUpdateStepRequest, StepRequest};
use crate::Engine;

/// Plan state of the session.
#[derive(Debug, Default)]
pub struct PlanState {
    pub plan: Plan,
}

fn summary(action: &Action) -> String {
    match action {
        Action::Trash => "To Recycle Bin".into(),
        Action::Quarantine { purge_after_days } => format!("Quarantine {purge_after_days} days"),
        Action::PermanentDelete => "Delete permanently".into(),
        Action::Move { dest, .. } => format!("Move to {}", dest.display()),
        Action::Copy { dest, .. } => format!("Copy to {}", dest.display()),
        Action::Hardlink => "Replace with hard links".into(),
        Action::Reflink => "Replace with reflinks".into(),
        Action::Dehydrate => "Make online-only".into(),
        Action::Optimise { .. } => "Optimise".into(),
        Action::Archive { .. } => "Archive".into(),
        Action::FsCompress { .. } => "Compress transparently".into(),
        Action::FsDecompress => "Decompress".into(),
        Action::None => "Review only".into(),
    }
}

/// Recompute totals and the "Before you apply" checklist.
pub(crate) fn refresh(plan: &mut Plan, undo_days: u32) {
    plan.estimate = plan.steps.iter().fold(Estimate::default(), |acc, s| acc.add(&s.estimate));
    plan.updated = Timestamp::now();
    let mut list = Vec::new();
    if !plan.steps.is_empty() {
        list.push(ChecklistItem {
            level: ChecklistLevel::Ok,
            code: "undo_window".into(),
            text: format!("Every reversible step can be undone from History for {undo_days} days"),
        });
    }
    let confirm = plan.steps.iter().filter(|s| s.safety.requires_confirmation()).count();
    if confirm > 0 {
        list.push(ChecklistItem {
            level: ChecklistLevel::Warning,
            code: "confirmation_steps".into(),
            text: format!("{confirm} step{} will ask for confirmation", if confirm == 1 { "" } else { "s" }),
        });
    }
    plan.checklist = list;
}

impl Engine {
    fn plan_changed(&self, plan: Plan) -> Result<Plan> {
        self.emit(Event::PlanChanged(plan.clone()));
        Ok(plan)
    }

    fn undo_days(&self) -> u32 {
        lock(&self.settings).general.undo_days
    }

    /// The plan.
    pub fn plan_get(&self) -> Result<Plan> {
        Ok(lock(&self.plan).plan.clone())
    }

    /// Add an opportunity as a step.
    pub fn plan_add_step(&self, req: PlanAddStepRequest) -> Result<Plan> {
        let (title, default_action, params, estimate, safety_default) = {
            let results = self.results.read().unwrap_or_else(|e| e.into_inner());
            let rec = results.get(&req.opportunity_id).ok_or_else(|| crate::results::unknown(&req.opportunity_id))?;
            let o = &rec.opportunity;
            if !o.availability.is_available() {
                return Err(Error::unsupported(o.title.clone(), "this opportunity is disabled here"));
            }
            let est = Estimate {
                files: o.selected,
                bytes_before: o.size_before,
                bytes_after: o.size_before.saturating_sub(o.saving),
                confidence: o.confidence,
                skipped: 0,
            };
            (o.title.clone(), o.default_action.clone(), o.params.clone(), est, o.safety)
        };
        let action = req.action.unwrap_or(default_action);
        let params = req.params.map(|p| params.merged(&p)).unwrap_or(params);
        let lossy = safety_default == SafetyClass::LossyOptIn;
        let safety = if matches!(action, Action::None) { SafetyClass::ReviewOnly } else { action.safety(lossy) };
        let undo_days = self.undo_days();
        let plan = {
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
                action,
                params,
                selection: Default::default(),
                estimate,
                safety,
                lossy,
            });
            refresh(&mut st.plan, undo_days);
            st.plan.clone()
        };
        self.plan_changed(plan)
    }

    /// Change a step's action or parameters.
    pub fn plan_update_step(&self, req: PlanUpdateStepRequest) -> Result<Plan> {
        let undo_days = self.undo_days();
        let plan = {
            let mut st = lock(&self.plan);
            let step =
                st.plan.steps.iter_mut().find(|s| s.id == req.step_id).ok_or_else(|| unknown_step(req.step_id))?;
            if let Some(a) = req.action {
                step.summary = summary(&a);
                step.safety = if matches!(a, Action::None) { SafetyClass::ReviewOnly } else { a.safety(step.lossy) };
                step.action = a;
            }
            if let Some(p) = req.params {
                step.params = step.params.merged(&p);
            }
            refresh(&mut st.plan, undo_days);
            st.plan.clone()
        };
        self.plan_changed(plan)
    }

    /// Remove a step.
    pub fn plan_remove_step(&self, req: StepRequest) -> Result<Plan> {
        let undo_days = self.undo_days();
        let plan = {
            let mut st = lock(&self.plan);
            let before = st.plan.steps.len();
            st.plan.steps.retain(|s| s.id != req.step_id);
            if st.plan.steps.len() == before {
                return Err(unknown_step(req.step_id));
            }
            refresh(&mut st.plan, undo_days);
            st.plan.clone()
        };
        self.plan_changed(plan)
    }

    /// Reorder steps (every id exactly once).
    pub fn plan_reorder(&self, req: PlanReorderRequest) -> Result<Plan> {
        let undo_days = self.undo_days();
        let plan = {
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
            refresh(&mut st.plan, undo_days);
            st.plan.clone()
        };
        self.plan_changed(plan)
    }

    /// Remove every step.
    pub fn plan_clear(&self) -> Result<Plan> {
        let undo_days = self.undo_days();
        let plan = {
            let mut st = lock(&self.plan);
            st.plan.steps.clear();
            refresh(&mut st.plan, undo_days);
            st.plan.clone()
        };
        self.plan_changed(plan)
    }

    /// True when the opportunity is in the plan.
    pub(crate) fn in_plan(&self, id: &OpportunityId) -> Option<StepId> {
        lock(&self.plan).plan.steps.iter().find(|s| &s.opportunity == id).map(|s| s.id)
    }
}

/// Error for an unknown step id.
pub(crate) fn unknown_step(id: StepId) -> Error {
    Error::InvalidInput(format!("unknown plan step {}", id.0))
}
