//! Volumes / drives with free space (drive bars, `drives_list`). Owner: W2-A
//! (Linux: `/proc/self/mounts` + `statvfs`; Windows: `GetLogicalDriveStringsW`,
//! `GetDiskFreeSpaceExW`, `GetVolumeInformationW`).

use std::path::Path;

use mj_core::{Error, Result};

use crate::traits::VolumeInfo;

/// All volumes worth showing.
pub fn volumes() -> Result<Vec<VolumeInfo>> {
    Err(Error::not_implemented("volume listing (W2-A)"))
}

/// Volume holding `path`.
pub fn volume_of(path: &Path) -> Result<VolumeInfo> {
    let _ = path;
    Err(Error::not_implemented("volume lookup (W2-A)"))
}
