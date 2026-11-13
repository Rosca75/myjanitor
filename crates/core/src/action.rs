//! Actions: what a plan step does to its selected candidates.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::candidate::TargetKind;
use crate::entry::FileStamp;
use crate::optimiser::OptimiserId;
use crate::params::ParamValues;
use crate::safety::SafetyClass;

/// What happens to the original when an optimisation replaces it (OPT-ENG-06).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum OriginalsPolicy {
    /// Original goes to the Recycle Bin / Trash (default; restorable).
    #[default]
    RecycleBin,
    /// Original is moved into MyJanitor's quarantine (restorable, auto-purged).
    Quarantine,
    /// Original is moved under this folder, keeping its path structure.
    BackupFolder {
        #[serde(with = "crate::pathutil::serde_path")]
        folder: PathBuf,
    },
    /// Original is deleted permanently (irreversible; needs confirmation).
    Delete,
}

impl OriginalsPolicy {
    /// True when the original can be restored.
    pub fn is_reversible(&self) -> bool {
        !matches!(self, OriginalsPolicy::Delete)
    }
}

/// Where an optimised result goes (OPT-ENG-07).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum OutputMode {
    /// The result replaces the original (original handled by `OriginalsPolicy`).
    #[default]
    Replace,
    /// The result is written beside the original as `<stem><suffix>.<ext>`; nothing is removed.
    LightCopy { suffix: String },
}

/// Archive container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    /// In-house ZIP (deflate via miniz_oxide).
    Zip,
    /// External `7z`.
    SevenZip,
    /// External `zstd` over a tar stream.
    TarZstd,
}

/// Transparent filesystem compression algorithm (OPT-FSC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsCompressAlgo {
    /// Windows WOF.
    Xpress4k,
    Xpress8k,
    Xpress16k,
    Lzx,
    /// Btrfs compression property.
    BtrfsZstd,
}

/// An action. Tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// To the Recycle Bin / freedesktop Trash (CLN-ACT-02).
    Trash,
    /// Into MyJanitor's quarantine, auto-purged after N days (CLN-ACT-04).
    Quarantine { purge_after_days: u32 },
    /// Delete permanently (CLN-ACT-03; needs confirmation).
    PermanentDelete,
    /// Move under `dest`, keeping the structure below the scan root when `keep_structure` (CLN-ACT-05).
    Move {
        #[serde(with = "crate::pathutil::serde_path")]
        dest: PathBuf,
        keep_structure: bool,
    },
    /// Copy under `dest` (nothing is removed).
    Copy {
        #[serde(with = "crate::pathutil::serde_path")]
        dest: PathBuf,
        keep_structure: bool,
    },
    /// Replace a duplicate with a hard link to its group's keeper (CLN-ACT-06).
    Hardlink,
    /// Replace a duplicate with a reflink (copy-on-write clone) of its group's keeper (CLN-ACT-06).
    Reflink,
    /// Cloud placeholder: free up space, keep online (CLN-ACT-07; Windows).
    Dehydrate,
    /// Run an optimiser (OPT-*).
    Optimise {
        optimiser: OptimiserId,
        #[serde(default)]
        params: ParamValues,
        #[serde(default)]
        output: OutputMode,
        #[serde(default)]
        originals: OriginalsPolicy,
        /// Keep the original if the output is not smaller (OPT-ENG-04). Always true for now.
        #[serde(default = "yes")]
        keep_if_not_smaller: bool,
    },
    /// Archive folders/files into one archive, then remove the sources via `originals` (OPT-ARC).
    Archive {
        format: ArchiveFormat,
        level: u8,
        /// Folder the archive is written to (default: beside the source).
        #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
        dest: Option<PathBuf>,
        /// What happens to the sources after a verified archive (OPT-ARC-02 test is mandatory).
        #[serde(default)]
        originals: OriginalsPolicy,
        /// Keep the sources (archive only).
        #[serde(default)]
        keep_sources: bool,
    },
    /// Apply transparent filesystem compression (OPT-FSC-01/06).
    FsCompress { algorithm: FsCompressAlgo },
    /// Revert transparent filesystem compression (OPT-FSC-05).
    FsDecompress,
    /// No automatic action (review-only opportunities).
    None,
}

fn yes() -> bool {
    true
}

/// Discriminant of [`Action`] (lists, journal, reports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Trash,
    Quarantine,
    PermanentDelete,
    Move,
    Copy,
    Hardlink,
    Reflink,
    Dehydrate,
    Optimise,
    Archive,
    FsCompress,
    FsDecompress,
    None,
}

impl Action {
    /// Discriminant.
    pub fn kind(&self) -> ActionKind {
        match self {
            Action::Trash => ActionKind::Trash,
            Action::Quarantine { .. } => ActionKind::Quarantine,
            Action::PermanentDelete => ActionKind::PermanentDelete,
            Action::Move { .. } => ActionKind::Move,
            Action::Copy { .. } => ActionKind::Copy,
            Action::Hardlink => ActionKind::Hardlink,
            Action::Reflink => ActionKind::Reflink,
            Action::Dehydrate => ActionKind::Dehydrate,
            Action::Optimise { .. } => ActionKind::Optimise,
            Action::Archive { .. } => ActionKind::Archive,
            Action::FsCompress { .. } => ActionKind::FsCompress,
            Action::FsDecompress => ActionKind::FsDecompress,
            Action::None => ActionKind::None,
        }
    }

    /// Safety class of this action. `lossy` says whether the optimiser parameters are lossy.
    pub fn safety(&self, lossy: bool) -> SafetyClass {
        match self {
            Action::Trash | Action::Quarantine { .. } | Action::Move { .. } => SafetyClass::Reversible,
            Action::Copy { .. } | Action::Dehydrate | Action::FsCompress { .. } | Action::FsDecompress => {
                SafetyClass::Reversible
            }
            Action::Hardlink | Action::Reflink => SafetyClass::Reversible,
            Action::PermanentDelete => SafetyClass::Permanent,
            Action::Optimise { output: OutputMode::LightCopy { .. }, .. } => SafetyClass::LightCopy,
            Action::Optimise { originals, .. } | Action::Archive { originals, .. } => {
                if lossy {
                    SafetyClass::LossyOptIn
                } else if originals.is_reversible() {
                    SafetyClass::Reversible
                } else {
                    SafetyClass::Permanent
                }
            }
            Action::None => SafetyClass::ReviewOnly,
        }
    }
}

/// One concrete unit of work for the action engine, expanded from a plan step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionItem {
    #[serde(with = "crate::pathutil::serde_path")]
    pub path: PathBuf,
    pub target: TargetKind,
    pub action: Action,
    /// For `Hardlink`/`Reflink`: the group keeper to link to.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub link_source: Option<PathBuf>,
    /// Scan root the path lies under (for `keep_structure`).
    #[serde(with = "crate::pathutil::serde_path")]
    pub root: PathBuf,
    /// Stamp at scan time. The engine re-stats right before acting and skips with
    /// `Error::Changed` on mismatch (TOCTOU guard). Folders use size 0 / mtime 0 and are not compared.
    pub expected: FileStamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_json_is_tagged() {
        let a = Action::Optimise {
            optimiser: OptimiserId::Audio,
            params: ParamValues::default(),
            output: OutputMode::Replace,
            originals: OriginalsPolicy::RecycleBin,
            keep_if_not_smaller: true,
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains(r#""kind":"optimise""#), "{json}");
        assert!(json.contains(r#""originals":{"policy":"recycle_bin"}"#), "{json}");
        let back: Action = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
        assert_eq!(serde_json::to_string(&Action::Trash).unwrap(), r#"{"kind":"trash"}"#);
        assert_eq!(a.safety(true), SafetyClass::LossyOptIn);
        assert_eq!(Action::PermanentDelete.safety(false), SafetyClass::Permanent);
    }
}
