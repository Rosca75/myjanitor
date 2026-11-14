//! Shared helpers for the engine integration tests. Every test builds its own fixture
//! tree and its own app folders inside `TempDir`s; the engine is never pointed at
//! anything else, the guard's allowlist is the fixture root, and the Trash route is the
//! test's own quarantine (nothing reaches the user's real Trash).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub mod fixtures;

use mj_core::testutil::{set_mtime, TempDir};
use mj_engine::api::*;
use mj_engine::{Engine, EngineConfig};

/// Events with their arrival time.
#[derive(Default)]
pub struct TimedSink(pub Mutex<Vec<(Instant, Event)>>);

impl EventSink for TimedSink {
    fn emit(&self, event: &Event) {
        self.0.lock().unwrap().push((Instant::now(), event.clone()));
    }
}

impl TimedSink {
    pub fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap().iter().map(|(_, e)| e.clone()).collect()
    }
}

pub struct Fixture {
    pub tree: TempDir,
    pub app: TempDir,
    pub events: Arc<TimedSink>,
    pub engine: Arc<Engine>,
}

pub fn config(app: &TempDir, events: Arc<TimedSink>) -> EngineConfig {
    let mut cfg =
        EngineConfig::with_dirs(app.join("data"), app.join("cache"), app.join("journal"), app.join("quarantine"));
    cfg.temp_dir = app.join("tmp");
    cfg.trash_route = mj_actions::TrashRoute::Quarantine;
    cfg.events = events;
    cfg
}

/// Engine over `tree` with fresh app folders.
pub fn engine_for(tree: TempDir) -> Fixture {
    let app = TempDir::new("engine-app").unwrap();
    let events = Arc::new(TimedSink::default());
    let engine = Engine::new(config(&app, events.clone())).unwrap();
    engine.scope_add_root(ScopePathRequest { path: tree.path().to_path_buf() }).unwrap();
    Fixture { tree, app, events, engine }
}

fn days_ago(d: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(d * 86_400)
}

/// Deterministic pseudo-random bytes.
pub fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// The standard tree: duplicates, a temp file, developer artefacts, an empty folder, an
/// old installer, a signed PDF (duplicated) and a symlink.
pub fn standard_tree() -> TempDir {
    let t = TempDir::new("engine-tree").unwrap();
    let dup = bytes(1, 300_000);
    t.write("photos/a.bin", &dup).unwrap();
    let copy = t.write("downloads/a.bin", &dup).unwrap();
    set_mtime(&copy, days_ago(3)).unwrap();
    set_mtime(&t.join("photos/a.bin"), days_ago(10)).unwrap();
    let tmp = t.write("work/report.tmp", &bytes(2, 5000)).unwrap();
    set_mtime(&tmp, days_ago(3)).unwrap();
    t.write("proj/package.json", b"{\"name\":\"x\"}").unwrap();
    t.write("proj/node_modules/lib/index.js", &bytes(3, 20_000)).unwrap();
    t.write("proj/node_modules/lib/package.json", b"{}").unwrap();
    t.mkdir("old/empty-folder").unwrap();
    let msi = t.write("downloads/setup-1.0.msi", &bytes(4, 40_000)).unwrap();
    set_mtime(&msi, days_ago(400)).unwrap();
    let mut pdf = b"%PDF-1.7\n1 0 obj << /Type /Sig /ByteRange [0 10 20 30] /Contents <00> >> endobj\n".to_vec();
    pdf.extend(bytes(5, 2000));
    pdf.extend_from_slice(b"\n%%EOF\n");
    t.write("contracts/signed.pdf", &pdf).unwrap();
    let pdf2 = t.write("downloads/signed.pdf", &pdf).unwrap();
    set_mtime(&pdf2, days_ago(2)).unwrap();
    set_mtime(&t.join("contracts/signed.pdf"), days_ago(20)).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(t.join("photos/a.bin"), t.join("photos/link-to-a")).unwrap();
    t
}

/// Poll `f` until it returns `Some` (20 s timeout).
pub fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Scan and wait for the end; returns the final status.
pub fn scan(e: &Arc<Engine>) -> ScanStatus {
    let started = e.scan_start(ScanStartRequest::default()).unwrap();
    wait("scan", || {
        let s = e.scan_status().unwrap();
        (s.generation == started.generation
            && matches!(s.state, ScanRunState::Done | ScanRunState::Failed | ScanRunState::Cancelled)
            && !s.progress.files.eq(&u64::MAX))
        .then_some(s)
    })
}

/// Wait until no job is queued or running.
pub fn wait_jobs(e: &Arc<Engine>) -> Vec<mj_core::Job> {
    wait("jobs", || {
        let jobs = e.jobs_list().unwrap();
        jobs.iter().all(|j| j.state.is_finished()).then_some(jobs)
    })
}

pub fn opp(e: &Engine, id: &str) -> Option<mj_core::Opportunity> {
    e.opportunities_list(OpportunitiesRequest::default()).unwrap().items.into_iter().find(|o| o.id.0 == id)
}

pub fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

pub fn path(t: &TempDir, rel: &str) -> PathBuf {
    t.join(rel)
}
