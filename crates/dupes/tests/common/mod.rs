//! Shared helpers for the mj-dupes integration tests: build a `FileIndex` from a
//! `TempDir` tree (std only) and run detectors against it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use mj_core::{
    CancelToken, DetectContext, Detector, DetectorOutput, EntryKind, FileEntry, FileIndex, MemoryFingerprintStore,
    ParamValue, ParamValues, Progress, ProgressSink, Result, ScanStats, Timestamp,
};

/// Recursively index `root` (no links followed).
pub fn index_of(root: &Path) -> FileIndex {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<FileEntry>) {
        let mut names: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        names.sort();
        for p in names {
            let md = std::fs::symlink_metadata(&p).unwrap();
            let kind = if md.is_dir() {
                EntryKind::Dir
            } else if md.is_file() {
                EntryKind::File
            } else {
                EntryKind::Link
            };
            let mut e = FileEntry::new(&p, kind, if md.is_file() { md.len() } else { 0 });
            e.modified = md.modified().ok().map(Timestamp::from_system_time);
            e.depth = depth;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                e.file_id = Some(mj_core::FileId { volume: md.dev(), index: md.ino() });
                e.links = Some(md.nlink() as u32);
            }
            out.push(e);
            if kind == EntryKind::Dir {
                walk(&p, depth + 1, out);
            }
        }
    }
    let mut entries = vec![FileEntry::new(root, EntryKind::Dir, 0)];
    walk(root, 1, &mut entries);
    FileIndex::new(vec![root.to_path_buf()], entries, ScanStats::default(), Timestamp::now())
}

/// Counts progress reports.
#[derive(Default)]
pub struct Counter(pub AtomicU64);

impl ProgressSink for Counter {
    fn report(&self, _: &Progress) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// Everything a detector run needs.
pub struct Harness {
    pub index: FileIndex,
    pub params: ParamValues,
    pub store: MemoryFingerprintStore,
    pub cancel: CancelToken,
    pub progress: Counter,
    pub threads: usize,
}

impl Harness {
    pub fn new(index: FileIndex) -> Self {
        Harness {
            index,
            params: ParamValues::new(),
            store: MemoryFingerprintStore::new(),
            cancel: CancelToken::new(),
            progress: Counter::default(),
            threads: 4,
        }
    }

    pub fn text(mut self, k: &str, v: &str) -> Self {
        self.params.set(k, ParamValue::Text(v.into()));
        self
    }

    pub fn num(mut self, k: &str, v: f64) -> Self {
        self.params.set(k, ParamValue::Number(v));
        self
    }

    pub fn flag(mut self, k: &str, v: bool) -> Self {
        self.params.set(k, ParamValue::Bool(v));
        self
    }

    pub fn run(&self, d: &dyn Detector) -> Result<DetectorOutput> {
        let ctx = DetectContext {
            index: &self.index,
            params: &self.params,
            now: Timestamp::now(),
            fingerprints: &self.store,
            cancel: &self.cancel,
            progress: &self.progress,
            threads: self.threads,
        };
        d.run(&ctx)
    }
}

/// Set a file's mtime to `days` days after 2024-01-01.
pub fn age(path: &Path, days: u64) {
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_704_067_200 + days * 86_400);
    mj_core::testutil::set_mtime(path, t).unwrap();
}

/// Deterministic pseudo-random bytes.
pub fn noise(seed: u64, n: usize) -> Vec<u8> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 32) as u8
        })
        .collect()
}

/// File names of the members of every group, sorted, for compact assertions.
pub fn group_names(out: &DetectorOutput) -> Vec<Vec<String>> {
    let mut v: Vec<Vec<String>> = out
        .groups
        .iter()
        .map(|g| {
            let mut m: Vec<String> = g.members.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
            m.sort();
            m
        })
        .collect();
    v.sort();
    v
}

/// The keeper's file name of the group containing `name`.
pub fn keeper_of(out: &DetectorOutput, name: &str) -> String {
    let g = out
        .groups
        .iter()
        .find(|g| g.members.iter().any(|p| p.file_name().unwrap() == name))
        .expect("group");
    g.keeper.as_ref().unwrap().file_name().unwrap().to_string_lossy().into_owned()
}

/// Every group has exactly one keeper candidate that is not selected.
pub fn assert_one_keeper_per_group(out: &DetectorOutput) {
    for g in &out.groups {
        let members: Vec<_> = out.candidates.iter().filter(|c| c.group == Some(g.id)).collect();
        assert_eq!(members.len(), g.members.len());
        let keepers: Vec<_> = members.iter().filter(|c| c.badges.contains(mj_core::Badges::KEEPER)).collect();
        assert_eq!(keepers.len(), 1, "group {}", g.label);
        assert!(!keepers[0].default_selected);
        assert_eq!(Some(&keepers[0].path), g.keeper.as_ref());
    }
}
