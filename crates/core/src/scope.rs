//! What to scan (SCAN-01, SCAN-02) and how.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The scan scope: one or more roots plus include/exclude patterns.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScanScope {
    /// Absolute root folders or drives (`C:\`, `/home/me`). Order is kept; `FileEntry::root` indexes it.
    #[serde(with = "crate::pathutil::serde_path::vec")]
    pub roots: Vec<PathBuf>,
    /// Glob patterns (path mode, see `glob`); when non-empty only matching *files* are indexed.
    /// Directories are always walked so includes can match below them.
    #[serde(default)]
    pub include: Vec<String>,
    /// Glob patterns or absolute paths excluded from the walk (a matching directory is not entered).
    /// Global exclusions from `Settings::exclusions` are added by the engine.
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub options: ScanOptions,
}

/// Scan options. Symlinks and junctions are **never** followed; that is not an option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanOptions {
    /// Index hidden files and enter hidden folders (`.cache`, `AppData`). Default true:
    /// most junk lives there.
    pub include_hidden: bool,
    /// Use the NTFS MFT fast path when available (Windows, needs volume access), SCAN-03.
    pub use_fast_index: bool,
    /// Use and update the scan cache, SCAN-04.
    pub use_cache: bool,
    /// Do not cross into other mounted filesystems below a root.
    pub one_file_system: bool,
    /// Maximum depth below each root (`None` = unlimited).
    pub max_depth: Option<u32>,
    /// Worker threads for stat/hash (0 = logical CPUs), AUT-05.
    pub threads: u16,
    /// Collect the file owner (costs a lookup per file on Windows), CLN-FLT-06.
    pub collect_owner: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            include_hidden: true,
            use_fast_index: true,
            use_cache: true,
            one_file_system: true,
            max_depth: None,
            threads: 0,
            collect_owner: false,
        }
    }
}
