//! Scheduled runs and job end states (W6-A review findings ENG-8, ENG-10, IPC-2, IPC-7).
//! The empty-folder detector is swapped for a double that holds the scan in its detector
//! phase while `HOLD` is set, so a test can act while a scheduled run is scanning.

use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use mj_core::testutil::TempDir;
use mj_core::{
    Action, Cadence, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, JobKind, JobOutcome, JobState,
    OpportunityId, Result, ScanScope, Schedule, ScheduledStep,
};

use crate::api::*;
use crate::jobs::{JobResult, Work};
use crate::{Engine, EngineConfig};

static HOLD: AtomicBool = AtomicBool::new(false);

/// Empty-folder detector double: waits (on the scan's token) while `HOLD` is set.
struct HeldDetector;

impl Detector for HeldDetector {
    fn info(&self) -> DetectorInfo {
        mj_detect::detectors().into_iter().find(|d| d.info().id == DetectorId::EmptyFolders).unwrap().info()
    }
    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        while HOLD.load(SeqCst) {
            ctx.cancel.checkpoint()?;
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(DetectorOutput::default())
    }
}

/// `HOLD` is shared: the tests that use it run one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fx {
    tree: TempDir,
    _app: TempDir,
    engine: Arc<Engine>,
}

fn old(t: &TempDir, rel: &str, days: u64, len: usize) {
    let p = t.write(rel, &vec![7u8; len]).unwrap();
    mj_core::testutil::set_mtime(&p, SystemTime::now() - Duration::from_secs(86_400 * days)).unwrap();
}

/// An old temp file and an old installer (two cards), the held detector in place.
fn fixture() -> Fx {
    let tree = TempDir::new("engine-schedjob").unwrap();
    old(&tree, "work/a.tmp", 3, 3000);
    old(&tree, "dl/setup-1.0.msi", 400, 40_000);
    let app = TempDir::new("engine-schedjob-app").unwrap();
    let mut cfg = EngineConfig::with_dirs(app.join("data"), app.join("cache"), app.join("journal"), app.join("q"));
    cfg.trash_route = Some(mj_actions::TrashRoute::Quarantine);
    let mut engine = Engine::new(cfg).unwrap();
    {
        let e = Arc::get_mut(&mut engine).unwrap();
        e.detectors.retain(|d| d.info().id != DetectorId::EmptyFolders);
        e.detectors.push(Box::new(HeldDetector));
    }
    engine.scope_add_root(ScopePathRequest { path: tree.path().to_path_buf() }).unwrap();
    Fx { tree, _app: app, engine }
}

fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn scan(e: &Arc<Engine>) {
    e.scan_start(ScanStartRequest::default()).unwrap();
    wait("scan", || (e.scan_status().unwrap().state == ScanRunState::Done && !e.scan_running()).then_some(()));
}

fn wait_jobs(e: &Arc<Engine>) -> Vec<mj_core::Job> {
    wait("jobs", || {
        let j = e.jobs_list().unwrap();
        j.iter().all(|j| j.state.is_finished()).then_some(j)
    })
}

fn job(e: &Engine, id: mj_core::JobId) -> mj_core::Job {
    e.jobs_list().unwrap().into_iter().find(|j| j.id == id).unwrap()
}

/// A schedule over `root` that trashes temp files.
fn save_schedule(e: &Engine, root: &std::path::Path) -> String {
    let schedule = Schedule {
        id: String::new(),
        name: "Tidy".into(),
        enabled: true,
        cadence: Cadence::Weekly { weekday: 1, hour: 9, minute: 13 },
        scope: ScanScope { roots: vec![root.to_path_buf()], ..Default::default() },
        steps: vec![ScheduledStep {
            opportunity: OpportunityId("det:temp_files".into()),
            action: Action::Trash,
            params: Default::default(),
        }],
        last_run: None,
        next_run: None,
    };
    e.schedule_save(ScheduleSaveRequest { schedule }).unwrap()[0].id.clone()
}

/// Start the schedule with the scan held in its detector phase; returns the schedule job.
fn start_held(fx: &Fx) -> mj_core::Job {
    let e = &fx.engine;
    let id = save_schedule(e, fx.tree.path());
    HOLD.store(true, SeqCst);
    let job = e.schedule_run_now(IdRequest { id }).unwrap();
    wait("detector phase", || {
        let s = e.scan_status().unwrap();
        (s.progress.phase == Some(mj_core::Phase::Detecting) && job_state(e, job.id) == JobState::Running).then_some(())
    });
    job
}

fn job_state(e: &Engine, id: mj_core::JobId) -> JobState {
    job(e, id).state
}

fn applies(e: &Engine) -> usize {
    e.jobs_list().unwrap().iter().filter(|j| j.kind == JobKind::Apply).count()
}

/// ENG-10: pausing a scheduled run pauses its scan and holds the apply until it is resumed.
#[test]
fn pausing_a_scheduled_run_holds_its_scan_and_its_apply() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fx = fixture();
    let e = &fx.engine;
    let job = start_held(&fx);
    e.job_pause(JobRequest { job_id: job.id }).unwrap();
    wait("scan paused", || (e.scan_status().unwrap().state == ScanRunState::Paused).then_some(()));
    HOLD.store(false, SeqCst);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(e.scan_status().unwrap().state, ScanRunState::Paused, "the scan waits for the job");
    assert_eq!(applies(e), 0, "nothing applied while paused");
    assert!(fx.tree.join("work/a.tmp").exists());
    e.job_resume(JobRequest { job_id: job.id }).unwrap();
    wait("apply queued", || (applies(e) == 1).then_some(()));
    let jobs = wait_jobs(e);
    assert_eq!(jobs.iter().find(|j| j.id == job.id).unwrap().state, JobState::Completed);
    assert!(!fx.tree.join("work/a.tmp").exists());
}

/// ENG-10: a scheduled run cancelled while paused stops its scan and applies nothing.
#[test]
fn cancelling_a_paused_scheduled_run_applies_nothing() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fx = fixture();
    let e = &fx.engine;
    let job = start_held(&fx);
    e.job_pause(JobRequest { job_id: job.id }).unwrap();
    wait("scan paused", || (e.scan_status().unwrap().state == ScanRunState::Paused).then_some(()));
    e.job_cancel(JobRequest { job_id: job.id }).unwrap();
    HOLD.store(false, SeqCst);
    let jobs = wait_jobs(e);
    assert_eq!(jobs.iter().find(|j| j.id == job.id).unwrap().state, JobState::Cancelled);
    wait("scan stopped", || (!e.scan_running()).then_some(()));
    assert_eq!(applies(e), 0);
    assert!(fx.tree.join("work/a.tmp").exists());
    assert!(e.plan_get().unwrap().steps.is_empty());
}

/// ENG-8: a step the user adds while a scheduled run is scanning is neither applied by
/// the run nor removed from the plan; the run applies its own step only.
#[test]
fn a_scheduled_run_applies_only_its_own_steps() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fx = fixture();
    let e = &fx.engine;
    scan(e);
    let job = start_held(&fx);
    // The previous results are still shown: the user adds a card to review later.
    e.plan_add_step(PlanAddStepRequest {
        opportunity_id: OpportunityId("det:old_installers".into()),
        action: None,
        params: None,
    })
    .unwrap();
    HOLD.store(false, SeqCst);
    wait("apply queued", || (applies(e) == 1).then_some(()));
    let jobs = wait_jobs(e);
    assert_eq!(jobs.iter().find(|j| j.id == job.id).unwrap().state, JobState::Completed);
    assert_eq!(applies(e), 1, "one apply job: the schedule's step");
    assert!(!fx.tree.join("work/a.tmp").exists(), "the schedule's step ran");
    assert!(fx.tree.join("dl/setup-1.0.msi").exists(), "the user's step did not run");
    let plan = e.plan_get().unwrap();
    assert_eq!(plan.steps.len(), 1, "{plan:?}");
    assert_eq!(plan.steps[0].opportunity.0, "det:old_installers");
    // IPC-7: the session scope is the schedule's, the folders its results describe.
    assert_eq!(e.scope_get().unwrap().roots, vec![fx.tree.path().to_path_buf()]);
}

/// ENG-10: a job cancelled after it did all its work ends as completed; one that stopped
/// early ends as cancelled.
#[test]
fn a_late_cancel_does_not_relabel_finished_work() {
    let fx = fixture();
    let e = &fx.engine;
    let run = |complete: bool| {
        let job = e.new_job(JobKind::PurgeQuarantine, None, None, "test", 1, 0);
        let id = job.id;
        let work: Work = Box::new(move |engine, ctx| {
            engine.job_cancel(JobRequest { job_id: ctx.job_id }).unwrap();
            let outcome = JobOutcome { succeeded: u64::from(complete), ..Default::default() };
            JobResult { outcome: Some(outcome), error: None, report: false, complete }
        });
        e.enqueue(vec![(job, work)]).unwrap();
        wait_jobs(e);
        job_state(e, id)
    };
    assert_eq!(run(true), JobState::Completed);
    assert_eq!(run(false), JobState::Cancelled);
}

/// IPC-2: the GUI builds the engine without the start-up purge and queues it as a job only
/// when something has expired.
#[test]
fn the_start_up_purge_is_a_job_and_only_when_something_expired() {
    let tree = TempDir::new("engine-startpurge").unwrap();
    old(&tree, "work/a.tmp", 3, 3000);
    old(&tree, "work/b.tmp", 3, 2000);
    let app = TempDir::new("engine-startpurge-app").unwrap();
    let cfg = || {
        let mut c = EngineConfig::with_dirs(app.join("data"), app.join("cache"), app.join("journal"), app.join("q"));
        c.trash_route = Some(mj_actions::TrashRoute::Quarantine);
        c.purge_at_start = false;
        c
    };
    {
        let e = Engine::new(cfg()).unwrap();
        e.scope_add_root(ScopePathRequest { path: tree.path().to_path_buf() }).unwrap();
        scan(&e);
        e.plan_add_step(PlanAddStepRequest {
            opportunity_id: OpportunityId("det:temp_files".into()),
            action: None,
            params: None,
        })
        .unwrap();
        e.plan_apply(PlanApplyRequest::default()).unwrap();
        wait_jobs(&e);
        assert_eq!(e.quarantine_items().len(), 2);
        assert!(e.quarantine_purge_if_due().unwrap().is_none(), "nothing expired: no job");
        assert!(e.jobs_list().unwrap().iter().all(|j| j.kind == JobKind::Apply));
        e.shutdown(Duration::from_secs(5));
    }
    // Forty days later (the journal says so): the items have expired.
    let journal = mj_actions::Journal::open(&app.join("journal")).unwrap();
    for mut entry in journal.all_entries().into_iter().filter(|x| x.retained_at.is_some()) {
        entry.at = entry.at.add_days(-40);
        journal.append(&entry).unwrap();
    }
    drop(journal);
    let e = Engine::new(cfg()).unwrap();
    assert_eq!(e.quarantine_items().len(), 2, "building the engine purges nothing");
    let job = e.quarantine_purge_if_due().unwrap().expect("a purge job");
    assert_eq!(job.kind, JobKind::PurgeQuarantine);
    wait_jobs(&e);
    assert_eq!(job_state(&e, job.id), JobState::Completed);
    assert!(e.quarantine_items().is_empty());
    assert!(e.quarantine_purge_if_due().unwrap().is_none());
}
