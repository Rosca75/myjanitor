//! Folder tree with aggregated sizes over a [`FileIndex`] (SCAN-05/06/07/08).
//!
//! [`FolderTree::build`] computes, in one O(n) pass, the parent of every
//! entry and the totals of every subtree (files, folders, logical, allocated
//! and reclaimable bytes, online-only placeholders, newest modification), so
//! a folder tree sorted by size, treemap nodes and breakdowns never re-walk
//! subtrees. The index is sorted by `path_key`, so an ancestor always precedes
//! its descendants and one reverse pass accumulates every subtree.
//!
//! Memory: two `u32` per entry plus one [`TreeTotals`] per entry (a transient
//! map of folder paths while building); no path is copied.
//!
//! Breakdowns (by extension, type group, age, owner) count local files only:
//! online-only placeholders hold no bytes on this disk (SCAN-09) and are
//! reported separately in [`TreeTotals`].

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use mj_core::time::NANOS_PER_DAY;
use mj_core::{EntryKind, FileEntry, FileIndex, Timestamp};

const NONE: u32 = u32::MAX;

/// Aggregates of one subtree (the entry itself included).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TreeTotals {
    pub files: u64,
    pub dirs: u64,
    pub links: u64,
    /// Logical bytes of local files.
    pub bytes: u64,
    /// Allocated bytes of local files (logical size when unknown), SCAN-08.
    pub allocated: u64,
    /// What removing everything would free on this disk (`FileEntry::reclaimable`).
    pub reclaimable: u64,
    pub online_only_files: u64,
    pub online_only_bytes: u64,
    pub newest_modified: Option<Timestamp>,
}

impl TreeTotals {
    fn add_entry(&mut self, e: &FileEntry) {
        match e.kind {
            EntryKind::File => {
                self.files += 1;
                if e.is_online_only() {
                    self.online_only_files += 1;
                    self.online_only_bytes += e.size;
                } else {
                    self.bytes += e.size;
                    self.allocated += e.allocated.unwrap_or(e.size);
                    self.reclaimable += e.reclaimable();
                }
                self.newest_modified = self.newest_modified.max(e.modified);
            }
            EntryKind::Dir => self.dirs += 1,
            EntryKind::Link => self.links += 1,
            EntryKind::Other => {}
        }
    }

    fn add(&mut self, o: &TreeTotals) {
        self.files += o.files;
        self.dirs += o.dirs;
        self.links += o.links;
        self.bytes += o.bytes;
        self.allocated += o.allocated;
        self.reclaimable += o.reclaimable;
        self.online_only_files += o.online_only_files;
        self.online_only_bytes += o.online_only_bytes;
        self.newest_modified = self.newest_modified.max(o.newest_modified);
    }

    /// Allocated / logical bytes (1.0 = no compression; < 1.0 compressed or sparse), SCAN-08.
    pub fn allocation_ratio(&self) -> Option<f64> {
        (self.bytes > 0).then(|| self.allocated as f64 / self.bytes as f64)
    }
}

/// Parents, children and subtree totals of an index.
#[derive(Debug, Clone)]
pub struct FolderTree<'a> {
    index: &'a FileIndex,
    parent: Vec<u32>,
    totals: Vec<TreeTotals>,
    /// Children in CSR form: `kids[offsets[i]..offsets[i + 1]]`.
    offsets: Vec<u32>,
    kids: Vec<u32>,
}

impl<'a> FolderTree<'a> {
    /// Build the tree in O(n).
    pub fn build(index: &'a FileIndex) -> FolderTree<'a> {
        let entries = index.entries();
        let n = entries.len();
        // Parent = nearest indexed ancestor directory. Looked up by path, not by a
        // stack over the key order: "/r/a-b" sorts between "/r/a" and "/r/a/x".
        let dirs: HashMap<&Path, u32> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == EntryKind::Dir)
            .map(|(i, e)| (e.path.as_path(), i as u32))
            .collect();
        let mut parent = vec![NONE; n];
        for (i, e) in entries.iter().enumerate() {
            let mut up = e.path.parent();
            while let Some(p) = up {
                if let Some(&j) = dirs.get(p) {
                    parent[i] = j;
                    break;
                }
                up = p.parent();
            }
        }
        let mut totals = vec![TreeTotals::default(); n];
        for i in (0..n).rev() {
            totals[i].add_entry(&entries[i]);
            if parent[i] != NONE {
                let t = totals[i];
                totals[parent[i] as usize].add(&t);
            }
        }
        let mut offsets = vec![0u32; n + 1];
        for &p in &parent {
            if p != NONE {
                offsets[p as usize + 1] += 1;
            }
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }
        let mut fill = offsets.clone();
        let mut kids = vec![0u32; offsets[n] as usize];
        for (i, &p) in parent.iter().enumerate() {
            if p != NONE {
                kids[fill[p as usize] as usize] = i as u32;
                fill[p as usize] += 1;
            }
        }
        FolderTree { index, parent, totals, offsets, kids }
    }

    /// The index.
    pub fn index(&self) -> &'a FileIndex {
        self.index
    }

    /// Totals of the subtree at entry `i`.
    pub fn totals(&self, i: usize) -> &TreeTotals {
        &self.totals[i]
    }

    /// Parent position (nearest indexed ancestor).
    pub fn parent(&self, i: usize) -> Option<usize> {
        (self.parent[i] != NONE).then_some(self.parent[i] as usize)
    }

    /// Direct children of entry `i`, in index (path) order.
    pub fn children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        self.kids[self.offsets[i] as usize..self.offsets[i + 1] as usize].iter().map(|&k| k as usize)
    }

    /// Direct children sorted by size, largest first (ties by name), SCAN-06.
    pub fn children_by_size(&self, i: usize) -> Vec<usize> {
        let mut c: Vec<usize> = self.children(i).collect();
        let entries = self.index.entries();
        c.sort_by(|&a, &b| {
            self.totals[b].bytes.cmp(&self.totals[a].bytes).then_with(|| entries[a].path.cmp(&entries[b].path))
        });
        c
    }

    /// Positions of entries without an indexed parent (the scan roots).
    pub fn tops(&self) -> Vec<usize> {
        (0..self.parent.len()).filter(|&i| self.parent[i] == NONE).collect()
    }

    /// The `n` largest folders anywhere, by local bytes.
    pub fn largest_folders(&self, n: usize) -> Vec<usize> {
        let entries = self.index.entries();
        let mut dirs: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].kind == EntryKind::Dir).collect();
        dirs.sort_by(|&a, &b| self.totals[b].bytes.cmp(&self.totals[a].bytes).then(a.cmp(&b)));
        dirs.truncate(n);
        dirs
    }

    /// Positions of the local files of the subtree at `i` (or the whole index for `None`).
    fn files_in(&self, at: Option<usize>) -> impl Iterator<Item = &'a FileEntry> + 'a {
        let index = self.index;
        let range = match at {
            Some(i) if index.entry(i).kind == EntryKind::Dir => index.subtree(i),
            Some(i) => i..i + 1,
            None => 0..index.len(),
        };
        index.entries()[range].iter().filter(|e| e.kind == EntryKind::File && !e.is_online_only())
    }

    /// Breakdown by a key function over local files, largest first (ties by key).
    pub fn breakdown_by(&self, at: Option<usize>, key: impl Fn(&FileEntry) -> String) -> Vec<BreakdownRow> {
        let mut m: HashMap<String, (u64, u64)> = HashMap::new();
        for e in self.files_in(at) {
            let s = m.entry(key(e)).or_default();
            s.0 += e.size;
            s.1 += 1;
        }
        let mut rows: Vec<BreakdownRow> =
            m.into_iter().map(|(key, (bytes, files))| BreakdownRow { label: key.clone(), key, bytes, files }).collect();
        rows.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.key.cmp(&b.key)));
        rows
    }

    /// By lower-case extension (`""` = no extension, labelled "No extension").
    pub fn by_extension(&self, at: Option<usize>) -> Vec<BreakdownRow> {
        let mut rows = self.breakdown_by(at, |e| e.extension());
        for r in &mut rows {
            r.label = if r.key.is_empty() { "No extension".into() } else { format!(".{}", r.key) };
        }
        rows
    }

    /// By type group: each extension maps to the first group (in key order) listing it;
    /// anything else is `other`.
    pub fn by_group(&self, at: Option<usize>, groups: &BTreeMap<String, Vec<String>>) -> Vec<BreakdownRow> {
        let mut ext_to_group: HashMap<String, &str> = HashMap::new();
        for (g, exts) in groups {
            for x in exts {
                ext_to_group.entry(x.trim_start_matches('.').to_lowercase()).or_insert(g.as_str());
            }
        }
        self.breakdown_by(at, |e| ext_to_group.get(&e.extension()).map(|g| g.to_string()).unwrap_or("other".into()))
    }

    /// By modification age relative to `now` ([`AGE_BUCKETS`], plus `unknown`).
    pub fn by_age(&self, at: Option<usize>, now: Timestamp) -> Vec<BreakdownRow> {
        let mut rows = self.breakdown_by(at, |e| age_bucket(e.modified, now).to_string());
        for r in &mut rows {
            r.label = AGE_BUCKETS
                .iter()
                .find(|b| b.0 == r.key)
                .map(|b| b.1.to_string())
                .unwrap_or_else(|| "Unknown date".into());
        }
        let order = |k: &str| AGE_BUCKETS.iter().position(|b| b.0 == k).unwrap_or(AGE_BUCKETS.len());
        rows.sort_by_key(|r| order(&r.key));
        rows
    }

    /// By owner (`uid:1000` on Linux; `unknown` when not collected).
    pub fn by_owner(&self, at: Option<usize>) -> Vec<BreakdownRow> {
        self.breakdown_by(at, |e| e.owner.clone().unwrap_or_else(|| "unknown".into()))
    }
}

/// One breakdown row (mirrors the engine's `BreakdownRow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakdownRow {
    pub key: String,
    pub label: String,
    pub bytes: u64,
    pub files: u64,
}

/// Age buckets: key, label, upper bound in days (exclusive).
pub const AGE_BUCKETS: &[(&str, &str, i64)] = &[
    ("lt_1m", "Less than a month", 30),
    ("1m_6m", "1 to 6 months", 182),
    ("6m_1y", "6 months to a year", 365),
    ("1y_2y", "1 to 2 years", 730),
    ("2y_5y", "2 to 5 years", 1826),
    ("gt_5y", "More than 5 years", i64::MAX),
];

/// The age bucket key of a modification time. Future times count as "less than a month".
pub fn age_bucket(modified: Option<Timestamp>, now: Timestamp) -> &'static str {
    let Some(m) = modified else { return "unknown" };
    let days = now.0.saturating_sub(m.0).max(0) / NANOS_PER_DAY;
    AGE_BUCKETS.iter().find(|b| days < b.2).map(|b| b.0).unwrap_or("gt_5y")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::{Attributes, ScanStats};
    use std::path::PathBuf;

    fn idx() -> FileIndex {
        let f = |p: &str, s: u64, days_old: i64| {
            let mut e = FileEntry::new(p, EntryKind::File, s);
            e.allocated = Some(s.div_ceil(4096) * 4096);
            e.modified = Some(Timestamp(1000 * NANOS_PER_DAY - days_old * NANOS_PER_DAY));
            e.owner = Some(if s > 50 { "uid:0".into() } else { "uid:1000".into() });
            e
        };
        let d = |p: &str| FileEntry::new(p, EntryKind::Dir, 0);
        let mut cloud = f("/r/a/cloud.docx", 5000, 1);
        cloud.attrs |= Attributes::ONLINE_ONLY;
        FileIndex::new(
            vec![PathBuf::from("/r")],
            vec![
                d("/r"),
                d("/r/a"),
                f("/r/a/x.jpg", 10, 5),
                d("/r/a/sub"),
                f("/r/a/sub/y.JPG", 100, 400),
                d("/r/a-b"),
                f("/r/a-b/z.txt", 7, 3000),
                f("/r/README", 1, 100),
                cloud,
                FileEntry::new("/r/link", EntryKind::Link, 0),
            ],
            ScanStats::default(),
            Timestamp(0),
        )
    }

    #[test]
    fn totals_parents_and_children() {
        let ix = idx();
        let t = FolderTree::build(&ix);
        let pos = |p: &str| ix.position(Path::new(p)).unwrap();
        let root = t.totals(pos("/r"));
        assert_eq!((root.files, root.dirs, root.links, root.bytes), (5, 4, 1, 118));
        assert_eq!((root.online_only_files, root.online_only_bytes), (1, 5000));
        assert_eq!(root.allocated, 4 * 4096);
        let a = t.totals(pos("/r/a"));
        assert_eq!((a.files, a.bytes, a.dirs), (3, 110, 2));
        assert_eq!(t.parent(pos("/r/a/sub/y.JPG")), Some(pos("/r/a/sub")));
        assert_eq!(t.parent(pos("/r/a-b")), Some(pos("/r")));
        assert_eq!(t.parent(pos("/r")), None);
        assert_eq!(t.tops(), vec![pos("/r")]);
        // Agrees with the index's own (slower) totals.
        assert_eq!(ix.totals(pos("/r/a")).bytes, 110 + 5000);
        let kids: Vec<_> = t.children_by_size(pos("/r")).into_iter().map(|i| ix.entry(i).name()).collect();
        assert_eq!(kids, vec!["a", "a-b", "README", "link"]);
        assert_eq!(t.children(pos("/r/a")).count(), 3);
        assert_eq!(t.largest_folders(2), vec![pos("/r"), pos("/r/a")]);
        assert!(t.totals(pos("/r/a/sub")).allocation_ratio().unwrap() > 1.0);
    }

    #[test]
    fn breakdowns() {
        let ix = idx();
        let t = FolderTree::build(&ix);
        let ext = t.by_extension(None);
        assert_eq!(ext[0].key, "jpg");
        assert_eq!((ext[0].bytes, ext[0].files, ext[0].label.as_str()), (110, 2, ".jpg"));
        assert!(ext.iter().any(|r| r.key.is_empty() && r.label == "No extension"));
        assert!(!ext.iter().any(|r| r.key == "docx"), "online-only files are not local bytes");
        let mut groups = BTreeMap::new();
        groups.insert("images".to_string(), vec!["jpg".to_string(), ".png".to_string()]);
        let g = t.by_group(None, &groups);
        assert_eq!((g[0].key.as_str(), g[0].bytes), ("images", 110));
        assert_eq!(g[1].key, "other");
        let now = Timestamp(1000 * NANOS_PER_DAY);
        let age: Vec<_> = t.by_age(None, now).into_iter().map(|r| r.key).collect();
        assert_eq!(age, vec!["lt_1m", "1m_6m", "1y_2y", "gt_5y"]);
        let owners = t.by_owner(None);
        assert_eq!((owners[0].key.as_str(), owners[0].bytes), ("uid:0", 100));
        let sub = ix.position(Path::new("/r/a")).unwrap();
        assert_eq!(t.by_extension(Some(sub)).iter().map(|r| r.files).sum::<u64>(), 2);
        assert_eq!(age_bucket(None, now), "unknown");
        assert_eq!(age_bucket(Some(Timestamp(now.0 + 5)), now), "lt_1m");
    }

    #[test]
    fn empty_index() {
        let ix = FileIndex::default();
        let t = FolderTree::build(&ix);
        assert!(t.tops().is_empty());
        assert!(t.by_extension(None).is_empty());
    }
}
