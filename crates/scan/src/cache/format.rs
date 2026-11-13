//! On-disk cache format (W2-A), little-endian, versioned, checksummed:
//!
//! ```text
//! header:  magic "MJCACHE\0" (8) | u32 format_version | u32 algo_version
//!          (= mj_core::fingerprint::FINGERPRINT_ALGO_VERSION) | u64 entry_count
//! record:  u32 key_len | key bytes (path codec string, UTF-8)
//!          u64 size | i64 mtime_ns | u16 presence bits
//!          then present fields in fixed order:
//!          partial u64, full u64, sha256 [32], dhash u64, phash u64,
//!          width u32, height u32, exif (u8 flags + i64 date + 2 x (u16 len + bytes)),
//!          flags u32
//! trailer: u64 xxh64 (seed 0) of every byte before it
//! ```
//!
//! Bad magic, other versions, short reads or a trailer mismatch -> empty cache.

/// File magic.
pub const MAGIC: &[u8; 8] = b"MJCACHE\0";
/// Container format version (bump on layout change).
pub const FORMAT_VERSION: u32 = 1;
