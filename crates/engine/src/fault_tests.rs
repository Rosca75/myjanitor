//! Fault injection through the whole engine (W4-INT item 8): a panicking detector, a
//! panicking optimiser, an optimiser cancelled in the middle of a file and a lossy
//! optimiser without ffmpeg. The engine is built normally, then (while it is still the
//! only owner) its analysers are swapped for test doubles, so nothing else changes.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mj_core::testutil::{snapshot, TempDir};
use mj_core::{
    ActionKind, Availability, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error,
    Estimate, ExternalTool, FileEntry, JobState, OpportunityId, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId,
    OptimiserInfo, ParamValues, PlatformSupport, Result, SafetyClass,
};
use mj_optimise::images::common::{analyse_entries, Described};

use crate::api::*;
use crate::{Engine, EngineConfig};

/// `.fake` files; behaviour by name: `panic*` panics, `slow*` writes half an output and
/// then waits for the cancel token, anything else is written at half size.
struct FakeOpt {
    lossy: bool,
}

impl Optimiser for FakeOpt {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::Video,
            title: "Fake video".into(),
            description: String::new(),
            category: Category::Optimise,
            safety: if self.lossy { SafetyClass::LossyOptIn } else { SafetyClass::Reversible },
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: vec![],
            settings: vec![],
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec![],
            unit: "files".into(),
        }
    }
    fn availability(&self, _: &[ExternalTool]) -> Availability {
        Availability::Available
    }
    fn applies_to(&self, e: &FileEntry) -> bool {
        e.is_file() && e.extension() == "fake"
    }
    fn estimate(&self, e: &FileEntry, _: &ParamValues) -> Result<Estimate> {
        Ok(Estimate { files: 1, bytes_before: e.size, bytes_after: e.size / 2, ..Default::default() })
    }
    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let name = job.src.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(job.src).map_err(|e| Error::io(job.src, e))?;
        if name.starts_with("panic") {
            std::fs::write(job.dst, &data[..10]).unwrap();
            panic!("hostile file");
        }
        std::fs::write(job.dst, &data[..data.len() / 2]).map_err(|e| Error::io(job.dst, e))?;
        if name.starts_with("slow") {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !job.cancel.is_cancelled() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            return Err(Error::Cancelled);
        }
        Ok(OptimiseOutput {
            bytes_before: data.len() as u64,
            bytes_after: (data.len() / 2) as u64,
            lossy: self.lossy,
            ..OptimiseOutput::default()
        })
    }
    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        analyse_entries(ctx, OptimiserId::Video, 1, &|e| self.applies_to(e), &|e, p| self.estimate(e, p), &|_, _| {
            Described { reason: "fake".into(), columns: Default::default(), sub_group: None }
        })
    }
}

struct PanicDetector;

impl Detector for PanicDetector {
    fn info(&self) -> DetectorInfo {
        let mut i = mj_detect::detectors().into_iter().find(|d| d.info().id == DetectorId::TempFiles).unwrap().info();
        i.title = "Panicking temp files".into();
        i
    }
    fn run(&self, _: &DetectContext<'_>) -> Result<DetectorOutput> {
        panic!("detector bug")
    }
}

struct Fx {
    tree: TempDir,
    _app: TempDir,
    engine: Arc<Engine>,
}

fn fixture(lossy: bool, swap_detector: bool, files: &[(&str, usize)]) -> Fx {
    let tree = TempDir::new("engine-faults").unwrap();
    for (name, n) in files {
        tree.write(name, &vec![b'x'; *n]).unwrap();
    }
    let app = TempDir::new("engine-faults-app").unwrap();
    let mut cfg = EngineConfig::with_dirs(app.join("data"), app.join("cache"), app.join("journal"), app.join("q"));
    cfg.temp_dir = app.join("tmp");
    cfg.trash_route = Some(mj_actions::TrashRoute::Quarantine);
    let mut engine = Engine::new(cfg).unwrap();
    {
        let e = Arc::get_mut(&mut engine).expect("sole owner right after construction");
        e.optimisers.retain(|o| o.info().id != OptimiserId::Video);
        e.optimisers.insert(0, Box::new(FakeOpt { lossy }));
        if swap_detector {
            e.detectors.retain(|d| d.info().id != DetectorId::TempFiles);
            e.detectors.push(Box::new(PanicDetector));
        }
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
    let s = wait("scan", || {
        let s = e.scan_status().unwrap();
        (!matches!(s.state, ScanRunState::Scanning | ScanRunState::Paused | ScanRunState::Stopping)
            && !e.scan_running())
        .then_some(s)
    });
    assert_eq!(s.state, ScanRunState::Done, "{s:?}");
}

fn add_fake_step(e: &Engine) -> mj_core::StepId {
    let plan = e
        .plan_add_step(PlanAddStepRequest {
            opportunity_id: OpportunityId("opt:video".into()),
            action: None,
            params: None,
        })
        .unwrap();
    plan.steps[0].id
}

fn wait_jobs(e: &Arc<Engine>) -> Vec<mj_core::Job> {
    wait("jobs", || {
        let j = e.jobs_list().unwrap();
        j.iter().all(|j| j.state.is_finished()).then_some(j)
    })
}

fn no_temp_left(dir: &Path) {
    let snap = snapshot(dir).unwrap();
    assert!(!snap.iter().any(|(p, _, _)| p.contains(".mj-")), "temporary output left: {snap:?}");
}

#[test]
fn a_panicking_detector_becomes_a_disabled_card_and_the_scan_survives() {
    let fx = fixture(false, true, &[("a.fake", 4000), ("b.tmp", 100)]);
    scan(&fx.engine);
    let list = fx.engine.opportunities_list(OpportunitiesRequest::default()).unwrap();
    let card = list.items.iter().find(|o| o.id.0 == "det:temp_files").expect("card listed");
    match &card.availability {
        Availability::Disabled { reason } => assert!(reason.contains("failed"), "{reason}"),
        a => panic!("{a:?}"),
    }
    assert!(list.items.iter().any(|o| o.id.0 == "opt:video" && o.availability.is_available()));
}

#[test]
fn a_panicking_optimiser_fails_its_item_only_and_leaves_the_original_and_no_temp() {
    let fx = fixture(false, false, &[("panic.fake", 4000), ("ok.fake", 4000)]);
    let before = std::fs::read(fx.tree.join("panic.fake")).unwrap();
    scan(&fx.engine);
    add_fake_step(&fx.engine);
    let started = fx.engine.plan_apply(PlanApplyRequest::default()).unwrap();
    let jobs = wait_jobs(&fx.engine);
    let j = jobs.iter().find(|j| j.run_id == Some(started.run_id)).unwrap();
    assert_eq!(j.state, JobState::CompletedWithErrors, "{j:?}");
    let out = j.outcome.as_ref().unwrap();
    assert_eq!((out.succeeded, out.failed), (1, 1), "{out:?}");
    assert!(out.errors[0].error.as_ref().unwrap().message.contains("hostile"), "{out:?}");
    assert_eq!(std::fs::read(fx.tree.join("panic.fake")).unwrap(), before, "original untouched");
    assert_eq!(std::fs::metadata(fx.tree.join("ok.fake")).unwrap().len(), 2000);
    no_temp_left(fx.tree.path());
    // The journal is settled: no intent left behind.
    let d = fx.engine.history_run_detail(RunDetailRequest { run_id: started.run_id, offset: 0, limit: 10 }).unwrap();
    assert!(d.entries.iter().all(|e| e.state != mj_core::JournalState::Intent), "{:?}", d.entries);
}

#[test]
fn cancelling_mid_file_removes_the_temp_output_and_keeps_the_original() {
    let fx = fixture(false, false, &[("slow.fake", 8000), ("z-later.fake", 8000)]);
    let before = snapshot(fx.tree.path()).unwrap();
    scan(&fx.engine);
    add_fake_step(&fx.engine);
    let started = fx.engine.plan_apply(PlanApplyRequest::default()).unwrap();
    let job = started.jobs[0].id;
    // Wait until the optimiser has written half its output beside the original.
    wait("temp output", || snapshot(fx.tree.path()).unwrap().iter().any(|(p, _, _)| p.contains(".mj-")).then_some(()));
    fx.engine.job_cancel(JobRequest { job_id: job }).unwrap();
    let jobs = wait_jobs(&fx.engine);
    let j = jobs.iter().find(|j| j.id == job).unwrap();
    assert_eq!(j.state, JobState::Cancelled, "{j:?}");
    assert_eq!(snapshot(fx.tree.path()).unwrap(), before, "originals untouched, temp removed, later file not started");
    let d = fx.engine.history_run_detail(RunDetailRequest { run_id: started.run_id, offset: 0, limit: 10 }).unwrap();
    assert!(d.entries.iter().all(|e| e.state != mj_core::JournalState::Intent), "{:?}", d.entries);
    assert!(d.entries.iter().all(|e| e.state != mj_core::JournalState::Done), "{:?}", d.entries);
    assert_eq!(fx.engine.quarantine_info().unwrap().items, 0);
}

#[test]
fn a_lossy_optimiser_is_refused_without_confirmation_and_runs_with_it() {
    let fx = fixture(true, false, &[("clip.fake", 4000)]);
    scan(&fx.engine);
    let step = add_fake_step(&fx.engine);
    assert_eq!(fx.engine.plan_get().unwrap().steps[0].safety, SafetyClass::LossyOptIn);
    let err = fx.engine.plan_apply(PlanApplyRequest::default()).unwrap_err();
    assert_eq!(err.code(), "confirmation_required");
    assert_eq!(std::fs::metadata(fx.tree.join("clip.fake")).unwrap().len(), 4000);
    let conf = mj_core::StepConfirmation {
        step_id: step,
        acknowledged: true,
        originals: Default::default(),
        synced_online_only: false,
        selection_digest: Some(
            fx.engine.plan_confirmation_info(StepRequest { step_id: step }).unwrap().selection_digest,
        ),
    };
    fx.engine.plan_apply(PlanApplyRequest { confirmations: vec![conf] }).unwrap();
    let jobs = wait_jobs(&fx.engine);
    assert_eq!(jobs[0].outcome.as_ref().unwrap().succeeded, 1, "{jobs:?}");
    assert_eq!(std::fs::metadata(fx.tree.join("clip.fake")).unwrap().len(), 2000);
}

#[test]
fn double_starts_are_refused_cleanly() {
    let fx = fixture(false, false, &[("slow.fake", 8000)]);
    scan(&fx.engine);
    add_fake_step(&fx.engine);
    let started = fx.engine.plan_apply(PlanApplyRequest::default()).unwrap();
    assert_eq!(fx.engine.plan_apply(PlanApplyRequest::default()).map(|_| ()).unwrap_err().code(), "conflict");
    fx.engine.job_cancel(JobRequest { job_id: started.jobs[0].id }).unwrap();
    wait_jobs(&fx.engine);
    // A scan while one runs is a conflict too (exercised on a larger tree in surfaces.rs).
}

static HOLD_DETECTOR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Empty-folder detector double that, while `HOLD_DETECTOR` is set, waits on the cancel
/// token (a long detector phase).
struct SlowDetector;

impl Detector for SlowDetector {
    fn info(&self) -> DetectorInfo {
        mj_detect::detectors().into_iter().find(|d| d.info().id == DetectorId::EmptyFolders).unwrap().info()
    }
    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        while HOLD_DETECTOR.load(std::sync::atomic::Ordering::SeqCst) {
            ctx.cancel.checkpoint()?;
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(DetectorOutput::default())
    }
}

#[test]
fn stopping_a_scan_in_the_detector_phase_keeps_the_previous_results() {
    use std::sync::atomic::Ordering::SeqCst;
    let tree = TempDir::new("engine-slowdet").unwrap();
    tree.write("a.tmp", &[1u8; 300]).unwrap();
    mj_core::testutil::set_mtime(&tree.join("a.tmp"), std::time::SystemTime::now() - Duration::from_secs(86_400 * 3))
        .unwrap();
    let app = TempDir::new("engine-slowdet-app").unwrap();
    let mut cfg = EngineConfig::with_dirs(app.join("data"), app.join("cache"), app.join("journal"), app.join("q"));
    cfg.trash_route = Some(mj_actions::TrashRoute::Quarantine);
    let mut engine = Engine::new(cfg).unwrap();
    {
        let e = Arc::get_mut(&mut engine).unwrap();
        e.detectors.retain(|d| d.info().id != DetectorId::EmptyFolders);
        e.detectors.push(Box::new(SlowDetector));
    }
    let e = engine;
    e.scope_add_root(ScopePathRequest { path: tree.path().to_path_buf() }).unwrap();
    scan(&e);
    let first = e.opportunities_list(OpportunitiesRequest::default()).unwrap();
    assert!(first.items.iter().any(|o| o.id.0 == "det:temp_files"));
    HOLD_DETECTOR.store(true, SeqCst);
    let g = e.scan_start(ScanStartRequest::default()).unwrap().generation;
    wait("detector phase", || {
        let s = e.scan_status().unwrap();
        (s.progress.phase == Some(mj_core::Phase::Detecting)).then_some(())
    });
    // Pause and resume in the detector phase, then stop.
    assert_eq!(e.scan_pause().unwrap().state, ScanRunState::Paused);
    assert_eq!(e.scan_resume().unwrap().state, ScanRunState::Scanning);
    e.scan_stop().unwrap();
    let s = wait("stopped", || {
        let s = e.scan_status().unwrap();
        (!e.scan_running() && s.state == ScanRunState::Cancelled).then_some(s)
    });
    HOLD_DETECTOR.store(false, SeqCst);
    assert_eq!(s.generation, g);
    let after = e.opportunities_list(OpportunitiesRequest::default()).unwrap();
    assert_eq!(after.generation, first.generation, "a stopped scan publishes nothing");
    assert_eq!(after.items.len(), first.items.len());
    // The next scan works.
    scan(&e);
}
