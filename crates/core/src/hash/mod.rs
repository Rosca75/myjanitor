//! In-house hashes, verified against official test vectors.
//!
//! - [`xxh64`]: xxHash64 (non-cryptographic), used for staged duplicate
//!   detection (partial and full hashes) and the cache trailer.
//! - [`sha256`]: SHA-256 (FIPS 180-4), used for the audit log and for the
//!   final confirmation of duplicate groups before a delete is offered.

pub mod sha256;
pub mod xxh64;

pub use sha256::{sha256, Sha256};
pub use xxh64::{xxh64, Xxh64};

/// Lower-case hex of a byte slice.
pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}
