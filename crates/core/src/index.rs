//! The in-memory file index a scan produces and every detector reads.
//!
//! Entries are sorted by [`path_key`], which makes every subtree a contiguous
//! range (all keys starting with `"<dir key>/"`), so folder sizes, children and
//! "topmost match" are binary searches rather than tree walks.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::entry::{EntryKind, FileEntry};
use crate::pathutil::path_key;
use crate::time::Timestamp;

/// Counters describing what the scan covered and, just as important, what it could not
/// ("a failed read must never look like no duplicates").
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanStats {
    pub files: u64,
    pub dirs: u64,
    pub links: u64,
    /// Sum of logical file sizes.
    pub bytes: u64,
    /// Sum of allocated sizes (falls back to logical size when unknown).
    pub allocated: u64,
    /// Cloud placeholders whose content is not local (excluded from reclaimable totals).
    pub online_only_files: u64,
    pub online_only_bytes: u64,
    /// Entries that could not be read (stat, open, enumerate).
    pub unreadable: u64,
    /// Up to 10 sample paths of unreadable entries.
    #[serde(with = "crate::pathutil::serde_path::vec")]
    pub unreadable_sample: Vec<PathBuf>,
    /// Of `unreadable`, how many were permission errors.
    pub permission_denied: u64,
    /// Entries skipped by exclusions / protected paths.
    pub excluded: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    /// Wall time of the scan.
    pub elapsed_ms: u64,
    /// True when the NTFS MFT fast path was used for at least one root.
    pub used_fast_index: bool,
}

impl ScanStats {
    /// Maximum paths kept in `unreadable_sample`.
    pub const SAMPLE: usize = 10;

    /// Record an unreadable path (counts it; keeps a bounded sample).
    pub fn record_unreadable(&mut self, path: &Path, permission_denied: bool) {
        self.unreadable += 1;
        if permission_denied {
            self.permission_denied += 1;
        }
        if self.unreadable_sample.len() < Self::SAMPLE {
            self.unreadable_sample.push(path.to_path_buf());
        }
    }
}

/// The result of a scan.
#[derive(Debug, Clone, Default)]
pub struct FileIndex {
    /// The scope roots, in `ScanScope` order.
    pub roots: Vec<PathBuf>,
    /// All entries (roots included), sorted by `path_key`.
    entries: Vec<FileEntry>,
    keys: Vec<String>,
    by_key: HashMap<String, usize>,
    pub stats: ScanStats,
    pub scanned_at: Timestamp,
}

impl FileIndex {
    /// Build an index (sorts entries, drops exact duplicate paths keeping the first).
    pub fn new(roots: Vec<PathBuf>, entries: Vec<FileEntry>, stats: ScanStats, scanned_at: Timestamp) -> Self {
        let mut keyed: Vec<(String, FileEntry)> = entries.into_iter().map(|e| (path_key(&e.path), e)).collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        keyed.dedup_by(|a, b| a.0 == b.0);
        let (keys, entries): (Vec<String>, Vec<FileEntry>) = keyed.into_iter().unzip();
        let by_key = keys.iter().enumerate().map(|(i, k)| (k.clone(), i)).collect();
        FileIndex { roots, entries, keys, by_key, stats, scanned_at }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All entries in key order.
    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    /// Entry at position `i`.
    pub fn entry(&self, i: usize) -> &FileEntry {
        &self.entries[i]
    }

    /// Regular files.
    pub fn files(&self) -> impl Iterator<Item = &FileEntry> {
        self.entries.iter().filter(|e| e.kind == EntryKind::File)
    }

    /// Directories.
    pub fn dirs(&self) -> impl Iterator<Item = &FileEntry> {
        self.entries.iter().filter(|e| e.kind == EntryKind::Dir)
    }

    /// Position of a path.
    pub fn position(&self, path: &Path) -> Option<usize> {
        self.by_key.get(&path_key(path)).copied()
    }

    /// Entry for a path.
    pub fn get(&self, path: &Path) -> Option<&FileEntry> {
        self.position(path).map(|i| &self.entries[i])
    }

    /// Positions of every descendant of entry `i` (empty for files).
    pub fn subtree(&self, i: usize) -> Range<usize> {
        let k = &self.keys[i];
        let prefix = if k.ends_with('/') { k.clone() } else { format!("{k}/") };
        let start = self.keys.partition_point(|x| x.as_str() < prefix.as_str());
        let len = self.keys[start..].partition_point(|x| x.starts_with(&prefix));
        start..start + len
    }

    /// Positions of the direct children of entry `i`.
    pub fn children(&self, i: usize) -> Vec<usize> {
        let depth = self.entries[i].path.components().count();
        self.subtree(i).filter(|&j| self.entries[j].path.components().count() == depth + 1).collect()
    }

    /// Position of the parent of entry `i`, when it is in the index.
    pub fn parent(&self, i: usize) -> Option<usize> {
        self.entries[i].path.parent().and_then(|p| self.position(p))
    }

    /// Totals of the subtree under entry `i` (the entry itself included when it is a file).
    pub fn totals(&self, i: usize) -> SubtreeTotals {
        let mut t = SubtreeTotals::default();
        let add = |t: &mut SubtreeTotals, e: &FileEntry| match e.kind {
            EntryKind::File => {
                t.files += 1;
                t.bytes += e.size;
                t.reclaimable += e.reclaimable();
                t.newest_modified = t.newest_modified.max(e.modified);
                t.newest_accessed = t.newest_accessed.max(e.accessed);
            }
            EntryKind::Dir => t.dirs += 1,
            _ => {}
        };
        add(&mut t, &self.entries[i]);
        for j in self.subtree(i) {
            add(&mut t, &self.entries[j]);
        }
        t
    }
}

/// Aggregates over a subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubtreeTotals {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub reclaimable: u64,
    pub newest_modified: Option<Timestamp>,
    pub newest_accessed: Option<Timestamp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> FileIndex {
        let e = |p: &str, k, s| FileEntry::new(p, k, s);
        FileIndex::new(
            vec![PathBuf::from("/r")],
            vec![
                e("/r", EntryKind::Dir, 0),
                e("/r/a", EntryKind::Dir, 0),
                e("/r/a/x.txt", EntryKind::File, 10),
                e("/r/a/sub", EntryKind::Dir, 0),
                e("/r/a/sub/y.txt", EntryKind::File, 5),
                e("/r/a-b", EntryKind::Dir, 0),
                e("/r/a-b/z.txt", EntryKind::File, 7),
            ],
            ScanStats::default(),
            Timestamp(0),
        )
    }

    #[test]
    fn subtree_is_contiguous_and_exact() {
        let ix = idx();
        let a = ix.position(Path::new("/r/a")).unwrap();
        let names: Vec<_> = ix.subtree(a).map(|j| ix.entry(j).path.clone()).collect();
        assert_eq!(names.len(), 3);
        assert!(names.iter().all(|p| p.starts_with("/r/a/")));
        let t = ix.totals(a);
        assert_eq!((t.files, t.dirs, t.bytes), (2, 2, 15));
        assert_eq!(ix.children(a).len(), 2);
        let root = ix.position(Path::new("/r")).unwrap();
        assert_eq!(ix.totals(root).bytes, 22);
        assert_eq!(ix.parent(a), Some(root));
    }
}
