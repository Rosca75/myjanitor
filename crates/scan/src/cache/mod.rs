//! The scan cache (SCAN-04): a persistent [`FingerprintStore`]. **Owner: W2-A.**
//!
//! Design (ARCHITECTURE.md §6, D-006, D-106):
//! - one global store `<cache dir>/scan-cache.bin`, keyed by `path_key`
//!   (lexically normalised, case-folded on Windows), so overlapping roots share
//!   entries;
//! - an entry is valid only on an **exact** `{size, mtime_ns}` match; a stamp
//!   whose mtime is within 2 s of the scan start (or in the future) is not
//!   trusted: it may still be written, or the filesystem's mtime is coarse;
//! - every file a scan sees gets a stamp-only record (`fp: None`); content
//!   stages (partial/full xxh64, SHA-256, perceptual hashes, dimensions, EXIF,
//!   flags) are explicit `Option`s other crates fill through `put`, merged
//!   under the same stamp and dropped on a different stamp;
//! - a file id (`st_dev` + `st_ino`, Windows volume serial + file index)
//!   carries fingerprints across renames and to new hard links when
//!   `(id, size, mtime)` match, because the content is then the same;
//! - in-house binary format with a checksum trailer ([`format`]); atomic save
//!   (`.tmp` in the same directory, `sync_all`, rename); a missing, corrupt or
//!   outdated file gives an empty cache and one log line, never an error;
//! - pruning after a completed scan drops entries under a walked root that the
//!   scan did not see (deleted, moved or excluded files) and everything under
//!   roots that no longer exist, but keeps entries below directories that
//!   could not be read and never prunes a partial scan (include patterns or a
//!   depth limit), so a transient error never throws the cache away.
//!
//! Locks are never held across file I/O: `save` encodes into memory under a
//! read lock, then writes without it.

pub mod format;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Mutex, RwLock};

use mj_core::pathutil::path_key;
use mj_core::time::NANOS_PER_SEC;
use mj_core::{
    CancelToken, Error, FileEntry, FileId, FileIndex, FileStamp, FingerprintStore, Fingerprints, Result, Timestamp,
};

use crate::walker::{RootState, WalkOutcome};
use format::{is_empty_fp, CacheRecord};

/// A stamp whose mtime is this close to the scan start is not trusted.
pub const UNTRUSTED_WINDOW_NS: i64 = 2 * NANOS_PER_SEC;

#[derive(Debug, Default)]
struct Inner {
    map: HashMap<Box<str>, CacheRecord>,
    /// File id -> key, only for records that hold fingerprints (rename carry-over).
    by_id: HashMap<FileId, Box<str>>,
}

impl Inner {
    fn index_id(&mut self, key: &str) {
        if let Some(r) = self.map.get(key) {
            if let (Some(id), Some(_)) = (r.file_id, &r.fp) {
                self.by_id.insert(id, key.into());
            }
        }
    }
}

/// What the last observed scan covered (pruning needs it).
#[derive(Debug)]
struct Coverage {
    epoch: u32,
    scanned_at: Timestamp,
    complete: bool,
    roots: Vec<(String, RootState)>,
    holes: Vec<String>,
}

/// The cache.
#[derive(Debug, Default)]
pub struct ScanCache {
    file: Option<PathBuf>,
    inner: RwLock<Inner>,
    /// Start of the current / last scan (ns), the anchor of the trust window; 0 = unset.
    scan_start: AtomicI64,
    epoch: AtomicU32,
    coverage: Mutex<Option<Coverage>>,
}

fn under(key: &str, root: &str) -> bool {
    key == root
        || (key.len() > root.len()
            && key.starts_with(root)
            && (root.ends_with('/') || key.as_bytes()[root.len()] == b'/'))
}

fn read_lock(l: &RwLock<Inner>) -> std::sync::RwLockReadGuard<'_, Inner> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

fn write_lock(l: &RwLock<Inner>) -> std::sync::RwLockWriteGuard<'_, Inner> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

fn tmp_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

impl ScanCache {
    /// A cache that is never persisted.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load `file`; a missing, corrupt or outdated file gives an empty cache.
    pub fn open(file: &Path) -> Self {
        let cache = ScanCache { file: Some(file.to_path_buf()), ..Default::default() };
        match fs::read(file) {
            Ok(bytes) => match format::decode(&bytes) {
                Ok(records) => {
                    let mut inner = write_lock(&cache.inner);
                    inner.map.reserve(records.len());
                    for (k, r) in records {
                        inner.map.insert(k, r);
                    }
                    let keys: Vec<Box<str>> = inner
                        .map
                        .iter()
                        .filter(|(_, r)| r.fp.is_some() && r.file_id.is_some())
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in keys {
                        inner.index_id(&k);
                    }
                }
                Err(why) => eprintln!("[mj-scan] scan cache {} ignored and rebuilt: {why}", file.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("[mj-scan] scan cache {} unreadable, starting empty: {e}", file.display()),
        }
        cache
    }

    /// Backing file, if persistent.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Entries held.
    pub fn len(&self) -> usize {
        read_lock(&self.inner).map.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when a stamp may be trusted (not modified within the window before the scan start).
    pub fn trusted(&self, mtime: Timestamp) -> bool {
        let anchor = match self.scan_start.load(Ordering::Relaxed) {
            0 => Timestamp::now().0,
            t => t,
        };
        mtime.0 <= anchor.saturating_sub(UNTRUSTED_WINDOW_NS)
    }

    /// Record what a completed walk saw: every local file gets (or keeps) a
    /// record under its stamp; returns `(hits, misses)`. Hits are files whose
    /// trusted stamp matched exactly; everything else is a miss whose old
    /// fingerprints are dropped (or carried over from a file with the same id
    /// and stamp). Online-only files are neither: they have no local content.
    pub fn observe(&self, walk: &WalkOutcome, started: Timestamp, workers: usize) -> (u64, u64) {
        self.scan_start.store(started.0, Ordering::Relaxed);
        let epoch = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let index = &walk.index;
        let files: Vec<&FileEntry> = index.files().filter(|e| !e.is_online_only()).collect();
        // Keys and read-only lookups in parallel (path_key costs a little per file).
        let looked = {
            let inner = read_lock(&self.inner);
            mj_core::pool::parallel_map(&files, workers, &CancelToken::new(), |_, e| {
                let key = path_key(&e.path);
                let st = e.stamp();
                let hit = self.trusted(st.mtime)
                    && inner.map.get(key.as_str()).is_some_and(|r| r.size == st.size && r.mtime_ns == st.mtime.0);
                (key, hit)
            })
        };
        let (mut hits, mut misses) = (0u64, 0u64);
        let mut inner = write_lock(&self.inner);
        for (e, slot) in files.iter().zip(looked) {
            let Some((key, hit)) = slot else { continue };
            let st = e.stamp();
            if hit {
                hits += 1;
                if let Some(r) = inner.map.get_mut(key.as_str()) {
                    r.seen = epoch;
                    if r.file_id.is_none() && e.file_id.is_some() {
                        r.file_id = e.file_id;
                        inner.index_id(&key);
                    }
                }
                continue;
            }
            misses += 1;
            let mut rec = CacheRecord::stamp(st.size, st.mtime, e.file_id);
            rec.seen = epoch;
            if let (Some(id), true) = (e.file_id, self.trusted(st.mtime)) {
                let donor = inner.by_id.get(&id).and_then(|k| inner.map.get(k));
                if let Some(d) =
                    donor.filter(|d| d.size == st.size && d.mtime_ns == st.mtime.0 && d.file_id == Some(id))
                {
                    rec.fp = d.fp.clone();
                }
            }
            inner.map.insert(key.clone().into_boxed_str(), rec);
            inner.index_id(&key);
        }
        drop(inner);
        let cov = Coverage {
            epoch,
            scanned_at: index.scanned_at,
            complete: walk.complete,
            roots: index.roots.iter().zip(&walk.roots).map(|(r, s)| (path_key(r), *s)).collect(),
            holes: walk.holes.iter().map(|h| path_key(h)).collect(),
        };
        *mj_core::lock(&self.coverage) = Some(cov);
        (hits, misses)
    }

    /// Persist atomically (`<file>.tmp`, `sync_all`, rename). In-memory caches do nothing.
    pub fn save(&self) -> Result<()> {
        let Some(file) = &self.file else { return Ok(()) };
        let bytes = {
            let inner = read_lock(&self.inner);
            let mut items: Vec<(&str, &CacheRecord)> = inner.map.iter().map(|(k, r)| (&**k, r)).collect();
            items.sort_unstable_by(|a, b| a.0.cmp(b.0));
            let mut buf = Vec::with_capacity(items.len() * 64 + 64);
            format::encode(&mut buf, items.len() as u64, items.into_iter()).map_err(|e| Error::io(file, e))?;
            buf
        };
        let tmp = tmp_path(file);
        let write = || -> std::io::Result<()> {
            if let Some(dir) = file.parent() {
                fs::create_dir_all(dir)?;
            }
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&tmp, file)
        };
        write().map_err(|e| {
            let _ = fs::remove_file(&tmp);
            Error::io(file, e)
        })
    }

    /// Drop entries no completed scan can vouch for any more (D-006). Returns entries removed.
    ///
    /// Uses what [`ScanCache::observe`] recorded for this same `index` (from
    /// [`crate::scan_with_cache`]): under every walked root, entries the scan
    /// did not see are dropped unless they lie below an unreadable directory;
    /// under roots that no longer exist, everything is dropped; partial scans
    /// (includes, depth limit) only drop vanished roots. Without that record
    /// (an index from plain [`crate::scan`]) only vanished roots are pruned.
    pub fn prune(&self, index: &FileIndex) -> usize {
        let recorded = {
            let cov = mj_core::lock(&self.coverage);
            cov.as_ref().filter(|c| c.scanned_at == index.scanned_at).map(|c| {
                let gone: Vec<String> =
                    c.roots.iter().filter(|r| r.1 == RootState::Gone).map(|r| r.0.clone()).collect();
                let walked: Vec<String> = if c.complete {
                    c.roots.iter().filter(|r| r.1 == RootState::Walked).map(|r| r.0.clone()).collect()
                } else {
                    Vec::new()
                };
                (gone, walked, c.holes.clone(), c.epoch)
            })
        };
        let (gone, walked, holes, epoch) = recorded.unwrap_or_else(|| {
            let gone = index
                .roots
                .iter()
                .filter(|r| matches!(fs::symlink_metadata(r), Err(e) if e.kind() == std::io::ErrorKind::NotFound))
                .map(|r| path_key(r))
                .collect();
            (gone, Vec::new(), Vec::new(), 0)
        });
        if gone.is_empty() && walked.is_empty() {
            return 0;
        }
        let mut inner = write_lock(&self.inner);
        let before = inner.map.len();
        inner.map.retain(|k, r| {
            if gone.iter().any(|g| under(k, g)) {
                return false;
            }
            let in_walked = walked.iter().any(|w| under(k, w));
            !in_walked || r.seen == epoch || holes.iter().any(|h| under(k, h))
        });
        let removed = before - inner.map.len();
        if removed > 0 {
            let Inner { map, by_id } = &mut *inner;
            by_id.retain(|_, k| map.contains_key(&**k));
        }
        removed
    }

    /// Remove everything (memory and file). Returns entries removed.
    pub fn clear(&self) -> Result<u64> {
        let n = {
            let mut inner = write_lock(&self.inner);
            let n = inner.map.len() as u64;
            inner.map = HashMap::new();
            inner.by_id = HashMap::new();
            n
        };
        *mj_core::lock(&self.coverage) = None;
        if let Some(file) = &self.file {
            for p in [file.clone(), tmp_path(file)] {
                match fs::remove_file(&p) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(Error::io(&p, e)),
                }
            }
        }
        Ok(n)
    }

    /// Keys of every record (tests and diagnostics), sorted.
    pub fn keys(&self) -> Vec<String> {
        let mut k: Vec<String> = read_lock(&self.inner).map.keys().map(|k| k.to_string()).collect();
        k.sort();
        k
    }

    /// Distinct keys with fingerprints (diagnostics).
    pub fn fingerprinted(&self) -> usize {
        read_lock(&self.inner).map.values().filter(|r| r.fp.is_some()).count()
    }

    /// Keys under any of `roots` (diagnostics).
    pub fn count_under(&self, roots: &[PathBuf]) -> usize {
        let keys: HashSet<String> = roots.iter().map(|r| path_key(r)).collect();
        read_lock(&self.inner).map.keys().filter(|k| keys.iter().any(|r| under(k, r))).count()
    }
}

impl FingerprintStore for ScanCache {
    fn get(&self, path: &Path, stamp: FileStamp) -> Option<Fingerprints> {
        if !self.trusted(stamp.mtime) {
            return None;
        }
        let key = path_key(path);
        let inner = read_lock(&self.inner);
        let r = inner.map.get(key.as_str())?;
        if r.size != stamp.size || r.mtime_ns != stamp.mtime.0 {
            return None;
        }
        r.fp.as_deref().cloned()
    }

    fn put(&self, path: &Path, stamp: FileStamp, fp: Fingerprints) {
        let key = path_key(path);
        let trusted = self.trusted(stamp.mtime);
        let mut inner = write_lock(&self.inner);
        if !inner.map.contains_key(key.as_str()) {
            inner.map.insert(key.clone().into_boxed_str(), CacheRecord::stamp(stamp.size, stamp.mtime, None));
        }
        let Some(slot) = inner.map.get_mut(key.as_str()) else { return };
        if slot.size != stamp.size || slot.mtime_ns != stamp.mtime.0 {
            let seen = slot.seen;
            *slot = CacheRecord::stamp(stamp.size, stamp.mtime, None);
            slot.seen = seen;
        }
        if !trusted {
            // Content of a file that may still change under the same stamp is never kept.
            slot.fp = None;
            return;
        }
        let mut merged = fp;
        if let Some(old) = &slot.fp {
            merged.merge_from(old);
        }
        slot.fp = (!is_empty_fp(&merged)).then(|| Box::new(merged));
        inner.index_id(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::EntryKind;

    fn old() -> Timestamp {
        Timestamp(1_600_000_000 * NANOS_PER_SEC)
    }

    fn st(size: u64) -> FileStamp {
        FileStamp { size, mtime: old() }
    }

    fn fp(partial: u64) -> Fingerprints {
        Fingerprints { partial: Some(partial), ..Default::default() }
    }

    #[test]
    fn exact_stamp_lookup_merge_and_replace() {
        let c = ScanCache::in_memory();
        let p = Path::new("/a/b");
        c.put(p, st(10), fp(1));
        c.put(p, st(10), Fingerprints { full: Some(2), ..Default::default() });
        let got = c.get(p, st(10)).unwrap();
        assert_eq!((got.partial, got.full), (Some(1), Some(2)));
        assert!(c.get(p, st(11)).is_none());
        assert!(c.get(p, FileStamp { size: 10, mtime: Timestamp(old().0 + 1) }).is_none());
        c.put(p, st(11), Fingerprints::default());
        assert!(c.get(p, st(10)).is_none());
        assert!(c.get(p, st(11)).is_none(), "an empty record yields nothing");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn recent_stamps_are_not_trusted() {
        let c = ScanCache::in_memory();
        let now = FileStamp { size: 1, mtime: Timestamp::now() };
        c.put(Path::new("/x"), now, fp(5));
        assert!(c.get(Path::new("/x"), now).is_none());
        let future = FileStamp { size: 1, mtime: Timestamp(Timestamp::now().0 + 3600 * NANOS_PER_SEC) };
        assert!(!c.trusted(future.mtime));
        assert!(c.trusted(old()));
    }

    #[test]
    fn save_and_reopen_round_trip_and_atomic_temp() {
        let t = TempDir::new("cache-rt").unwrap();
        let file = t.join("sub/scan-cache.bin");
        let c = ScanCache::open(&file);
        assert!(c.is_empty());
        c.put(Path::new("/a"), st(1), fp(7));
        c.put(Path::new("/b"), st(2), Fingerprints { sha256: Some([1; 32]), ..Default::default() });
        c.save().unwrap();
        assert!(file.exists());
        assert!(!tmp_path(&file).exists());
        let d = ScanCache::open(&file);
        assert_eq!(d.len(), 2);
        assert_eq!(d.get(Path::new("/a"), st(1)).unwrap().partial, Some(7));
        assert_eq!(d.get(Path::new("/b"), st(2)).unwrap().sha256, Some([1; 32]));
        // Deterministic bytes.
        let first = fs::read(&file).unwrap();
        d.save().unwrap();
        assert_eq!(fs::read(&file).unwrap(), first);
        assert_eq!(d.clear().unwrap(), 2);
        assert!(!file.exists());
        assert!(d.is_empty());
        assert_eq!(d.clear().unwrap(), 0);
        assert!(ScanCache::in_memory().save().is_ok());
    }

    #[test]
    fn corrupted_or_old_files_give_an_empty_cache() {
        let t = TempDir::new("cache-bad").unwrap();
        let file = t.join("scan-cache.bin");
        let c = ScanCache::open(&file);
        c.put(Path::new("/a"), st(1), fp(7));
        c.save().unwrap();
        let good = fs::read(&file).unwrap();
        for bad in [good[..good.len() - 1].to_vec(), b"garbage".to_vec(), Vec::new(), {
            let mut b = good.clone();
            b[30] ^= 1;
            b
        }] {
            fs::write(&file, &bad).unwrap();
            let d = ScanCache::open(&file);
            assert!(d.is_empty());
            // The rebuilt cache saves over the bad file.
            d.put(Path::new("/z"), st(3), fp(1));
            d.save().unwrap();
            assert_eq!(ScanCache::open(&file).len(), 1);
        }
        // A directory where the file should be: unreadable, still not an error.
        let dir = t.mkdir("isdir").unwrap();
        assert!(ScanCache::open(&dir).is_empty());
        assert!(ScanCache::open(&dir).save().is_err());
    }

    #[test]
    fn under_is_component_wise() {
        assert!(under("/a/b", "/a"));
        assert!(under("/a", "/a"));
        assert!(!under("/ab", "/a"));
        assert!(under("/x", "/"));
        assert!(under("c:/x", "c:/"));
    }

    fn walk(roots: &[&str], files: &[(&str, u64)], holes: &[&str], complete: bool) -> WalkOutcome {
        let entries = files
            .iter()
            .map(|(p, s)| {
                let mut e = FileEntry::new(*p, EntryKind::File, *s);
                e.modified = Some(old());
                e
            })
            .collect();
        WalkOutcome {
            index: FileIndex::new(
                roots.iter().map(PathBuf::from).collect(),
                entries,
                Default::default(),
                Timestamp::now(),
            ),
            roots: roots.iter().map(|_| RootState::Walked).collect(),
            holes: holes.iter().map(PathBuf::from).collect(),
            complete,
        }
    }

    #[test]
    fn observe_counts_hits_and_prune_respects_holes() {
        let c = ScanCache::in_memory();
        let w1 = walk(&["/r"], &[("/r/a", 1), ("/r/b", 2), ("/r/locked/c", 3)], &[], true);
        assert_eq!(c.observe(&w1, Timestamp::now(), 2), (0, 3));
        c.put(Path::new("/r/a"), st(1), fp(9));
        c.put(Path::new("/other/x"), st(1), fp(1));
        // Second scan: b deleted, locked/ unreadable, a unchanged, d new.
        let w2 = walk(&["/r"], &[("/r/a", 1), ("/r/d", 4)], &["/r/locked"], true);
        assert_eq!(c.observe(&w2, Timestamp::now(), 2), (1, 1));
        assert_eq!(c.prune(&w2.index), 1);
        assert_eq!(c.keys(), vec!["/other/x", "/r/a", "/r/d", "/r/locked/c"]);
        assert_eq!(c.get(Path::new("/r/a"), st(1)).unwrap().partial, Some(9));
        // A prune with an unrelated index only drops roots that no longer exist.
        assert_eq!(c.prune(&walk(&["/"], &[], &[], true).index), 0);
        let t = TempDir::new("cache-gone").unwrap();
        let gone = t.join("vanished");
        c.put(&gone.join("f"), st(1), fp(1));
        assert_eq!(c.prune(&walk(&[gone.to_str().unwrap()], &[], &[], true).index), 1);
        // Partial scans never prune what they did not see.
        let w3 = walk(&["/r"], &[("/r/a", 1)], &[], false);
        c.observe(&w3, Timestamp::now(), 1);
        assert_eq!(c.prune(&w3.index), 0);
        // A vanished root drops everything under it.
        let mut w4 = walk(&["/other"], &[], &[], true);
        w4.roots = vec![RootState::Gone];
        c.observe(&w4, Timestamp::now(), 1);
        assert_eq!(c.prune(&w4.index), 1);
        assert_eq!(c.count_under(&[PathBuf::from("/other")]), 0);
    }

    #[test]
    fn file_ids_carry_fingerprints_across_renames() {
        let c = ScanCache::in_memory();
        let id = FileId { volume: 1, index: 42 };
        let mut w1 = walk(&["/r"], &[("/r/old.bin", 100)], &[], true);
        let mut e = w1.index.entries()[0].clone();
        e.file_id = Some(id);
        w1.index = FileIndex::new(vec!["/r".into()], vec![e.clone()], Default::default(), Timestamp::now());
        c.observe(&w1, Timestamp::now(), 1);
        c.put(Path::new("/r/old.bin"), st(100), fp(77));
        let mut moved = e;
        moved.path = "/r/new.bin".into();
        let w2 = WalkOutcome {
            index: FileIndex::new(vec!["/r".into()], vec![moved], Default::default(), Timestamp::now()),
            ..walk(&["/r"], &[], &[], true)
        };
        assert_eq!(c.observe(&w2, Timestamp::now(), 1), (0, 1));
        assert_eq!(c.get(Path::new("/r/new.bin"), st(100)).unwrap().partial, Some(77));
        assert_eq!(c.fingerprinted(), 2);
        c.prune(&w2.index);
        assert_eq!(c.keys(), vec!["/r/new.bin"]);
    }
}
