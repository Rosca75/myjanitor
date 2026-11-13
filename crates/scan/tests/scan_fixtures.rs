//! Integration tests of the scan engine on generated fixture trees: links and
//! loops, unreadable entries, several roots, cancel and pause, the fast-index
//! path, cloud placeholders, other filesystems and the warm rescan with the cache.

mod common;

use std::path::Path;
use std::time::Duration;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::{CancelToken, EntryKind, Error, FileIndex, FingerprintStore, Fingerprints, NullProgress, ScanScope};
use mj_platform::{NativePlatform, ScanPlatform};
use mj_scan::{scan, scan_with_cache, FolderTree, ScanCache, ScanRequest};

fn scope(roots: &[&Path]) -> ScanScope {
    ScanScope { roots: roots.iter().map(|r| r.to_path_buf()).collect(), ..Default::default() }
}

fn run(scope: &ScanScope, p: &dyn ScanPlatform) -> mj_core::Result<FileIndex> {
    let cancel = CancelToken::new();
    scan(&ScanRequest { scope, exclusions: &[], platform: p, cancel: &cancel, progress: &NullProgress })
}

fn paths(ix: &FileIndex, root: &Path) -> Vec<String> {
    ix.entries()
        .iter()
        .map(|e| e.path.strip_prefix(root).unwrap_or(&e.path).to_string_lossy().replace('\\', "/"))
        .collect()
}

#[test]
fn walks_a_tree_with_totals_and_depths() {
    let t = TempDir::new("it-basic").unwrap();
    small_tree(&t);
    let ix = run(&scope(&[t.path()]), &NativePlatform::new()).unwrap();
    assert_eq!((ix.stats.files, ix.stats.dirs, ix.stats.bytes), (6, 5, 163));
    assert_eq!(ix.stats.unreadable, 0);
    assert!(!ix.stats.used_fast_index);
    assert_eq!(ix.get(&t.join("docs/deep/c.log")).unwrap().depth, 3);
    let tree = FolderTree::build(&ix);
    let root = ix.position(t.path()).unwrap();
    assert_eq!(tree.totals(root).bytes, 163);
    let top: Vec<String> = tree.children_by_size(root).into_iter().map(|i| ix.entry(i).name()).collect();
    assert_eq!(top, vec!["photos", "docs", "top.bin"]);
    // Deterministic: a second scan gives the same entries (atime of listed folders may move).
    let again = run(&scope(&[t.path()]), &NativePlatform::new()).unwrap();
    assert_eq!(paths(&ix, t.path()), paths(&again, t.path()));
    assert!(ix.entries().iter().zip(again.entries()).all(|(a, b)| (a.size, a.modified) == (b.size, b.modified)));
}

#[cfg(unix)]
#[test]
fn symlink_loops_are_recorded_and_never_followed() {
    use std::os::unix::fs::symlink;
    let t = TempDir::new("it-loop").unwrap();
    small_tree(&t);
    symlink(t.path(), t.join("docs/loop-to-root")).unwrap();
    symlink(t.join("docs"), t.join("photos/raw/loop-to-docs")).unwrap();
    symlink(t.join("top.bin"), t.join("file-link")).unwrap();
    symlink(t.join("missing"), t.join("broken")).unwrap();
    let ix = run(&scope(&[t.path()]), &NativePlatform::new()).unwrap();
    assert_eq!(ix.stats.links, 4);
    assert_eq!(ix.stats.files, 6, "no file counted twice through a link");
    assert_eq!(ix.get(&t.join("docs/loop-to-root")).unwrap().kind, EntryKind::Link);
    assert!(ix.get(&t.join("docs/loop-to-root/top.bin")).is_none());
    assert!(ix.get(&t.join("photos/raw/loop-to-docs/a.txt")).is_none());
    // A root that is itself a link is recorded, not followed.
    let ix = run(&scope(&[&t.join("photos/raw/loop-to-docs")]), &NativePlatform::new()).unwrap();
    assert_eq!((ix.len(), ix.stats.links, ix.stats.files), (1, 1, 0));
}

#[test]
fn unreadable_directories_and_files_are_reported() {
    let t = TempDir::new("it-denied").unwrap();
    small_tree(&t);
    let p =
        TestPlatform { deny_list: vec![t.join("photos")], deny_stat: vec![t.join("docs/b.txt")], ..Default::default() };
    let ix = run(&scope(&[t.path()]), &p).unwrap();
    assert_eq!(ix.stats.unreadable, 2);
    assert_eq!(ix.stats.permission_denied, 2);
    assert!(ix.stats.unreadable_sample.contains(&t.join("photos")));
    assert!(ix.stats.unreadable_sample.contains(&t.join("docs/b.txt")));
    // The unreadable directory itself is still listed; its content is not.
    assert_eq!(ix.get(&t.join("photos")).unwrap().kind, EntryKind::Dir);
    assert!(ix.get(&t.join("photos/p1.jpg")).is_none());
    assert_eq!(ix.stats.files, 3);
}

#[cfg(unix)]
#[test]
fn real_permission_denied_when_not_root() {
    use std::os::unix::fs::PermissionsExt;
    if mj_platform::ffi::linux::effective_uid() == 0 {
        return; // root reads everything; covered by the injected test above
    }
    let t = TempDir::new("it-chmod").unwrap();
    small_tree(&t);
    let locked = t.join("photos");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let ix = run(&scope(&[t.path()]), &NativePlatform::new());
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    let ix = ix.unwrap();
    assert_eq!(ix.stats.permission_denied, 1);
    assert_eq!(ix.stats.unreadable_sample, vec![locked]);
}

#[test]
fn several_roots_overlapping_roots_and_exclusions() {
    let a = TempDir::new("it-root-a").unwrap();
    let b = TempDir::new("it-root-b").unwrap();
    small_tree(&a);
    write_old(&b, "x/y.dat", b"0123456789");
    write_old(&b, "cache/junk.tmp", b"j");
    let mut s = scope(&[a.path(), &a.join("docs"), b.path(), &a.join("photos/raw")]);
    s.exclude = vec!["cache".into()];
    let cancel = CancelToken::new();
    let ix = scan(&ScanRequest {
        scope: &s,
        exclusions: &["*.raw".into()],
        platform: &NativePlatform::new(),
        cancel: &cancel,
        progress: &NullProgress,
    })
    .unwrap();
    assert_eq!(ix.roots.len(), 2, "nested roots collapse into their parent");
    assert_eq!(ix.stats.files, 5 + 1);
    assert_eq!(ix.stats.excluded, 2);
    let yi = ix.get(&b.join("x/y.dat")).unwrap();
    let ai = ix.get(&a.join("docs/a.txt")).unwrap();
    assert_ne!(yi.root, ai.root);
    assert_eq!(ix.roots[usize::from(yi.root)], b.path());
    assert!(ix.get(&b.join("cache")).is_none());
    // No path appears twice.
    let mut keys: Vec<_> = ix.entries().iter().map(|e| e.path.clone()).collect();
    keys.dedup();
    assert_eq!(keys.len(), ix.len());
}

#[test]
fn cancel_mid_scan_and_pause_resume() {
    let t = TempDir::new("it-cancel").unwrap();
    for i in 0..30 {
        write_old(&t, &format!("d{i}/e/f.txt"), b"1");
    }
    let token = CancelToken::new();
    let p = TestPlatform { cancel_at: Some((5, token.clone())), ..Default::default() };
    let s = scope(&[t.path()]);
    let r = scan(&ScanRequest { scope: &s, exclusions: &[], platform: &p, cancel: &token, progress: &NullProgress });
    assert_eq!(r.unwrap_err(), Error::Cancelled);
    assert!(p.listed.lock().unwrap().len() < 61, "the walk stopped early");

    // Pause blocks the walk; resume lets it finish.
    let token = CancelToken::new();
    token.pause();
    let t2 = token.clone();
    let root = t.path().to_path_buf();
    let h = std::thread::spawn(move || {
        let s = scope(&[&root]);
        let p = NativePlatform::new();
        scan(&ScanRequest { scope: &s, exclusions: &[], platform: &p, cancel: &t2, progress: &NullProgress })
            .map(|ix| ix.stats.files)
    });
    std::thread::sleep(Duration::from_millis(80));
    assert!(!h.is_finished(), "paused scans wait");
    token.resume();
    assert_eq!(h.join().unwrap().unwrap(), 30);
}

#[test]
fn progress_is_throttled_and_ends_with_a_final_update() {
    let t = TempDir::new("it-progress").unwrap();
    small_tree(&t);
    let rec = Recorder::default();
    let s = scope(&[t.path()]);
    let cancel = CancelToken::new();
    let p = NativePlatform::new();
    scan(&ScanRequest { scope: &s, exclusions: &[], platform: &p, cancel: &cancel, progress: &rec }).unwrap();
    let got = rec.0.lock().unwrap();
    assert!(!got.is_empty() && got.len() < 10);
    let last = got.last().unwrap();
    assert_eq!(last.total, Some(last.done));
    assert_eq!(last.done, 11);
    assert_eq!(last.bytes, 163);
}

#[test]
fn fast_index_path_applies_the_same_rules_and_falls_back() {
    let t = TempDir::new("it-fast").unwrap();
    small_tree(&t);
    write_old(&t, ".hidden/secret.txt", b"s");
    write_old(&t, "photos/skip/x.jpg", b"x");
    let mut s = scope(&[t.path()]);
    s.exclude = vec![t.join("photos/skip").to_string_lossy().into_owned()];
    s.options.include_hidden = false;
    s.options.max_depth = Some(2);
    let walked = run(&s, &NativePlatform::new()).unwrap();
    let fast = run(&s, &TestPlatform { fast: Fast::Works, ..Default::default() }).unwrap();
    assert!(fast.stats.used_fast_index);
    assert_eq!(paths(&fast, t.path()), paths(&walked, t.path()));
    assert_eq!(fast.stats.files, walked.stats.files);
    assert_eq!(fast.stats.excluded, walked.stats.excluded);
    assert!(fast.get(&t.join("docs/deep/c.log")).is_none(), "depth limit");
    // A failure midway discards the partial result and walks.
    let fallback = run(&s, &TestPlatform { fast: Fast::FailsMidway, ..Default::default() }).unwrap();
    assert!(!fallback.stats.used_fast_index);
    assert_eq!(paths(&fallback, t.path()), paths(&walked, t.path()));
    // Disabled by option: never asked.
    s.options.use_fast_index = false;
    assert!(!run(&s, &TestPlatform { fast: Fast::Works, ..Default::default() }).unwrap().stats.used_fast_index);
}

#[test]
fn online_only_placeholders_are_excluded_from_totals() {
    let t = TempDir::new("it-cloud").unwrap();
    small_tree(&t);
    let ix = run(&scope(&[t.path()]), &TestPlatform { online_only: true, ..Default::default() }).unwrap();
    assert_eq!((ix.stats.files, ix.stats.bytes, ix.stats.allocated), (6, 0, 0));
    assert_eq!((ix.stats.online_only_files, ix.stats.online_only_bytes), (6, 163));
    let tree = FolderTree::build(&ix);
    let root = ix.position(t.path()).unwrap();
    assert_eq!((tree.totals(root).bytes, tree.totals(root).online_only_files), (0, 6));
}

#[test]
fn other_filesystems_and_bind_loops_are_not_entered() {
    let t = TempDir::new("it-onefs").unwrap();
    small_tree(&t);
    let root_id = NativePlatform::new().metadata(t.path()).unwrap().file_id;
    let p = TestPlatform {
        other_volume: vec![t.join("photos")],
        forced_ids: root_id.map(|id| vec![(t.join("docs/deep"), id)]).unwrap_or_default(),
        ..Default::default()
    };
    let ix = run(&scope(&[t.path()]), &p).unwrap();
    assert!(ix.get(&t.join("photos")).is_none(), "a mount point is not indexed");
    if root_id.is_some() {
        assert!(ix.get(&t.join("docs/deep")).is_none(), "a directory seen twice is not entered again");
    }
    assert_eq!(ix.stats.excluded, if root_id.is_some() { 2 } else { 1 });
    let mut s = scope(&[t.path()]);
    s.options.one_file_system = false;
    let p = TestPlatform { other_volume: vec![t.join("photos")], ..Default::default() };
    assert!(run(&s, &p).unwrap().get(&t.join("photos/p1.jpg")).is_some());
}

#[test]
fn warm_rescan_uses_the_cache() {
    let t = TempDir::new("it-cache").unwrap();
    let data = TempDir::new("it-cache-file").unwrap();
    small_tree(&t);
    let file = data.join("scan-cache.bin");
    let s = scope(&[t.path()]);
    let p = NativePlatform::new();
    let cancel = CancelToken::new();
    let req = ScanRequest { scope: &s, exclusions: &[], platform: &p, cancel: &cancel, progress: &NullProgress };

    // Cold: every file is a miss; a hash stage is stored for one file.
    let cache = ScanCache::open(&file);
    let cold = scan_with_cache(&req, &cache).unwrap();
    assert_eq!((cold.stats.cache_hits, cold.stats.cache_misses), (0, 6));
    let p1 = t.join("photos/p1.jpg");
    let st = cold.get(&p1).unwrap().stamp();
    cache.put(&p1, st, Fingerprints { partial: Some(42), full: Some(43), ..Default::default() });
    assert_eq!(cache.prune(&cold), 0);
    cache.save().unwrap();

    // Warm, from disk: every file hits and the fingerprints are back.
    let cache = ScanCache::open(&file);
    assert_eq!(cache.len(), 6);
    let warm = scan_with_cache(&req, &cache).unwrap();
    assert_eq!((warm.stats.cache_hits, warm.stats.cache_misses), (6, 0));
    assert_eq!(cache.get(&p1, st).unwrap().full, Some(43));

    // A changed file misses and loses its stale stages; a deleted one is pruned.
    write_old(&t, "photos/p1.jpg", &[9u8; 101]);
    std::fs::remove_file(t.join("top.bin")).unwrap();
    let third = scan_with_cache(&req, &cache).unwrap();
    assert_eq!((third.stats.cache_hits, third.stats.cache_misses), (4, 1));
    assert!(cache.get(&p1, third.get(&p1).unwrap().stamp()).is_none());
    assert_eq!(cache.prune(&third), 1);
    cache.save().unwrap();
    assert_eq!(ScanCache::open(&file).len(), 5);

    // Toggle off: the cache is not consulted or changed.
    let mut off = s.clone();
    off.options.use_cache = false;
    let req = ScanRequest { scope: &off, exclusions: &[], platform: &p, cancel: &cancel, progress: &NullProgress };
    let ix = scan_with_cache(&req, &cache).unwrap();
    assert_eq!((ix.stats.cache_hits, ix.stats.cache_misses), (0, 0));

    // Clear: memory and file.
    assert_eq!(cache.clear().unwrap(), 5);
    assert!(!file.exists());
    assert_eq!(cache.file(), Some(file.as_path()));
}

#[test]
fn freshly_written_files_are_never_cache_hits() {
    let t = TempDir::new("it-fresh").unwrap();
    t.write("new.txt", b"just written").unwrap();
    let s = scope(&[t.path()]);
    let p = NativePlatform::new();
    let cancel = CancelToken::new();
    let req = ScanRequest { scope: &s, exclusions: &[], platform: &p, cancel: &cancel, progress: &NullProgress };
    let cache = ScanCache::in_memory();
    let ix = scan_with_cache(&req, &cache).unwrap();
    let st = ix.get(&t.join("new.txt")).unwrap().stamp();
    cache.put(&t.join("new.txt"), st, Fingerprints { partial: Some(1), ..Default::default() });
    assert!(cache.get(&t.join("new.txt"), st).is_none());
    let ix = scan_with_cache(&req, &cache).unwrap();
    assert_eq!(ix.stats.cache_hits, 0);
}
