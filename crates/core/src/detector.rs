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

/// Environment facts the engine supplies to every detector and optimiser analysis
/// (Wave 3 contract revision). They travel inside [`DetectContext::params`] under
/// reserved keys so `DetectContext` keeps its shape; read them with
/// [`DetectContext::env_paths`] and [`DetectContext::is_protected`], never by
/// guessing folder names when the engine supplied them.
///
/// Encoding: a `ParamValue::Text` holding entries separated by NUL (`\0` never occurs
/// in a path), each path in the lossless codec of [`crate::pathutil::encode_path`].
/// A missing key means *unknown* (the engine could not detect it), which is not the
/// same as an empty list (detected, none).
pub mod env {
    /// Detected sync roots (OneDrive, SharePoint libraries, Dropbox, Nextcloud...).
    pub const SYNC_ROOTS: &str = "env.sync_roots";
    /// The user's Downloads folders.
    pub const DOWNLOADS: &str = "env.downloads";
    /// System and user font folders.
    pub const FONT_FOLDERS: &str = "env.font_folders";
    /// Protected paths (built-in and user) and built-in protected glob patterns.
    /// An absolute entry protects that path and everything below it; any other
    /// entry is a path-mode glob matched against the whole path.
    pub const PROTECTED: &str = "env.protected";

    /// Encode a list for one of the keys above: entries joined with NUL (empty entries are
    /// left out).
    pub fn encode(paths: &[impl AsRef<str>]) -> String {
        paths.iter().map(|p| p.as_ref()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\0")
    }

    /// Encode paths with the lossless codec.
    pub fn encode_paths(paths: &[std::path::PathBuf]) -> String {
        let v: Vec<String> = paths.iter().map(|p| crate::pathutil::encode_path(p)).collect();
        encode(&v)
    }

    /// Decode one key's value. NUL occurs in an entry only as the first character of an
    /// escaped (non-Unicode) path from [`encode_paths`], and no entry is empty, so an empty
    /// piece between separators is that marker: it is given back to the next entry (RF-5;
    /// it used to be dropped, which turned the path into a literal `%XX` string).
    pub fn decode(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut escaped = false;
        for piece in text.split('\0') {
            if piece.is_empty() {
                escaped = true;
            } else if std::mem::take(&mut escaped) {
                out.push(format!("{}{piece}", crate::pathutil::ESCAPED_PREFIX));
            } else {
                out.push(piece.to_string());
            }
        }
        out
    }
}

impl DetectContext<'_> {
    /// Paths the engine supplied under an [`env`] key; `None` when unknown.
    pub fn env_paths(&self, key: &str) -> Option<Vec<std::path::PathBuf>> {
        match self.params.0.get(key) {
            Some(crate::params::ParamValue::Text(t)) => {
                Some(env::decode(t).iter().filter_map(|s| crate::pathutil::decode_path(s).ok()).collect())
            }
            _ => None,
        }
    }

    /// True when `path` is protected according to [`env::PROTECTED`] (false when the engine
    /// supplied no list; the engine's own safety inspection still badges protected rows).
    pub fn is_protected(&self, path: &std::path::Path) -> bool {
        let Some(crate::params::ParamValue::Text(t)) = self.params.0.get(env::PROTECTED) else {
            return false;
        };
        env::decode(t).iter().any(|entry| match crate::pathutil::decode_path(entry) {
            // RF-2: the shared classifier; an absolute entry is a folder even with `[` in it.
            Ok(p) if p.is_absolute() && !crate::glob::is_pattern_entry(entry) => crate::pathutil::is_within(&p, path),
            _ => crate::glob::entry_glob(entry).matches_path(path),
        })
    }
}

/// A detector. Implementations live in `mj-dupes`, `mj-detect` and `mj-optimise`.
pub trait Detector: Send + Sync {
    /// Static description.
    fn info(&self) -> DetectorInfo;

    /// Analyse the index. Must call `ctx.cancel.checkpoint()` regularly and report
    /// every unreadable file through `DetectorOutput::diagnostics`.
    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::NoFingerprintStore;
    use crate::params::ParamValue;
    use crate::progress::NullProgress;
    #[test]
    fn env_keys_round_trip_and_protect() {
        let index = FileIndex::new(Vec::new(), Vec::new(), Default::default(), Timestamp(0));
        let mut params = ParamValues::new();
        let abs = crate::testutil::abs;
        let roots = vec![abs("/home/me/OneDrive - Acme"), abs("/home/me/line\nbreak")];
        params.set(env::SYNC_ROOTS, ParamValue::Text(env::encode_paths(&roots)));
        let etc = crate::pathutil::encode_path(&abs("/etc"));
        params.set(env::PROTECTED, ParamValue::Text(env::encode(&[etc.as_str(), "**/.ssh/**"])));
        let cancel = CancelToken::new();
        let ctx = DetectContext {
            index: &index,
            params: &params,
            now: Timestamp(0),
            fingerprints: &NoFingerprintStore,
            cancel: &cancel,
            progress: &NullProgress,
            threads: 1,
        };
        assert_eq!(ctx.env_paths(env::SYNC_ROOTS), Some(roots));
        assert_eq!(ctx.env_paths(env::DOWNLOADS), None, "missing = unknown");
        assert!(ctx.is_protected(&abs("/etc/passwd")));
        assert!(ctx.is_protected(&abs("/home/me/.ssh/id_rsa")));
        assert!(!ctx.is_protected(&abs("/etcetera/x")));
        assert!(!ctx.is_protected(&abs("/home/me/doc.txt")));
    }
}
