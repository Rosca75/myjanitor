//! [`NativePlatform`]: the real implementation, delegating each trait method to
//! the free function of the owning module. Owners change their module files,
//! never this one.

use std::path::Path;

use mj_core::{CancelToken, FileEntry, FsCompressAlgo, Result, TrashTicket};

use crate::traits::{ActionPlatform, CloudInfo, DirItem, ScanPlatform, SyncRoot, VolumeInfo};
use crate::{action, scan};

/// The platform of the running OS. Stateless; cheap to clone.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativePlatform;

impl NativePlatform {
    /// The platform.
    pub fn new() -> Self {
        NativePlatform
    }
}

impl ScanPlatform for NativePlatform {
    fn metadata(&self, path: &Path) -> Result<FileEntry> {
        scan::metadata::metadata(path)
    }
    fn read_dir(&self, dir: &Path) -> Result<Vec<DirItem>> {
        scan::walk::read_dir(dir)
    }
    fn volumes(&self) -> Result<Vec<VolumeInfo>> {
        scan::volumes::volumes()
    }
    fn volume_of(&self, path: &Path) -> Result<VolumeInfo> {
        scan::volumes::volume_of(path)
    }
    fn fast_enumerate(&self, root: &Path, sink: &mut dyn FnMut(FileEntry), cancel: &CancelToken) -> Result<()> {
        scan::mft::fast_enumerate(root, sink, cancel)
    }
    fn cloud_info(&self, path: &Path) -> Result<CloudInfo> {
        scan::cloud::cloud_info(path)
    }
}

impl ActionPlatform for NativePlatform {
    fn trash(&self, path: &Path) -> Result<TrashTicket> {
        action::trash::trash(path)
    }
    fn restore_trashed(&self, ticket: &TrashTicket, original: &Path) -> Result<()> {
        action::trash::restore(ticket, original)
    }
    fn hardlink(&self, existing: &Path, new_link: &Path) -> Result<()> {
        action::links::hardlink(existing, new_link)
    }
    fn reflink(&self, src: &Path, dst: &Path) -> Result<()> {
        action::links::reflink(src, dst)
    }
    fn dehydrate(&self, path: &Path) -> Result<()> {
        action::cloud::dehydrate(path)
    }
    fn hydrate(&self, path: &Path) -> Result<()> {
        action::cloud::hydrate(path)
    }
    fn sync_roots(&self) -> Result<Vec<SyncRoot>> {
        action::cloud::sync_roots()
    }
    fn is_elevated(&self) -> bool {
        action::elevation::is_elevated()
    }
    fn fs_compress(&self, path: &Path, algo: FsCompressAlgo) -> Result<()> {
        action::fs_compress::compress(path, algo)
    }
    fn fs_decompress(&self, path: &Path) -> Result<()> {
        action::fs_compress::decompress(path)
    }
    fn fs_compression(&self, path: &Path) -> Result<Option<FsCompressAlgo>> {
        action::fs_compress::state(path)
    }
}
