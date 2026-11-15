//! Staged exact-duplicate detection on generated trees (CLN-DET-01, CLN-ACT-01).

mod common;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::{Attributes, EntryKind, Error, FileEntry, FileIndex, FingerprintStore, ScanStats, Timestamp};
use mj_dupes::exact::ExactDuplicates;

const BIG: usize = 300 * 1024;

fn tree() -> TempDir {
    let t = TempDir::new("exact").unwrap();
    let a = noise(1, BIG);
    t.write("a.bin", &a).unwrap();
    t.write("copies/a.bin", &a).unwrap();
    t.write("copies/deeper/a (1).bin", &a).unwrap();
    // Same size, different content.
    t.write("b.bin", &noise(2, BIG)).unwrap();
    // Same head, different tail.
    let mut c = a.clone();
    c[BIG - 10] ^= 0xFF;
    t.write("same_head.bin", &c).unwrap();
    // Same head, middle and tail windows; differs between them: partial collides, full differs.
    let mut d = a.clone();
    d[100 * 1024] ^= 0xFF;
    t.write("same_windows.bin", &d).unwrap();
    // Small identical files (hashed whole in the partial stage).
    for n in ["s1.txt", "s2.txt", "sub/s3.txt"] {
        t.write(n, b"hello duplicate world").unwrap();
    }
    t.write("s_other.txt", b"hello duplicate WORLD").unwrap();
    // Zero-byte files belong to the empty-files detector.
    t.write("empty1", b"").unwrap();
    t.write("empty2", b"").unwrap();
    t
}

#[test]
fn finds_exact_groups_through_every_stage() {
    let t = tree();
    let h = Harness::new(index_of(t.path()));
    let out = h.run(&ExactDuplicates).unwrap();
    assert_eq!(
        group_names(&out),
        vec![vec!["a (1).bin", "a.bin", "a.bin"], vec!["s1.txt", "s2.txt", "s3.txt"]],
        "{:#?}",
        out.groups
    );
    assert_one_keeper_per_group(&out);
    assert_eq!(out.groups[0].confidence, 100.0);
    assert_eq!(out.diagnostics.unreadable, 0);
    // The big group comes first (most wasted bytes) and two of its three files are selected.
    assert_eq!(out.groups[0].members.len(), 3);
    assert_eq!(out.candidates.iter().filter(|c| c.default_selected).count(), 4);
    assert!(h.progress.0.load(std::sync::atomic::Ordering::Relaxed) > 0);
    // Every stage was written back to the cache: full + SHA-256 for collision members.
    let a = h.index.get(&t.join("a.bin")).unwrap();
    let fp = h.store.get(&a.path, a.stamp()).unwrap();
    assert!(fp.partial.is_some() && fp.full.is_some() && fp.sha256.is_some());
    let b = h.index.get(&t.join("b.bin")).unwrap();
    let fpb = h.store.get(&b.path, b.stamp()).unwrap();
    assert!(fpb.partial.is_some() && fpb.full.is_none(), "unique partial: no full read");
}

#[test]
fn identical_copy_of_a_cached_unique_file_is_found() {
    // D-006: size buckets include cache hits.
    let t = TempDir::new("cached").unwrap();
    t.write("old/report.pdf", &noise(9, 70_000)).unwrap();
    t.write("other.bin", &noise(10, 5_000)).unwrap();
    let mut h = Harness::new(index_of(t.path()));
    assert!(h.run(&ExactDuplicates).unwrap().groups.is_empty());
    t.write("new/report.pdf", &noise(9, 70_000)).unwrap();
    h.index = index_of(t.path());
    let out = h.run(&ExactDuplicates).unwrap();
    assert_eq!(group_names(&out), vec![vec!["report.pdf", "report.pdf"]]);
    // Second run on a warm cache gives the same answer.
    assert_eq!(group_names(&h.run(&ExactDuplicates).unwrap()), group_names(&out));
}

#[cfg(unix)]
#[test]
fn hardlinks_are_one_file_not_duplicates() {
    let t = TempDir::new("hardlinks").unwrap();
    let p = t.write("x.bin", &noise(3, 1000)).unwrap();
    std::fs::hard_link(&p, t.join("x-link.bin")).unwrap();
    let h = Harness::new(index_of(t.path()));
    let out = h.run(&ExactDuplicates).unwrap();
    assert!(out.groups.is_empty());
    assert_eq!(out.diagnostics.skipped.get("hardlink"), Some(&1));
    // A real copy next to them is a duplicate of the file, reported once.
    t.write("x-copy.bin", &noise(3, 1000)).unwrap();
    let h = Harness::new(index_of(t.path()));
    let out = h.run(&ExactDuplicates).unwrap();
    assert_eq!(out.groups.len(), 1);
    assert_eq!(out.groups[0].members.len(), 2);
}

#[cfg(unix)]
#[test]
fn a_hard_linked_copy_is_kept_since_removing_it_frees_nothing() {
    // DUP-4: x.bin has a second name outside the scan, so trashing x.bin frees
    // nothing; the real copy is the one whose removal frees space.
    let t = TempDir::new("hardlinkkeep").unwrap();
    let outside = TempDir::new("hardlinkkeep-out").unwrap();
    let data = noise(4, 5000);
    let x = t.write("x.bin", &data).unwrap();
    std::fs::hard_link(&x, outside.join("x-link.bin")).unwrap();
    let copy = t.write("x-copy.bin", &data).unwrap();
    age(&x, 30);
    age(&copy, 1);
    for rule in ["newest", "oldest", "shortest_path"] {
        let out = Harness::new(index_of(t.path())).text("keep_rule", rule).run(&ExactDuplicates).unwrap();
        assert_one_keeper_per_group(&out);
        assert_eq!(out.groups[0].keeper.as_ref(), Some(&x), "{rule}");
        let selected: Vec<_> = out.candidates.iter().filter(|c| c.default_selected).collect();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].path, copy);
        let x_row = out.candidates.iter().find(|c| c.path == x).unwrap();
        assert_eq!(x_row.saving(), 0, "a hard-linked name frees nothing");
        let total: u64 = out.candidates.iter().filter(|c| c.default_selected).map(|c| c.saving()).sum();
        assert_eq!(total, selected[0].size);
    }
    // An explicit preferred folder still wins; the hard-linked name it leaves
    // behind is listed, not pre-selected, and promises nothing.
    let out = Harness::new(index_of(t.path()))
        .text("keep_rule", "preferred_folder")
        .text("preferred_folder", &t.path().to_string_lossy())
        .run(&ExactDuplicates)
        .unwrap();
    assert_eq!(out.candidates.iter().filter(|c| c.default_selected).count(), 1);
    // Both lie in the preferred folder: the hard-linked one is kept.
    assert_eq!(out.groups[0].keeper.as_ref(), Some(&x));

    // Two hard-linked members: whichever is not kept is listed for review only.
    let y = t.write("y.bin", &data).unwrap();
    std::fs::hard_link(&y, outside.join("y-link.bin")).unwrap();
    let out = Harness::new(index_of(t.path())).text("keep_rule", "newest").run(&ExactDuplicates).unwrap();
    let linked: Vec<_> = out.candidates.iter().filter(|c| c.path == x || c.path == y).collect();
    assert!(linked.iter().all(|c| !c.default_selected && c.saving() == 0));
    assert!(out.candidates.iter().any(|c| c.path == copy && c.default_selected));
}

fn index_with(t: &TempDir, extra: Vec<FileEntry>) -> FileIndex {
    let base = index_of(t.path());
    // Extra entries first: `FileIndex::new` keeps the first of two equal paths.
    let mut entries = extra;
    entries.extend(base.entries().iter().cloned());
    FileIndex::new(base.roots.clone(), entries, ScanStats::default(), Timestamp::now())
}

#[test]
fn unreadable_files_are_reported_never_dropped() {
    let t = TempDir::new("unreadable").unwrap();
    let data = noise(4, BIG);
    t.write("one.bin", &data).unwrap();
    t.write("two.bin", &data).unwrap();
    // Indexed, then gone before hashing (same size as the pair, so it must be read).
    let mut ghost = FileEntry::new(t.join("ghost.bin"), EntryKind::File, BIG as u64);
    ghost.modified = Some(Timestamp(1));
    // A directory masquerading as a file of the same size: reading it fails.
    std::fs::create_dir(t.join("dir.bin")).unwrap();
    let mut dir = FileEntry::new(t.join("dir.bin"), EntryKind::File, BIG as u64);
    dir.modified = Some(Timestamp(1));
    let h = Harness::new(index_with(&t, vec![ghost, dir]));
    let out = h.run(&ExactDuplicates).unwrap();
    assert_eq!(group_names(&out), vec![vec!["one.bin", "two.bin"]]);
    assert_eq!(out.diagnostics.unreadable, 2);
    assert!(out.diagnostics.unreadable_sample.contains(&t.join("ghost.bin")));
}

#[test]
fn size_change_during_scan_is_skipped_not_unique() {
    let t = TempDir::new("changed").unwrap();
    t.write("one.bin", &noise(5, 1000)).unwrap();
    t.write("two.bin", &noise(5, 1000)).unwrap();
    let h = Harness::new(index_of(t.path()));
    t.write("two.bin", &noise(5, 1001)).unwrap();
    let out = h.run(&ExactDuplicates).unwrap();
    assert!(out.groups.is_empty());
    assert_eq!(out.diagnostics.skipped.get("changed_during_scan"), Some(&1));
}

#[test]
fn online_only_placeholders_are_never_read() {
    let t = TempDir::new("online").unwrap();
    t.write("local.bin", &noise(6, 500)).unwrap();
    // A placeholder that does not exist locally: reading it would be an unreadable error.
    let mut p = FileEntry::new(t.join("cloud.bin"), EntryKind::File, 500);
    p.attrs |= Attributes::ONLINE_ONLY;
    let h = Harness::new(index_with(&t, vec![p]));
    let out = h.run(&ExactDuplicates).unwrap();
    assert!(out.groups.is_empty());
    assert_eq!(out.diagnostics.unreadable, 0);
    assert_eq!(out.diagnostics.skipped.get("online_only"), Some(&1));
}

#[test]
fn keep_rules_choose_the_keeper() {
    let t = TempDir::new("keep").unwrap();
    let data = noise(7, 2000);
    let old = t.write("archive/2019/deep/photo.bin", &data).unwrap();
    let new = t.write("photo.bin", &data).unwrap();
    let mid = t.write("pref/photo-copy.bin", &data).unwrap();
    age(&old, 1);
    age(&mid, 50);
    age(&new, 100);
    let cases = [
        ("oldest", "photo.bin", Some(&old)),
        ("newest", "photo.bin", Some(&new)),
        ("shortest_path", "photo.bin", Some(&new)),
        ("preferred_folder", "photo-copy.bin", Some(&mid)),
    ];
    for (rule, keeper, path) in cases {
        let h = Harness::new(index_of(t.path()))
            .text("keep_rule", rule)
            .text("preferred_folder", &t.join("pref").to_string_lossy());
        let out = h.run(&ExactDuplicates).unwrap();
        assert_one_keeper_per_group(&out);
        assert_eq!(keeper_of(&out, "photo.bin"), keeper, "{rule}");
        assert_eq!(out.groups[0].keeper.as_ref(), path, "{rule}");
    }
    // Default rule: oldest.
    let out = Harness::new(index_of(t.path())).run(&ExactDuplicates).unwrap();
    assert_eq!(out.groups[0].keeper.as_ref(), Some(&old));
}

#[test]
fn cancel_stops_the_run() {
    let t = tree();
    let h = Harness::new(index_of(t.path()));
    h.cancel.cancel();
    assert_eq!(h.run(&ExactDuplicates), Err(Error::Cancelled));
}

#[test]
fn serial_worker_count_gives_the_same_groups() {
    let t = tree();
    let mut h = Harness::new(index_of(t.path()));
    h.threads = 1;
    let serial = group_names(&h.run(&ExactDuplicates).unwrap());
    let h = Harness::new(index_of(t.path()));
    assert_eq!(serial, group_names(&h.run(&ExactDuplicates).unwrap()));
}

/// Counts the hashes stored per stage; each `put` is slow, so that two detectors
/// started together both find the cache empty at the start of a stage.
#[derive(Default)]
struct CountingStore {
    inner: mj_core::MemoryFingerprintStore,
    partial: std::sync::atomic::AtomicUsize,
    full: std::sync::atomic::AtomicUsize,
}

impl FingerprintStore for CountingStore {
    fn get(&self, p: &std::path::Path, s: mj_core::FileStamp) -> Option<mj_core::Fingerprints> {
        self.inner.get(p, s)
    }
    fn put(&self, p: &std::path::Path, s: mj_core::FileStamp, fp: mj_core::Fingerprints) {
        use std::sync::atomic::Ordering::SeqCst;
        std::thread::sleep(std::time::Duration::from_millis(40));
        if fp.partial.is_some() {
            self.partial.fetch_add(1, SeqCst);
        }
        if fp.full.is_some() && fp.partial.is_none() {
            self.full.fetch_add(1, SeqCst);
        }
        self.inner.put(p, s, fp)
    }
}

#[test]
fn concurrent_detectors_hash_each_file_once() {
    // DUP-7: the engine runs Exact, Similar, Attachments and Fonts at the same time and
    // each runs the exact lane over its files: a file two of them need is read once.
    use std::sync::atomic::Ordering::SeqCst;
    let t = TempDir::new("singleflight").unwrap();
    let a = noise(21, BIG);
    t.write("one/a.bin", &a).unwrap();
    t.write("two/a.bin", &a).unwrap();
    let index = index_of(t.path());
    let store = CountingStore::default();
    let params = mj_core::ParamValues::new();
    let cancel = mj_core::CancelToken::new();
    let progress = Counter::default();
    let barrier = std::sync::Barrier::new(3);
    let groups: Vec<usize> = std::thread::scope(|s| {
        let runs: Vec<_> = (0..3)
            .map(|_| {
                s.spawn(|| {
                    let ctx = mj_core::DetectContext {
                        index: &index,
                        params: &params,
                        now: Timestamp::now(),
                        fingerprints: &store,
                        cancel: &cancel,
                        progress: &progress,
                        threads: 2,
                    };
                    barrier.wait();
                    mj_core::Detector::run(&ExactDuplicates, &ctx).unwrap().groups.len()
                })
            })
            .collect();
        runs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(groups, [1, 1, 1]);
    assert_eq!(store.partial.load(SeqCst), 2, "one partial hash per file");
    assert_eq!(store.full.load(SeqCst), 2, "one full hash per file");
}
