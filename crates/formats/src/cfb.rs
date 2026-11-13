//! OLE compound files (MS-CFB): detect encrypted OOXML (`EncryptionInfo` /
//! `EncryptedPackage` streams) and legacy-Office password protection (SAF-07).
//! Owner: W2-E.

use mj_core::{Error, Result};

/// Signature of a compound file.
pub const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Names of the root-storage streams of a compound file.
pub fn stream_names(data: &[u8]) -> Result<Vec<String>> {
    let _ = data;
    Err(Error::not_implemented("CFB directory (W2-E)"))
}
