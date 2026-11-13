//! Dry run, confirmation dialog data and apply (OPT-ENG-03, SAF-10/11). Owner: W3-D.
//!
//! Contract: `plan_apply` refuses with `confirmation_required` unless every step
//! whose `safety.requires_confirmation()` has a `StepConfirmation` with
//! `acknowledged: true`; it expands steps into `ActionItem`s (selected,
//! actionable candidates only), allocates a `RunId`, writes the `RunRecord`,
//! queues one job per step and runs them through `mj_actions::execute` with
//! the scope roots as the guard's allowed roots.

use std::sync::Arc;

use mj_core::{lock, Error, Job, Result};

use crate::api::{ApplyStarted, ConfirmationInfo, PlanApplyRequest, StepRequest};
use crate::Engine;

impl Engine {
    /// Queue a dry run of the plan.
    pub fn plan_dry_run(self: &Arc<Self>) -> Result<Job> {
        if lock(&self.plan).plan.steps.is_empty() {
            return Err(Error::InvalidInput("the plan is empty".into()));
        }
        Err(Error::not_implemented("dry run (W3-D)"))
    }

    /// Content of the confirmation dialog for one step.
    pub fn plan_confirmation_info(&self, req: StepRequest) -> Result<ConfirmationInfo> {
        if !lock(&self.plan).plan.steps.iter().any(|s| s.id == req.step_id) {
            return Err(crate::plan::unknown_step(req.step_id));
        }
        Err(Error::not_implemented("confirmation dialog data (W3-D)"))
    }

    /// Apply the plan.
    pub fn plan_apply(self: &Arc<Self>, req: PlanApplyRequest) -> Result<ApplyStarted> {
        let plan = lock(&self.plan).plan.clone();
        if plan.steps.is_empty() {
            return Err(Error::InvalidInput("the plan is empty".into()));
        }
        for s in plan.steps.iter().filter(|s| s.safety.requires_confirmation()) {
            let ok = req.confirmations.iter().any(|c| c.step_id == s.id && c.acknowledged);
            if !ok {
                return Err(Error::ConfirmationRequired(format!("step {} ({}) needs confirmation", s.id.0, s.title)));
            }
        }
        Err(Error::not_implemented("apply plan (W3-D)"))
    }
}
