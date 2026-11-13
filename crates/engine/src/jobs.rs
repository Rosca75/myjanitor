//! Job queue (OPT-ENG-11). Owner: W3-D.

use std::collections::HashMap;

use mj_core::{lock, CancelToken, Error, Job, JobId, JobState, Result};

use crate::api::{Event, JobRequest, JobsReorderRequest};
use crate::Engine;

/// Queue state of the session.
#[derive(Debug, Default)]
pub struct JobsState {
    pub jobs: Vec<Job>,
    pub tokens: HashMap<JobId, CancelToken>,
}

impl Engine {
    fn job_update(&self, id: JobId, f: impl FnOnce(&mut Job, Option<&CancelToken>)) -> Result<Job> {
        let job = {
            let mut st = lock(&self.jobs);
            let token = st.tokens.get(&id).cloned();
            let job = st
                .jobs
                .iter_mut()
                .find(|j| j.id == id)
                .ok_or_else(|| Error::InvalidInput(format!("unknown job {}", id.0)))?;
            f(job, token.as_ref());
            job.clone()
        };
        self.emit(Event::JobState(job.clone()));
        Ok(job)
    }

    /// Queue contents.
    pub fn jobs_list(&self) -> Result<Vec<Job>> {
        Ok(lock(&self.jobs).jobs.clone())
    }

    /// Pause a job.
    pub fn job_pause(&self, req: JobRequest) -> Result<Job> {
        self.job_update(req.job_id, |j, t| {
            if j.state == JobState::Running {
                if let Some(t) = t {
                    t.pause();
                }
                j.state = JobState::Paused;
            }
        })
    }

    /// Resume a job.
    pub fn job_resume(&self, req: JobRequest) -> Result<Job> {
        self.job_update(req.job_id, |j, t| {
            if j.state == JobState::Paused {
                if let Some(t) = t {
                    t.resume();
                }
                j.state = JobState::Running;
            }
        })
    }

    /// Cancel a job.
    pub fn job_cancel(&self, req: JobRequest) -> Result<Job> {
        self.job_update(req.job_id, |j, t| {
            if !j.state.is_finished() {
                if let Some(t) = t {
                    t.cancel();
                }
                j.state = if j.state == JobState::Queued { JobState::Cancelled } else { JobState::Cancelling };
            }
        })
    }

    /// Reorder queued jobs.
    pub fn jobs_reorder(&self, req: JobsReorderRequest) -> Result<Vec<Job>> {
        let mut st = lock(&self.jobs);
        let (mut queued, rest): (Vec<Job>, Vec<Job>) = st.jobs.drain(..).partition(|j| j.state == JobState::Queued);
        let mut ordered = Vec::new();
        for id in &req.job_ids {
            if let Some(pos) = queued.iter().position(|j| j.id == *id) {
                ordered.push(queued.remove(pos));
            }
        }
        ordered.extend(queued);
        st.jobs = rest.into_iter().chain(ordered).collect();
        Ok(st.jobs.clone())
    }
}
