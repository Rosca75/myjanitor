//! The scan cache (SCAN-04): a persistent [`FingerprintStore`]. **Owner: W2-A.**
//!
//! Design (ARCHITECTURE.md §6): one global store `<cache dir>/scan-cache.bin`
//! keyed by `path_key`, validity stamp `{size, mtime_ns}` compared by exact
//! equality, explicit `Option` stages, binary format in [`format`], atomic save
//! (`.tmp` + `sync_all` + rename), any decode problem = empty cache, pruning
//! of entries not seen under a scanned root. Cache failures are never fatal.
//!
//! Wave 1: in-memory only (`open` loads nothing, `save` reports NotImplemented).

pub mod format;

use std::path::{Path, PathBuf};

use mj_core::{Error, FileIndex, FileStamp, FingerprintStore, Fingerprints, MemoryFingerprintStore, Result};

/// The cache.
#[derive(Debug, Default)]
pub struct ScanCache {
    file: Option<PathBuf>,
    store: MemoryFingerprintStore,
}

impl ScanCache {
    /// A cache that is never persisted.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load `file`; a missing, corrupt or outdated file gives an empty cache.
    pub fn open(file: &Path) -> Self {
        ScanCache { file: Some(file.to_path_buf()), store: MemoryFingerprintStore::new() }
    }

    /// Backing file, if persistent.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Entries held.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    /// Persist atomically.
    pub fn save(&self) -> Result<()> {
        Err(Error::not_implemented("scan cache persistence (W2-A)"))
    }

    /// Drop entries under `index.roots` whose path is not in `index` (D-006). Returns entries removed.
    pub fn prune(&self, index: &FileIndex) -> usize {
        let _ = index;
        0
    }

    /// Remove everything (memory and file). Returns entries removed.
    pub fn clear(&self) -> Result<u64> {
        Err(Error::not_implemented("scan cache clear (W2-A)"))
    }
}

impl FingerprintStore for ScanCache {
    fn get(&self, path: &Path, stamp: FileStamp) -> Option<Fingerprints> {
        self.store.get(path, stamp)
    }

    fn put(&self, path: &Path, stamp: FileStamp, fp: Fingerprints) {
        self.store.put(path, stamp, fp)
    }
}
