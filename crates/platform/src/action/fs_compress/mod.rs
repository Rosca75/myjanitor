//! Transparent filesystem compression. **Owner: W3-B.**
//! Windows: WOF (`FSCTL_SET_EXTERNAL_BACKING` with `WOF_PROVIDER_FILE`,
//! XPRESS4K/8K/16K, LZX; revert with `FSCTL_DELETE_EXTERNAL_BACKING`).
//! Linux: Btrfs compression (`FS_IOC_SETFLAGS` `FS_COMPR_FL` / the
//! `btrfs.compression` xattr) followed by a rewrite; elsewhere `Unsupported`.

#[cfg(target_os = "linux")]
mod btrfs;
#[cfg(windows)]
mod wof;

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
