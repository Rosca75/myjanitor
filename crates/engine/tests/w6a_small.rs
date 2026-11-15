//! Regression tests for the smaller IPC, CLI and diagnostics findings fixed in W6-A
//! (IPC-4 progress file counts, XC-3 diagnostics).

mod common;

use common::*;
use mj_core::testutil::TempDir;
use mj_engine::api::*;

/// IPC-4: every scan://progress event after the walk carries the walked file count, not 0.
#[test]
fn progress_events_after_the_walk_keep_the_file_count() {
    let t = TempDir::new("w6a-progress").unwrap();
    for d in 0..30 {
        for f in 0..20 {
            t.write(format!("dir{d}/f{f}.txt"), format!("{d}-{f}").as_bytes()).unwrap();
        }
    }
    t.write("dup/a.bin", &bytes(1, 5000)).unwrap();
    t.write("dup/b.bin", &bytes(1, 5000)).unwrap();
    let f = engine_for(t);
    assert_eq!(scan(&f.engine).state, ScanRunState::Done);
    let progress: Vec<ScanProgressEvent> = f
        .events
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::ScanProgress(p) => Some(p),
            _ => None,
        })
        .collect();
    let walked = progress
        .iter()
        .filter(|p| p.phase == Some(mj_core::Phase::Walking))
        .map(|p| p.files)
        .max()
        .expect("a walking event");
    assert!(walked >= 600, "walked {walked}");
    let later: Vec<_> = progress.iter().filter(|p| p.phase != Some(mj_core::Phase::Walking)).collect();
    assert!(!later.is_empty());
    for p in later {
        assert_eq!(p.files, walked, "phase {:?} lost the file count", p.phase);
    }
}
