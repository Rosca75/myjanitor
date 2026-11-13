//! # mj-scan
//!
//! Scan engine (W2-A): walk the scope roots (NTFS MFT fast path on Windows,
//! directory walk elsewhere), build the [`FileIndex`], and persist content
//! fingerprints in the [`ScanCache`] for near-instant rescans (SCAN-01..11).
//!
//! Wave 1 ships a correct **sequential** walker so the rest of the workspace
//! can be tested end to end; W2-A replaces `walker` with the parallel version
//! and implements the cache file format in `cache::format`.
#![forbid(unsafe_code)]

pub mod cache;
pub mod walker;

use std::path::PathBuf;

use mj_core::{CancelToken, FileIndex, ProgressSink, Result, ScanScope};
use mj_platform::ScanPlatform;

pub use cache::ScanCache;

/// Everything one scan needs.
pub struct ScanRequest<'a> {
    pub scope: &'a ScanScope,
    /// Global exclusions from settings (globs or absolute paths), added to `scope.exclude`.
    pub exclusions: &'a [String],
    pub platform: &'a dyn ScanPlatform,
    pub cancel: &'a CancelToken,
    /// Receives `Phase::Walking` progress (`done` = entries so far, `total` unknown).
    pub progress: &'a dyn ProgressSink,
}

/// Scan the scope. Errors only for fatal conditions (no root readable, cancelled);
/// unreadable entries are counted in `ScanStats`.
pub fn scan(req: &ScanRequest<'_>) -> Result<FileIndex> {
    walker::scan(req)
}

/// Roots of a scope after lexical normalisation and removal of roots nested in other roots.
pub fn effective_roots(scope: &ScanScope) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = scope.roots.iter().map(|r| mj_core::pathutil::normalize_lexical(r)).collect();
    roots.sort_by_key(|r| mj_core::pathutil::path_key(r));
    roots.dedup_by_key(|r| mj_core::pathutil::path_key(r));
    let all = roots.clone();
    roots.retain(|r| !all.iter().any(|o| o != r && mj_core::pathutil::is_within(o, r)));
    roots
}
