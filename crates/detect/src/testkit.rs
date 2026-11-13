//! Test support shared by the unit tests and `tests/fixture_tree.rs` (included there with `#[path]`):
//! a tiny directory walker that builds a [`FileIndex`] from a fixture tree, a detector runner and
//! timestamp helpers. Only ever pointed at `mj_core::testutil::TempDir` trees.
#![allow(dead_code)]

use std::fs::{self, FileTimes};
use std::path::{Path, PathBuf};

use mj_core::{
    CancelToken, DetectContext, Detector, DetectorOutput, EntryKind, FileIndex, NoFingerprintStore, NullProgress,
    ParamValues, ScanStats, Timestamp,
};

/// Walk `root` (never following links) into an index; the root has depth 0.
pub fn index_of(root: &Path) -> FileIndex {
    let mut entries = Vec::new();
    let mut top = mj_platform::scan::metadata::metadata(root).expect("root metadata");
    top.depth = 0;
    entries.push(top);
    walk(root, 1, &mut entries);
    FileIndex::new(vec![root.to_path_buf()], entries, ScanStats::default(), Timestamp::now())
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<mj_core::FileEntry>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for item in rd.flatten() {
        let Ok(mut e) = mj_platform::scan::metadata::metadata(&item.path()) else { continue };
        e.depth = depth;
        let is_dir = e.kind == EntryKind::Dir;
        let p = e.path.clone();
        out.push(e);
        if is_dir {
            walk(&p, depth + 1, out);
        }
    }
}

/// Run a detector with default parameters merged with `params`.
pub fn run(d: &dyn Detector, ix: &FileIndex, params: &ParamValues) -> DetectorOutput {
    run_at(d, ix, params, Timestamp::now(), &CancelToken::new()).expect("detector run")
}

/// Run with an explicit `now` and cancel token.
pub fn run_at(
    d: &dyn Detector,
    ix: &FileIndex,
    params: &ParamValues,
    now: Timestamp,
    cancel: &CancelToken,
) -> mj_core::Result<DetectorOutput> {
    let merged = mj_core::params::defaults(&d.info().settings).merged(params);
    let ctx = DetectContext {
        index: ix,
        params: &merged,
        now,
        fingerprints: &NoFingerprintStore,
        cancel,
        progress: &NullProgress,
        threads: 2,
    };
    d.run(&ctx)
}

/// Candidate paths relative to `root`, `/`-separated and sorted.
pub fn rel(out: &DetectorOutput, root: &Path) -> Vec<String> {
    let mut v: Vec<String> = out
        .candidates
        .iter()
        .map(|c| c.path.strip_prefix(root).unwrap_or(&c.path).to_string_lossy().replace('\\', "/"))
        .collect();
    v.sort();
    v
}

/// Set both access and modification time `days` before now.
pub fn age(path: &Path, days: i64) {
    let t = Timestamp::now().add_days(-days).to_system_time();
    let f = fs::OpenOptions::new().write(true).open(path).expect("open for times");
    f.set_times(FileTimes::new().set_accessed(t).set_modified(t)).expect("set times");
}

/// Set times on a directory (opened read-only; works on Linux, best effort elsewhere).
pub fn age_dir(path: &Path, days: i64) {
    let t = Timestamp::now().add_days(-days).to_system_time();
    if let Ok(f) = fs::File::open(path) {
        let _ = f.set_times(FileTimes::new().set_accessed(t).set_modified(t));
    }
}

/// `params` builder.
pub fn p(pairs: &[(&str, f64)]) -> ParamValues {
    let mut v = ParamValues::new();
    for (k, n) in pairs {
        v.set(k, mj_core::ParamValue::Number(*n));
    }
    v
}

/// Make a `PathBuf` from a temp dir join (readability helper).
pub fn j(root: &Path, rel: &str) -> PathBuf {
    root.join(rel)
}
