//! # mj-platform
//!
//! The platform abstraction: two traits, [`ScanPlatform`] (read side) and
//! [`ActionPlatform`] (write side), implemented by [`NativePlatform`] for the
//! OS this binary targets. All OS calls are hand-written FFI (no `libc`,
//! `nix`, `windows`, `windows-sys`).
//!
//! Module ownership (see `docs/ARCHITECTURE.md` §11):
//! - `traits`, `native`, `ffi` (common declarations): Wave 1, frozen.
//! - `scan::*`: W2-A (statx, walk, MFT, volumes, allocated size, cloud attributes).
//! - `action::*`: W2-E (trash, links, dehydrate, sync roots, elevation).
//! - `action::fs_compress::*`: W3-B (WOF, Btrfs compression).
//!
//! Advanced calls return `Error::Unsupported` (not available here) or
//! `Error::NotImplemented` (owner has not landed it yet).

pub mod action;
pub mod ffi;
pub mod native;
pub mod scan;
pub mod traits;

pub use native::NativePlatform;
pub use traits::{
    ActionPlatform, CloudFileState, CloudInfo, DirItem, Platform, ScanPlatform, SyncProvider, SyncRoot, VolumeFeatures,
    VolumeInfo,
};
