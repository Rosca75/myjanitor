//! Job queue (OPT-ENG-11). Owner: W3-D.
//!
//! Jobs that change files (apply steps, undo, quarantine purge) run one at a time,
//! in queue order, on one large-stack worker thread (`mj-jobs`), so `jobs_reorder`
//! decides what runs next and two jobs never act on the same files concurrently.
//! Dry runs only read and run on their own thread at once. Pause and cancel act
//! between items (`CancelToken::checkpoint`); items already applied stay applied and
//! undoable. Every job body runs inside `catch_unwind`.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use mj_core::{
    lock, CancelToken, Error, Job, JobId, JobKind, JobOutcome, JobProgress, JobState, Progress, ProgressSink, Result,
    RunId, StepId, ThrottledSink, Timestamp,
};

use crate::api::{Event, JobDoneEvent, JobProgressEvent, JobRequest, JobsReorderRequest};
use crate::Engine;

/// What a job body returns.
#[derive(Debug, Default)]
pub(crate) struct JobResult {
    pub outcome: Option<JobOutcome>,
    pub error: Option<Error>,
    /// `report_get` works for the job's run.
    pub report: bool,
    /// ENG-10: the body did all its work. A job whose token was cancelled after its last
    /// item is reported by what it did (completed), not as cancelled.
    pub complete: bool,
}

/// Body of a job.
pub(crate) type Work = Box<dyn FnOnce(&Arc<Engine>, &JobCtx<'_>) -> JobResult + Send>;

/// Handed to a job body.
pub(crate) struct JobCtx<'a> {
    pub job_id: JobId,
    pub cancel: &'a CancelToken,
    pub progress: &'a dyn ProgressSink,
}

/// Queue state of the session.
#[derive(Default)]
pub struct JobsState {
    pub jobs: Vec<Job>,
    pub tokens: HashMap<JobId, CancelToken>,
    pub(crate) work: HashMap<JobId, Work>,
    pub worker_alive: bool,
    /// Detached jobs (dry runs) still running.
    pub detached: usize,
}

impl std::fmt::Debug for JobsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobsState").field("jobs", &self.jobs).field("worker_alive", &self.worker_alive).finish()
    }
}

/// Keep at most this many finished jobs in `jobs_list`.
const MAX_FINISHED: usize = 200;

impl Engine {
    /// A fresh job record.
    pub(crate) fn new_job(
        &self,
        kind: JobKind,
        run_id: Option<RunId>,
        step_id: Option<StepId>,
        title: &str,
        total: u64,
        bytes_total: u64,
    ) -> Job {
        Job {
            id: JobId(self.next_id()),
            kind,
            run_id,
            step_id,
            title: title.to_string(),
            state: JobState::Queued,
            progress: JobProgress { total, bytes_total, ..JobProgress::default() },
            created: Timestamp::now(),
            started: None,
            finished: None,
            outcome: None,
            error: None,
        }
    }

    /// Queue jobs (in order) and make sure the worker runs.
    pub(crate) fn enqueue(self: &Arc<Self>, jobs: Vec<(Job, Work)>) -> Result<()> {
        let start = {
            let mut st = lock(&self.jobs);
            for (job, work) in jobs {
                st.tokens.insert(job.id, CancelToken::new());
                st.work.insert(job.id, work);
                st.jobs.push(job);
            }
            let start = !st.worker_alive;
            st.worker_alive = true;
            start
        };
        for j in self.jobs_list()?.into_iter().filter(|j| j.state == JobState::Queued) {
            self.emit(Event::JobState(j));
        }
        if start {
            let me = Arc::clone(self);
            if let Err(e) = mj_core::pool::spawn_large("mj-jobs", move || me.worker_loop()) {
                lock(&self.jobs).worker_alive = false;
                return Err(Error::Internal(format!("cannot start the job worker: {e}")));
            }
        }
        Ok(())
    }

    /// Run one job on its own thread at once (read-only jobs: dry runs).
    pub(crate) fn spawn_detached(self: &Arc<Self>, job: Job, work: Work) -> Result<Job> {
        let id = job.id;
        {
            let mut st = lock(&self.jobs);
            st.tokens.insert(id, CancelToken::new());
            st.work.insert(id, work);
            st.jobs.push(job.clone());
            st.detached += 1;
        }
        self.emit(Event::JobState(job.clone()));
        let me = Arc::clone(self);
        let spawned = mj_core::pool::spawn_large("mj-job", move || {
            me.run_job(id);
            lock(&me.jobs).detached -= 1;
        });
        if let Err(e) = spawned {
            let mut st = lock(&self.jobs);
            st.detached -= 1;
            st.work.remove(&id);
            return Err(Error::Internal(format!("cannot start the job thread: {e}")));
        }
        Ok(job)
    }

    fn worker_loop(self: Arc<Self>) {
        loop {
            let next = {
                let mut st = lock(&self.jobs);
                let next =
                    st.jobs.iter().find(|j| j.state == JobState::Queued && st.work.contains_key(&j.id)).map(|j| j.id);
                if next.is_none() {
                    st.worker_alive = false;
                }
                next
            };
            match next {
                Some(id) => self.run_job(id),
                None => return,
            }
        }
    }

    /// Run job `id` on the current thread and publish its end.
    fn run_job(self: &Arc<Self>, id: JobId) {
        let (work, token, job) = {
            let mut st = lock(&self.jobs);
            let Some(work) = st.work.remove(&id) else { return };
            let token = st.tokens.get(&id).cloned().unwrap_or_default();
            let Some(job) = st.jobs.iter_mut().find(|j| j.id == id) else { return };
            if job.state != JobState::Queued {
                return;
            }
            job.state = JobState::Running;
            job.started = Some(Timestamp::now());
            (work, token, job.clone())
        };
        self.emit(Event::JobState(job.clone()));
        let me = Arc::clone(self);
        let run_id = job.run_id;
        let progress = ThrottledSink::new(move |p: &Progress| {
            let ev = {
                let mut st = lock(&me.jobs);
                let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) else { return };
                j.progress.done = p.done;
                if let Some(t) = p.total {
                    j.progress.total = t;
                }
                j.progress.saved = p.bytes;
                j.progress.bytes_done = p.bytes;
                JobProgressEvent { job_id: id, run_id, progress: j.progress, current: p.current.clone() }
            };
            me.emit(Event::JobProgress(ev));
        });
        let ctx = JobCtx { job_id: id, cancel: &token, progress: &progress };
        let result = catch_unwind(AssertUnwindSafe(|| work(self, &ctx))).unwrap_or_else(|p| {
            let text = mj_core::diag::panic_text(&*p);
            self.log("ERROR", &format!("job {} stopped unexpectedly: {text}", id.0));
            JobResult {
                error: Some(Error::Internal(format!("the job stopped unexpectedly: {text}"))),
                ..Default::default()
            }
        });
        let job = {
            let mut st = lock(&self.jobs);
            st.tokens.remove(&id);
            let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) else { return };
            j.finished = Some(Timestamp::now());
            j.state = match (&result.error, &result.outcome) {
                (Some(Error::Cancelled), _) => JobState::Cancelled,
                (Some(_), _) => JobState::Failed,
                _ if token.is_cancelled() && !result.complete => JobState::Cancelled,
                (None, Some(o)) if o.failed > 0 || o.skipped > 0 => JobState::CompletedWithErrors,
                _ => JobState::Completed,
            };
            if let Some(o) = &result.outcome {
                j.progress.done = o.succeeded + o.skipped + o.failed;
                j.progress.saved = o.bytes_saved;
                j.progress.bytes_done = o.bytes_saved;
            }
            j.outcome = result.outcome.clone();
            j.error = result.error.as_ref().filter(|e| !matches!(e, Error::Cancelled)).map(Error::info);
            let job = j.clone();
            let finished: Vec<JobId> = st.jobs.iter().filter(|j| j.state.is_finished()).map(|j| j.id).collect();
            if finished.len() > MAX_FINISHED {
                let drop: Vec<JobId> = finished[..finished.len() - MAX_FINISHED].to_vec();
                st.jobs.retain(|j| !drop.contains(&j.id));
            }
            job
        };
        self.emit(Event::JobState(job.clone()));
        self.emit(Event::JobDone(JobDoneEvent { job, report_available: result.report }));
    }

    /// True while any job runs or waits.
    pub(crate) fn jobs_running(&self) -> bool {
        let st = lock(&self.jobs);
        st.worker_alive || st.detached > 0
    }

    /// Cancel every queued and running job (shutdown).
    pub(crate) fn cancel_all_jobs(&self) {
        let ids: Vec<JobId> = lock(&self.jobs).jobs.iter().filter(|j| !j.state.is_finished()).map(|j| j.id).collect();
        for id in ids {
            let _ = self.job_cancel(JobRequest { job_id: id });
        }
    }

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
            let job = job.clone();
            if job.state == JobState::Cancelled {
                // A queued job cancelled before it started never runs.
                st.work.remove(&id);
                st.tokens.remove(&id);
            }
            job
        };
        self.emit(Event::JobState(job.clone()));
        if job.state == JobState::Cancelled && job.started.is_none() {
            self.emit(Event::JobDone(JobDoneEvent { job: job.clone(), report_available: false }));
        }
        Ok(job)
    }

    /// Queue contents: running, queued and the jobs finished this session.
    pub fn jobs_list(&self) -> Result<Vec<Job>> {
        Ok(lock(&self.jobs).jobs.clone())
    }

    /// Pause a running job (takes effect between items). Queued jobs are not paused:
    /// cancel or reorder them instead.
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

    /// Cancel a job: queued jobs become `cancelled` at once, running ones go through
    /// `cancelling`.
    pub fn job_cancel(&self, req: JobRequest) -> Result<Job> {
        self.job_update(req.job_id, |j, t| {
            if !j.state.is_finished() {
                if let Some(t) = t {
                    t.resume();
                    t.cancel();
                }
                if j.state == JobState::Queued {
                    j.state = JobState::Cancelled;
                    j.finished = Some(Timestamp::now());
                } else {
                    j.state = JobState::Cancelling;
                }
            }
        })
    }

    /// Reorder queued jobs only (running and finished jobs keep their place).
    pub fn jobs_reorder(&self, req: JobsReorderRequest) -> Result<Vec<Job>> {
        let mut st = lock(&self.jobs);
        let queued: Vec<JobId> = st.jobs.iter().filter(|j| j.state == JobState::Queued).map(|j| j.id).collect();
        if let Some(bad) = req.job_ids.iter().find(|id| !queued.contains(id)) {
            return Err(Error::InvalidInput(format!("job {} is not queued", bad.0)));
        }
        let (mut q, rest): (Vec<Job>, Vec<Job>) = st.jobs.drain(..).partition(|j| j.state == JobState::Queued);
        let mut ordered = Vec::new();
        for id in &req.job_ids {
            if let Some(pos) = q.iter().position(|j| j.id == *id) {
                ordered.push(q.remove(pos));
            }
        }
        ordered.extend(q);
        st.jobs = rest.into_iter().chain(ordered).collect();
        Ok(st.jobs.clone())
    }
}
