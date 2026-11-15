//! Read-side platform code. **Owner: W2-A.** Each file exposes the free
//! functions `native.rs` delegates to; keep those signatures.
//!
//! - `metadata`: lstat semantics (Linux `statx`, Windows Win32 attributes).
//! - `walk`: directory listing (Linux `getdents` via std, Windows `FindFirstFileExW`).
//! - `mft`: NTFS MFT fast path (`FSCTL_ENUM_USN_DATA`), with the pure parsers in
//!   `usn` (USN records) and `ntfs` (FILE records), tested on every platform.
//! - `volumes`: mounted volumes and free space.
//! - `cloud`: cloud placeholder state.
//!
//! Parsers that only the Windows code calls are compiled everywhere so they
//! stay unit-tested on Linux.

pub mod cloud;
pub mod metadata;
pub mod mft;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod ntfs;
pub mod statx;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod usn;
pub mod volumes;
pub mod walk;
#[cfg(windows)]
pub mod win;
