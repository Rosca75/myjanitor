//! The platform traits and the plain data they exchange. **Frozen contract.**
//!
//! Every method that changes the filesystem lives in [`ActionPlatform`] and is
//! only ever called by `mj-actions`, after its root-allowlist, protected-path,
//! link and stamp checks. Implementations must never follow symlinks or
//! junctions unless a method says so.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use mj_core::{CancelToken, EntryKind, FileEntry, FsCompressAlgo, Result, TrashTicket};
use serde::{Deserialize, Serialize};

/// One directory listing item (no stat yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirItem {
    pub name: OsString,
    pub path: PathBuf,
    /// From the directory entry type (no extra syscall); `Link` is never descended into.
    pub kind: EntryKind,
}

/// Capabilities of a volume, as far as cheaply known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VolumeFeatures {
    pub hardlinks: bool,
    /// Copy-on-write clones (Btrfs/XFS `FICLONE`, ReFS block cloning).
    pub reflinks: bool,
    /// Windows WOF compression (NTFS).
    pub wof: bool,
    /// Btrfs compression property.
    pub btrfs_compression: bool,
    /// A Recycle Bin / Trash exists for this volume.
    pub trash: bool,
    /// MFT enumeration possible (NTFS + volume handle access).
    pub mft: bool,
}

/// A mounted volume / drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeInfo {
    /// Stable id for the session: drive letter (`C:`) or mount point.
    pub id: String,
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub mount: PathBuf,
    /// Volume label ("System", "Projects"), may be empty.
    pub label: String,
    /// Filesystem type (`NTFS`, `ext4`, `btrfs`).
    pub fs_type: String,
    pub total: u64,
    pub free: u64,
    /// Free bytes available to this user.
    pub available: u64,
    pub removable: bool,
    pub features: VolumeFeatures,
}

/// Cloud sync state of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudFileState {
    /// Not managed by a sync engine.
    NotCloud,
    /// Placeholder with local content (can be dehydrated).
    Hydrated,
    /// Online-only: no local content (excluded from totals).
    Dehydrated,
    /// "Always keep on this device".
    Pinned,
}

/// Cloud information for a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudInfo {
    pub state: CloudFileState,
    /// Provider display name when known ("OneDrive - Acme").
    pub provider: Option<String>,
}

/// Kind of sync provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncProvider {
    OneDrive,
    OneDriveBusiness,
    SharePoint,
    Dropbox,
    GoogleDrive,
    Nextcloud,
    Other,
}

/// A synced folder root: deleting below it propagates to the cloud (SAF-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRoot {
    pub provider: SyncProvider,
    /// Display name ("OneDrive - Acme").
    pub name: String,
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub path: PathBuf,
    /// Supports dehydration ("Free up space").
    pub can_dehydrate: bool,
}

/// Read-side platform calls (W2-A).
pub trait ScanPlatform: Send + Sync {
    /// Metadata of `path` **without following links** (lstat semantics). `depth` and
    /// `root` of the returned entry are left 0; the caller sets them.
    fn metadata(&self, path: &Path) -> Result<FileEntry>;

    /// List a directory (unsorted). Never follows links.
    fn read_dir(&self, dir: &Path) -> Result<Vec<DirItem>>;

    /// All mounted volumes worth showing (fixed and removable drives, not pseudo filesystems).
    fn volumes(&self) -> Result<Vec<VolumeInfo>>;

    /// The volume holding `path`.
    fn volume_of(&self, path: &Path) -> Result<VolumeInfo>;

    /// Fast enumeration of everything under `root` (NTFS MFT, SCAN-03). Calls `sink` for each
    /// entry. `Err(Unsupported)` when not possible; the caller then walks directories.
    fn fast_enumerate(&self, root: &Path, sink: &mut dyn FnMut(FileEntry), cancel: &CancelToken) -> Result<()>;

    /// Cloud placeholder state (SCAN-09).
    fn cloud_info(&self, path: &Path) -> Result<CloudInfo>;
}

/// Write-side platform calls (W2-E; filesystem compression W3-B). Only `mj-actions` calls these.
pub trait ActionPlatform: Send + Sync {
    /// Move to the Recycle Bin / freedesktop Trash (CLN-ACT-02).
    fn trash(&self, path: &Path) -> Result<TrashTicket>;

    /// Put a trashed item back at `original` (fails if `original` exists).
    fn restore_trashed(&self, ticket: &TrashTicket, original: &Path) -> Result<()>;

    /// Create hard link `new_link` pointing at `existing` (both on one volume).
    fn hardlink(&self, existing: &Path, new_link: &Path) -> Result<()>;

    /// Create `dst` as a copy-on-write clone of `src` (`FICLONE`, ReFS block cloning).
    fn reflink(&self, src: &Path, dst: &Path) -> Result<()>;

    /// Make a hydrated cloud placeholder online-only ("Free up space", CLN-ACT-07).
    fn dehydrate(&self, path: &Path) -> Result<()>;

    /// Bring an online-only placeholder back locally (undo of `dehydrate`).
    fn hydrate(&self, path: &Path) -> Result<()>;

    /// Synced folder roots of the current user (SAF-10).
    fn sync_roots(&self) -> Result<Vec<SyncRoot>>;

    /// True when running elevated / as root (SAF-05).
    fn is_elevated(&self) -> bool;

    /// Apply transparent compression to one file (OPT-FSC-01/06).
    fn fs_compress(&self, path: &Path, algo: FsCompressAlgo) -> Result<()>;

    /// Remove transparent compression (OPT-FSC-05).
    fn fs_decompress(&self, path: &Path) -> Result<()>;

    /// Current transparent compression of a file, `None` when uncompressed.
    fn fs_compression(&self, path: &Path) -> Result<Option<FsCompressAlgo>>;
}

/// Both halves.
pub trait Platform: ScanPlatform + ActionPlatform {}

impl<T: ScanPlatform + ActionPlatform> Platform for T {}
