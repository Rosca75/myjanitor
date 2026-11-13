//! Scheduled profiles (AUT-02). Owner: W3-E. Stored in `<data>/schedules.json`.
//! Running them (in-app timer while open; optional OS task registration) is W3-E's.

use std::sync::Arc;

use mj_core::{lock, Error, Result, Schedule, ScheduledStep};

use crate::api::{ApplyStarted, IdRequest, ScheduleFromPlanRequest, ScheduleSaveRequest};
use crate::Engine;

const FILE: &str = "schedules.json";

pub(crate) fn load(engine: &Engine) {
    if engine.cfg.persist {
        if let Some(v) = crate::store::read_json::<Vec<Schedule>>(&engine.cfg.data_dir.join(FILE)) {
            *lock(&engine.schedules) = v;
        }
    }
}

fn validate(s: &Schedule) -> Result<()> {
    if s.name.trim().is_empty() {
        return Err(Error::InvalidInput("a schedule needs a name".into()));
    }
    for step in &s.steps {
        if step.action.safety(false).requires_confirmation() {
            return Err(Error::InvalidInput("lossy or permanent steps cannot run unattended".into()));
        }
    }
    Ok(())
}

impl Engine {
    fn save_schedules(&self) -> Result<()> {
        if self.cfg.persist {
            crate::store::write_json(&self.cfg.data_dir.join(FILE), &*lock(&self.schedules))?;
        }
        Ok(())
    }

    /// Schedules.
    pub fn schedules_list(&self) -> Result<Vec<Schedule>> {
        Ok(lock(&self.schedules).clone())
    }

    /// Insert or replace a schedule.
    pub fn schedule_save(&self, req: ScheduleSaveRequest) -> Result<Vec<Schedule>> {
        let mut s = req.schedule;
        validate(&s)?;
        if s.id.is_empty() {
            s.id = format!("sched_{}", self.next_id());
        }
        {
            let mut v = lock(&self.schedules);
            match v.iter_mut().find(|x| x.id == s.id) {
                Some(slot) => *slot = s,
                None => v.push(s),
            }
        }
        self.save_schedules()?;
        self.schedules_list()
    }

    /// Delete a schedule.
    pub fn schedule_delete(&self, req: IdRequest) -> Result<Vec<Schedule>> {
        lock(&self.schedules).retain(|s| s.id != req.id);
        self.save_schedules()?;
        self.schedules_list()
    }

    /// "Save as weekly schedule" from the current plan and scope.
    pub fn schedule_from_plan(&self, req: ScheduleFromPlanRequest) -> Result<Schedule> {
        let plan = lock(&self.plan).plan.clone();
        if plan.steps.is_empty() {
            return Err(Error::InvalidInput("the plan is empty".into()));
        }
        if plan.steps.iter().any(|s| s.safety.requires_confirmation()) {
            return Err(Error::InvalidInput(
                "lossy or permanent steps cannot run unattended; remove them first".into(),
            ));
        }
        let schedule = Schedule {
            id: String::new(),
            name: req.name,
            enabled: true,
            cadence: req.cadence,
            scope: lock(&self.scope).clone(),
            steps: plan
                .steps
                .iter()
                .map(|s| ScheduledStep {
                    opportunity: s.opportunity.clone(),
                    action: s.action.clone(),
                    params: s.params.clone(),
                })
                .collect(),
            last_run: None,
            next_run: None,
        };
        let list = self.schedule_save(ScheduleSaveRequest { schedule })?;
        list.last().cloned().ok_or_else(|| Error::Internal("schedule not stored".into()))
    }

    /// Run a schedule now.
    pub fn schedule_run_now(self: &Arc<Self>, req: IdRequest) -> Result<ApplyStarted> {
        if !lock(&self.schedules).iter().any(|s| s.id == req.id) {
            return Err(Error::InvalidInput(format!("unknown schedule {}", req.id)));
        }
        Err(Error::not_implemented("running schedules (W3-E)"))
    }
}
