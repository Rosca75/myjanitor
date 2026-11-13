//! Content fingerprints shared between the scan cache (W2-A) and the duplicate
//! engine (W2-B). Every stage is an explicit `Option`: absent means "not
//! computed", never a zero sentinel (D-006).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::entry::FileStamp;
use crate::flags::flag_set;
use crate::lock;
use crate::pathutil::path_key;
use crate::time::Timestamp;

/// Bump when the meaning of any stored fingerprint changes (window layout, seed, grid,
/// downsampler), not only its shape. Stored in the cache header as `algo_version`.
pub const FINGERPRINT_ALGO_VERSION: u32 = 1;

/// Partial-hash window size (D-005: head, middle and tail windows of 64 KiB each).
pub const PARTIAL_WINDOW: u64 = 64 * 1024;

/// Files up to this size are hashed fully in the partial stage (3 windows would cover them).
pub const PARTIAL_FULL_THRESHOLD: u64 = 3 * PARTIAL_WINDOW;

flag_set! {
    /// Facts about a fingerprint attempt that must survive in the cache.
    pub struct FingerprintFlags: u32 {
        /// A read failed; retry next scan.
        UNREADABLE = 1 << 0 => "unreadable",
        /// Image without an embedded thumbnail: exact-match only (no full decode by design).
        NO_THUMBNAIL = 1 << 1 => "no_thumbnail",
        /// Perceptual hashes were computed from the embedded thumbnail.
        FROM_THUMBNAIL = 1 << 2 => "from_thumbnail",
        /// Image could not be decoded.
        DECODE_FAILED = 1 << 3 => "decode_failed",
    }
}

/// EXIF facts captured from bytes already read (never a new open during hashing).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExifSummary {
    pub date_taken: Option<Timestamp>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub has_gps: bool,
    pub has_description: bool,
}

/// Everything the content stages know about one file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Fingerprints {
    /// xxHash64 over the head+middle+tail windows (or the whole file when small).
    pub partial: Option<u64>,
    /// xxHash64 of the whole file.
    pub full: Option<u64>,
    /// SHA-256 of the whole file (confirmation before deletion; audit).
    pub sha256: Option<[u8; 32]>,
    /// 64-bit difference hash (9x8 box-averaged luma grid).
    pub dhash: Option<u64>,
    /// 64-bit DCT perceptual hash (32x32 grid, 63 coefficients, never 0).
    pub phash: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// `Some` only when EXIF was really read (captured flag).
    pub exif: Option<ExifSummary>,
    pub flags: FingerprintFlags,
}

impl Fingerprints {
    /// Fill every stage `other` has and `self` lacks (later scans add stages lazily).
    pub fn merge_from(&mut self, other: &Fingerprints) {
        self.partial = self.partial.or(other.partial);
        self.full = self.full.or(other.full);
        self.sha256 = self.sha256.or(other.sha256);
        self.dhash = self.dhash.or(other.dhash);
        self.phash = self.phash.or(other.phash);
        self.width = self.width.or(other.width);
        self.height = self.height.or(other.height);
        if self.exif.is_none() {
            self.exif = other.exif.clone();
        }
        self.flags |= other.flags;
    }
}

/// Fingerprint storage keyed by path + validity stamp. Implemented by the scan
/// cache (`mj-scan`) and by [`MemoryFingerprintStore`] for tests.
pub trait FingerprintStore: Send + Sync {
    /// Stored fingerprints, only when the stored stamp equals `stamp` exactly.
    fn get(&self, path: &Path, stamp: FileStamp) -> Option<Fingerprints>;

    /// Store (merging with what is stored under the same stamp; replacing on a different stamp).
    fn put(&self, path: &Path, stamp: FileStamp, fp: Fingerprints);
}

/// In-memory store (tests; also a fallback when the cache is disabled).
#[derive(Debug, Default)]
pub struct MemoryFingerprintStore {
    map: Mutex<HashMap<String, (FileStamp, Fingerprints)>>,
}

impl MemoryFingerprintStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        lock(&self.map).len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl FingerprintStore for MemoryFingerprintStore {
    fn get(&self, path: &Path, stamp: FileStamp) -> Option<Fingerprints> {
        lock(&self.map).get(&path_key(path)).filter(|(s, _)| *s == stamp).map(|(_, f)| f.clone())
    }

    fn put(&self, path: &Path, stamp: FileStamp, fp: Fingerprints) {
        let mut map = lock(&self.map);
        let slot = map.entry(path_key(path)).or_insert_with(|| (stamp, Fingerprints::default()));
        if slot.0 != stamp {
            *slot = (stamp, Fingerprints::default());
        }
        let mut merged = fp;
        merged.merge_from(&slot.1);
        slot.1 = merged;
    }
}

/// A store that remembers nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoFingerprintStore;

impl FingerprintStore for NoFingerprintStore {
    fn get(&self, _: &Path, _: FileStamp) -> Option<Fingerprints> {
        None
    }
    fn put(&self, _: &Path, _: FileStamp, _: Fingerprints) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_mismatch_is_a_miss_and_stages_merge() {
        let s = MemoryFingerprintStore::new();
        let st = FileStamp { size: 10, mtime: Timestamp(5) };
        s.put(Path::new("/a"), st, Fingerprints { partial: Some(1), ..Default::default() });
        s.put(Path::new("/a"), st, Fingerprints { full: Some(2), ..Default::default() });
        let got = s.get(Path::new("/a"), st).unwrap();
        assert_eq!((got.partial, got.full), (Some(1), Some(2)));
        assert!(s.get(Path::new("/a"), FileStamp { size: 10, mtime: Timestamp(6) }).is_none());
        s.put(Path::new("/a"), FileStamp { size: 11, mtime: Timestamp(6) }, Fingerprints::default());
        assert!(s.get(Path::new("/a"), st).is_none());
    }
}
