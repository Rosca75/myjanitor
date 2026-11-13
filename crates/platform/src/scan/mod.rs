//! Read-side platform code. **Owner: W2-A.** Each file exposes the free
//! functions `native.rs` delegates to; keep those signatures.
//!
//! Wave 1 ships std-only fallbacks for `metadata` and `read_dir` so every crate
//! can be tested; W2-A replaces them with `statx` (Linux) and native Win32 calls,
//! and fills in volumes, MFT enumeration and cloud attributes.

pub mod cloud;
pub mod metadata;
pub mod mft;
pub mod volumes;
pub mod walk;
