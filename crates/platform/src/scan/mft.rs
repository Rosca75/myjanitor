//! NTFS MFT enumeration (`FSCTL_ENUM_USN_DATA`), SCAN-03. Owner: W2-A.
//! Windows only; needs a volume handle (admin). Elsewhere: `Unsupported`.

use std::path::Path;

use mj_core::{CancelToken, Error, FileEntry, Result};

/// Enumerate everything under `root` from the MFT.
pub fn fast_enumerate(root: &Path, sink: &mut dyn FnMut(FileEntry), cancel: &CancelToken) -> Result<()> {
    let _ = (root, sink, cancel);
    if cfg!(windows) {
        Err(Error::not_implemented("MFT enumeration (W2-A)"))
    } else {
        Err(Error::unsupported("Fast indexing", "NTFS MFT enumeration is Windows only"))
    }
}
