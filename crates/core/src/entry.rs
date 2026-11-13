//! What the scanner knows about one filesystem entry.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::flags::flag_set;
use crate::time::Timestamp;

/// Kind of entry. Symlinks and junctions are recorded as [`EntryKind::Link`]
/// and never followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    /// Symlink, junction, mount point or other name-surrogate reparse point.
    Link,
    /// Device, socket, FIFO...
    Other,
}

flag_set! {
    /// File attributes, normalised across platforms (CLN-FLT-05, SCAN-08/09).
    /// Serialised as an array of snake_case names, e.g. `["hidden","readonly"]`.
    pub struct Attributes: u32 {
        HIDDEN = 1 << 0 => "hidden",
        SYSTEM = 1 << 1 => "system",
        READONLY = 1 << 2 => "readonly",
        ARCHIVE = 1 << 3 => "archive",
        COMPRESSED = 1 << 4 => "compressed",
        SPARSE = 1 << 5 => "sparse",
        ENCRYPTED = 1 << 6 => "encrypted",
        /// FILE_ATTRIBUTE_OFFLINE.
        OFFLINE = 1 << 7 => "offline",
        TEMPORARY = 1 << 8 => "temporary",
        REPARSE_POINT = 1 << 9 => "reparse_point",
        /// Cloud placeholder whose content is not local (RECALL_ON_DATA_ACCESS / RECALL_ON_OPEN).
        ONLINE_ONLY = 1 << 10 => "online_only",
        /// Cloud file pinned "always keep on this device".
        PINNED = 1 << 11 => "pinned",
        /// Cloud placeholder that is fully hydrated but may be dehydrated ("free up space").
        CLOUD_LOCAL = 1 << 12 => "cloud_local",
        /// WOF (Windows Overlay Filter) compressed file.
        WOF_COMPRESSED = 1 << 13 => "wof_compressed",
        /// Linux immutable attribute.
        IMMUTABLE = 1 << 14 => "immutable",
    }
}

/// Stable identity of a file on its volume, when cheap to obtain:
/// Linux `st_dev` + `st_ino`; Windows volume serial + file index (only from
/// MFT enumeration or an open handle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FileId {
    pub volume: u64,
    pub index: u64,
}

/// The cache-validity stamp of a file: equality of both fields means "unchanged".
/// Also used by the action engine to refuse acting on a file changed since the scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileStamp {
    pub size: u64,
    pub mtime: Timestamp,
}

/// One filesystem entry as recorded by the scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Absolute path, as given by the walk (not canonicalised).
    #[serde(with = "crate::pathutil::serde_path")]
    pub path: PathBuf,
    pub kind: EntryKind,
    /// Logical size in bytes (0 for directories; see `FileIndex` aggregates).
    pub size: u64,
    /// Bytes allocated on disk, when known (SCAN-08). For online-only placeholders this is 0.
    pub allocated: Option<u64>,
    pub created: Option<Timestamp>,
    pub modified: Option<Timestamp>,
    pub accessed: Option<Timestamp>,
    /// Linux ctime (inode change), an extra cache stamp; `None` on Windows.
    pub changed: Option<Timestamp>,
    pub attrs: Attributes,
    pub file_id: Option<FileId>,
    /// Hard-link count, when known.
    pub links: Option<u32>,
    /// Owner: `"uid:1000"` on Linux, a SID string on Windows; `None` when not collected.
    pub owner: Option<String>,
    /// Components below the scan root that contains it (0 = the root itself).
    pub depth: u32,
    /// Index into `ScanScope::roots` of the root this entry was found under.
    pub root: u16,
}

impl FileEntry {
    /// A minimal entry (tests, fixtures, synthetic entries).
    pub fn new(path: impl Into<PathBuf>, kind: EntryKind, size: u64) -> Self {
        FileEntry {
            path: path.into(),
            kind,
            size,
            allocated: None,
            created: None,
            modified: None,
            accessed: None,
            changed: None,
            attrs: Attributes::empty(),
            file_id: None,
            links: None,
            owner: None,
            depth: 0,
            root: 0,
        }
    }

    /// True for regular files.
    pub fn is_file(&self) -> bool {
        self.kind == EntryKind::File
    }

    /// True for directories.
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }

    /// Validity stamp (mtime 0 when unknown).
    pub fn stamp(&self) -> FileStamp {
        FileStamp { size: self.size, mtime: self.modified.unwrap_or_default() }
    }

    /// True when content is not stored locally (excluded from reclaimable totals, SCAN-09).
    pub fn is_online_only(&self) -> bool {
        self.attrs.contains(Attributes::ONLINE_ONLY) || self.attrs.contains(Attributes::OFFLINE)
    }

    /// Bytes that removing this entry would actually free on this disk:
    /// the allocated size when known, else the logical size; 0 for online-only files.
    pub fn reclaimable(&self) -> u64 {
        if self.is_online_only() {
            0
        } else {
            self.allocated.unwrap_or(self.size)
        }
    }

    /// Lower-case extension without the dot.
    pub fn extension(&self) -> String {
        crate::pathutil::extension_lower(&self.path)
    }

    /// File name (lossy).
    pub fn name(&self) -> String {
        crate::pathutil::file_name_lossy(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_serialize_as_names() {
        let a = Attributes::HIDDEN | Attributes::READONLY;
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(json, r#"["hidden","readonly"]"#);
        let back: Attributes = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
        assert!(serde_json::from_str::<Attributes>(r#"["nope"]"#).is_err());
    }

    #[test]
    fn reclaimable_excludes_online_only() {
        let mut e = FileEntry::new("/x", EntryKind::File, 100);
        e.allocated = Some(4096);
        assert_eq!(e.reclaimable(), 4096);
        e.attrs |= Attributes::ONLINE_ONLY;
        assert_eq!(e.reclaimable(), 0);
    }
}
