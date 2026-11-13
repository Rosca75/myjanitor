//! # mj-scan
//!
//! Scan engine (W2-A): walk the scope roots (NTFS MFT fast path on Windows,
//! parallel directory walk everywhere), build the [`FileIndex`], aggregate it
//! into a [`FolderTree`] (folder sizes, breakdowns), and persist content
//! fingerprints in the [`ScanCache`] for near-instant rescans (SCAN-01..11).
//!
//! Entry points:
//! - [`scan`]: walk only (the frozen Wave 1 API);
//! - [`scan_with_cache`]: walk, then record every file in the cache and fill
//!   `ScanStats::cache_hits` / `cache_misses`; after the analyses, the caller
//!   runs [`ScanCache::prune`] and [`ScanCache::save`] (cache failures are
//!   never fatal);
//! - [`effective_roots`]: the scope roots after normalisation and nesting.
#![forbid(unsafe_code)]

pub mod cache;
pub mod tree;
pub mod walker;

use std::path::PathBuf;

use mj_core::{CancelToken, FileIndex, ProgressSink, Result, ScanScope, Timestamp};
use mj_platform::ScanPlatform;

pub use cache::ScanCache;
pub use tree::{BreakdownRow, FolderTree, TreeTotals};

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
    walker::run(req).map(|w| w.index)
}

/// [`scan`], then record what the scan saw in `cache` (when
/// `scope.options.use_cache`): fills `cache_hits` / `cache_misses` and
/// remembers the coverage [`ScanCache::prune`] needs for this index.
pub fn scan_with_cache(req: &ScanRequest<'_>, cache: &ScanCache) -> Result<FileIndex> {
    let started = Timestamp::now();
    let walk = walker::run(req)?;
    if !req.scope.options.use_cache {
        return Ok(walk.index);
    }
    req.cancel.checkpoint()?;
    let workers = match req.scope.options.threads {
        0 => mj_core::pool::default_workers(),
        n => usize::from(n),
    };
    let (hits, misses) = cache.observe(&walk, started, workers);
    let mut index = walk.index;
    index.stats.cache_hits = hits;
    index.stats.cache_misses = misses;
    Ok(index)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_roots_drop_nested() {
        let scope =
            ScanScope { roots: vec!["/a/b".into(), "/a".into(), "/c".into(), "/a/".into()], ..Default::default() };
        assert_eq!(effective_roots(&scope), vec![PathBuf::from("/a"), "/c".into()]);
        let scope = ScanScope { roots: vec!["/a/./b/../b".into(), "/a/b".into(), "/ab".into()], ..Default::default() };
        assert_eq!(effective_roots(&scope), vec![PathBuf::from("/a/b"), "/ab".into()]);
    }
}
