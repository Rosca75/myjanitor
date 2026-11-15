//! Transparent filesystem compression. **Owner: W3-B.**
//! Windows: WOF (`FSCTL_SET_EXTERNAL_BACKING` with `WOF_PROVIDER_FILE`,
//! XPRESS4K/8K/16K, LZX; revert with `FSCTL_DELETE_EXTERNAL_BACKING`).
//! Linux: Btrfs compression (the `btrfs.compression` property) followed by a
//! rewrite of the extents (`BTRFS_IOC_DEFRAG_RANGE`); elsewhere `Unsupported`.
//!
//! Besides the three trait-backed calls, this module offers [`support`]
//! (per-path volume check with a one-line reason), [`any_volume_supported`]
//! (for the optimiser's availability) and [`on_disk_size`].

#[cfg(target_os = "linux")]
pub mod btrfs;
#[cfg(windows)]
mod wof;
pub mod wof_layout;

use std::path::Path;

use mj_core::{FsCompressAlgo, Result};

/// Compress one file in place (content unchanged).
pub fn compress(path: &Path, algo: FsCompressAlgo) -> Result<()> {
    #[cfg(windows)]
    return wof::compress(path, algo);
    #[cfg(target_os = "linux")]
    return btrfs::compress(path, algo);
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (path, algo);
        Err(mj_core::Error::unsupported("Filesystem compression", "not available on this platform"))
    }
}

/// Remove compression.
pub fn decompress(path: &Path) -> Result<()> {
    #[cfg(windows)]
    return wof::decompress(path);
    #[cfg(target_os = "linux")]
    return btrfs::decompress(path);
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = path;
        Err(mj_core::Error::unsupported("Filesystem compression", "not available on this platform"))
    }
}

/// Current compression, `None` when uncompressed.
pub fn state(path: &Path) -> Result<Option<FsCompressAlgo>> {
    #[cfg(windows)]
    return wof::state(path);
    #[cfg(target_os = "linux")]
    return btrfs::state(path);
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = path;
        Ok(None)
    }
}

/// `Ok` when the volume holding `path` supports transparent compression,
/// else a one-line reason ("Needs a Btrfs volume", "Needs an NTFS volume").
pub fn support(path: &Path) -> std::result::Result<(), String> {
    #[cfg(windows)]
    return wof::support(path);
    #[cfg(target_os = "linux")]
    return btrfs::support(path);
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = path;
        Err("Windows (NTFS) or Linux (Btrfs) only".into())
    }
}

/// Whether any volume here can be compressed: always on Windows (NTFS is the
/// system volume), on Linux only when a Btrfs filesystem is mounted.
pub fn any_volume_supported() -> std::result::Result<(), String> {
    #[cfg(windows)]
    return Ok(());
    #[cfg(target_os = "linux")]
    return if btrfs::any_btrfs_mounted() { Ok(()) } else { Err("Needs a Btrfs volume".into()) };
    #[cfg(not(any(windows, target_os = "linux")))]
    Err("Windows (NTFS) or Linux (Btrfs) only".into())
}

/// Bytes the file occupies on disk (compressed size where compressed).
pub fn on_disk_size(path: &Path) -> Result<u64> {
    #[cfg(windows)]
    return wof::on_disk_size(path);
    #[cfg(target_os = "linux")]
    return btrfs::on_disk_size(path);
    #[cfg(not(any(windows, target_os = "linux")))]
    std::fs::metadata(path).map(|m| m.len()).map_err(|e| mj_core::Error::io(path, e))
}
