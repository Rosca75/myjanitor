//! The detector contract: everything that finds files to clean implements [`Detector`].

use serde::{Deserialize, Serialize};

use crate::action::ActionKind;
use crate::cancel::CancelToken;
use crate::candidate::DetectorOutput;
use crate::error::Result;
use crate::fingerprint::FingerprintStore;
use crate::index::FileIndex;
use crate::opportunity::Category;
use crate::params::{ParamValues, SettingSpec};
use crate::progress::ProgressSink;
use crate::safety::SafetyClass;
use crate::time::Timestamp;

/// Every detector, with its stable wire id. Adding one is a contract change (orchestrator).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorId {
    /// CLN-DET-01 (W2-B).
    ExactDuplicates,
    /// CLN-DET-02 (W2-B).
    SimilarImages,
    /// CLN-DET-03 (W3-C).
    SimilarVideos,
    /// CLN-DET-04 (W3-C).
    SameMusic,
    /// CLN-DET-05 (W3-C).
    DuplicateRecordings,
    /// CLN-DET-06 (W2-D).
    EmptyFiles,
    /// CLN-DET-06 (W2-D), nested empty trees reported topmost.
    EmptyFolders,
    /// CLN-DET-07 (W2-D).
    ZeroedFiles,
    /// CLN-DET-08 (W2-D).
    BrokenFiles,
    /// CLN-DET-09 (W2-D).
    BrokenLinks,
    /// CLN-DET-10 (W2-D).
    TempFiles,
    /// CLN-DET-11 (W2-D).
    AppCaches,
    /// CLN-DET-12 (W2-D).
    OfficeLeftovers,
    /// CLN-DET-13 (W2-D).
    DevArtefacts,
    /// CLN-DET-14 (W2-D).
    OldInstallers,
    /// CLN-DET-15 (W2-D).
    OrphanSidecars,
    /// CLN-DET-15 (W2-D).
    RawJpegPairs,
    /// CLN-DET-16 (W2-D).
    LanguagePacks,
    /// CLN-DET-17 (W2-D).
    StaleFiles,
    /// CLN-DET-18 (W2-B).
    VersionChains,
    /// CLN-DET-19 (W2-B).
    DerivedFiles,
    /// CLN-DET-20 (W2-B).
    AttachmentCopies,
    /// CLN-DET-21 (W2-D).
    InactiveFolders,
    /// OPT-FNT-01 (W2-B).
    DuplicateFonts,
    /// OPT-FNT-02 (W2-D), review only.
    UnusedFonts,
}

/// Platforms a feature works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformSupport {
    Both,
    WindowsOnly,
    LinuxOnly,
}

impl PlatformSupport {
    /// True on the platform this binary was built for.
    pub fn here(self) -> bool {
        match self {
            PlatformSupport::Both => true,
            PlatformSupport::WindowsOnly => cfg!(windows),
            PlatformSupport::LinuxOnly => cfg!(target_os = "linux"),
        }
    }
}

/// Static description of a detector (drives the opportunity card).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectorInfo {
    pub id: DetectorId,
    /// Card title ("Exact duplicates").
    pub title: String,
    /// One-line description template ("Files with identical content").
    pub description: String,
    pub category: Category,
    pub safety: SafetyClass,
    pub default_action: ActionKind,
    pub available_actions: Vec<ActionKind>,
    /// Detector settings (age, keep rule, threshold...).
    pub settings: Vec<SettingSpec>,
    pub platform: PlatformSupport,
    /// Spec feature ids covered (`["CLN-DET-01"]`).
    pub feature_ids: Vec<String>,
    /// Unit noun for the card count ("files", "folders", "apps").
    pub unit: String,
}

/// Everything a detector may read. Detectors are pure with respect to this context:
/// they never modify files.
pub struct DetectContext<'a> {
    pub index: &'a FileIndex,
    /// Values for `DetectorInfo::settings` (missing keys = defaults).
    pub params: &'a ParamValues,
    /// "Now" for age computations (fixed per scan so results are reproducible).
    pub now: Timestamp,
    /// Hash/fingerprint cache (shared with the scan cache).
    pub fingerprints: &'a dyn FingerprintStore,
    pub cancel: &'a CancelToken,
    pub progress: &'a dyn ProgressSink,
    /// Worker threads to use (0 = logical CPUs).
    pub threads: usize,
}

/// A detector. Implementations live in `mj-dupes`, `mj-detect` and `mj-optimise`.
pub trait Detector: Send + Sync {
    /// Static description.
    fn info(&self) -> DetectorInfo;

    /// Analyse the index. Must call `ctx.cancel.checkpoint()` regularly and report
    /// every unreadable file through `DetectorOutput::diagnostics`.
    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput>;
}
