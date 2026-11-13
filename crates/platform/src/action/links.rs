//! Hard links (`CreateHardLinkW` / `link`) and reflinks (`FICLONE` ioctl on
//! Btrfs/XFS, `FSCTL_DUPLICATE_EXTENTS_TO_FILE` on ReFS). Owner: W2-E.

use std::path::Path;

use mj_core::{Error, Result};

/// Create `new_link` as a hard link to `existing` (std fallback; W2-E may replace).
pub fn hardlink(existing: &Path, new_link: &Path) -> Result<()> {
    std::fs::hard_link(existing, new_link).map_err(|e| Error::io(new_link, e))
}

/// Create `dst` as a copy-on-write clone of `src`.
pub fn reflink(src: &Path, dst: &Path) -> Result<()> {
    let _ = (src, dst);
    Err(Error::not_implemented("reflink (W2-E)"))
}
