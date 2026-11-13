//! Shared helpers for the mj-scan integration tests: a platform wrapper that
//! injects faults around the real `NativePlatform`, and fixture builders.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use mj_core::testutil::{set_mtime, TempDir};
use mj_core::{Attributes, CancelToken, EntryKind, Error, FileEntry, FileId, Progress, Result};
use mj_platform::{CloudInfo, DirItem, NativePlatform, ScanPlatform, VolumeInfo};

/// How `fast_enumerate` behaves.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fast {
    /// `Unsupported`, like Linux.
    Off,
    /// Enumerate the real tree (walking natively), like a working MFT.
    Works,
    /// Emit some entries, then fail (the scan must discard them and walk).
    FailsMidway,
}

/// `NativePlatform` with injected behaviour.
pub struct TestPlatform {
    pub inner: NativePlatform,
    /// `read_dir` of these fails with permission denied.
    pub deny_list: Vec<PathBuf>,
    /// `metadata` of these fails with permission denied.
    pub deny_stat: Vec<PathBuf>,
    /// Cancel this token on the N-th `read_dir` call.
    pub cancel_at: Option<(usize, CancelToken)>,
    pub calls: AtomicUsize,
    /// Every file is reported as an online-only cloud placeholder.
    pub online_only: bool,
    /// Entries under these report another volume (a mounted filesystem).
    pub other_volume: Vec<PathBuf>,
    /// These directories report the given file id (bind-mount loops).
    pub forced_ids: Vec<(PathBuf, FileId)>,
    pub fast: Fast,
    /// Paths `read_dir` was called on.
    pub listed: Mutex<Vec<PathBuf>>,
}

impl Default for TestPlatform {
    fn default() -> Self {
        TestPlatform {
            inner: NativePlatform::new(),
            deny_list: Vec::new(),
            deny_stat: Vec::new(),
            cancel_at: None,
            calls: AtomicUsize::new(0),
            online_only: false,
            other_volume: Vec::new(),
            forced_ids: Vec::new(),
            fast: Fast::Off,
            listed: Mutex::new(Vec::new()),
        }
    }
}

impl TestPlatform {
    fn walk_native(&self, dir: &Path, out: &mut Vec<FileEntry>) {
        for item in self.read_dir(dir).unwrap_or_default() {
            if let Ok(e) = self.metadata(&item.path) {
                let is_dir = e.kind == EntryKind::Dir;
                out.push(e);
                if is_dir {
                    self.walk_native(&item.path, out);
                }
            }
        }
    }
}

impl ScanPlatform for TestPlatform {
    fn metadata(&self, path: &Path) -> Result<FileEntry> {
        if self.deny_stat.iter().any(|p| p == path) {
            return Err(Error::PermissionDenied(path.to_path_buf()));
        }
        let mut e = self.inner.metadata(path)?;
        if self.online_only && e.kind == EntryKind::File {
            e.attrs |= Attributes::ONLINE_ONLY;
            e.allocated = Some(0);
        }
        if self.other_volume.iter().any(|p| path.starts_with(p)) {
            if let Some(id) = e.file_id.as_mut() {
                id.volume = id.volume.wrapping_add(1);
            }
        }
        if let Some((_, id)) = self.forced_ids.iter().find(|(p, _)| p == path) {
            e.file_id = Some(*id);
        }
        Ok(e)
    }

    fn read_dir(&self, dir: &Path) -> Result<Vec<DirItem>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some((at, token)) = &self.cancel_at {
            if n == *at {
                token.cancel();
            }
        }
        self.listed.lock().unwrap().push(dir.to_path_buf());
        if self.deny_list.iter().any(|p| p == dir) {
            return Err(Error::PermissionDenied(dir.to_path_buf()));
        }
        self.inner.read_dir(dir)
    }

    fn volumes(&self) -> Result<Vec<VolumeInfo>> {
        self.inner.volumes()
    }

    fn volume_of(&self, path: &Path) -> Result<VolumeInfo> {
        self.inner.volume_of(path)
    }

    fn fast_enumerate(&self, root: &Path, sink: &mut dyn FnMut(FileEntry), cancel: &CancelToken) -> Result<()> {
        match self.fast {
            Fast::Off => Err(Error::unsupported("Fast indexing", "test")),
            Fast::Works => {
                let mut all = Vec::new();
                self.walk_native(root, &mut all);
                for e in all {
                    cancel.checkpoint()?;
                    sink(e);
                }
                Ok(())
            }
            Fast::FailsMidway => {
                let mut all = Vec::new();
                self.walk_native(root, &mut all);
                for e in all.into_iter().take(2) {
                    sink(e);
                }
                Err(Error::Io { path: None, kind: "other", message: "volume went away".into() })
            }
        }
    }

    fn cloud_info(&self, path: &Path) -> Result<CloudInfo> {
        self.inner.cloud_info(path)
    }
}

/// Collects progress updates.
#[derive(Default)]
pub struct Recorder(pub Mutex<Vec<Progress>>);

impl mj_core::ProgressSink for Recorder {
    fn report(&self, p: &Progress) {
        self.0.lock().unwrap().push(p.clone());
    }
}

/// A file time well outside the cache trust window.
pub fn old_time() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000)
}

/// Write a file with an old mtime.
pub fn write_old(t: &TempDir, rel: &str, content: &[u8]) -> PathBuf {
    let p = t.write(rel, content).unwrap();
    set_mtime(&p, old_time()).unwrap();
    p
}

/// A small mixed tree: 6 files, 4 directories below the root.
pub fn small_tree(t: &TempDir) {
    write_old(t, "docs/a.txt", b"aaaa");
    write_old(t, "docs/b.txt", b"bb");
    write_old(t, "docs/deep/c.log", b"cccccc");
    write_old(t, "photos/p1.jpg", &[1u8; 100]);
    write_old(t, "photos/raw/p2.raw", &[2u8; 50]);
    write_old(t, "top.bin", b"z");
}
