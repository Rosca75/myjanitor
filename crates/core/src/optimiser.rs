//! The optimiser contract (W3-A framework). An optimiser is a **pure transform**:
//! it reads `src` and writes a new file at `dst`. It never deletes, renames or
//! overwrites anything; replacing the original, keep-if-not-smaller, light
//! copies, the originals policy and the journal are the action engine's job.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::action::{ActionKind, ArchiveFormat};
use crate::cancel::CancelToken;
use crate::candidate::DetectorOutput;
use crate::detector::{DetectContext, PlatformSupport};
use crate::entry::FileEntry;
use crate::error::Result;
use crate::estimate::Estimate;
use crate::opportunity::{Availability, Category};
use crate::params::{ParamValues, SettingSpec, StepPreset};
use crate::progress::ProgressSink;
use crate::safety::SafetyClass;
use crate::tools::ExternalTool;

/// Every optimiser, with its stable wire id. Adding one is a contract change (orchestrator).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimiserId {
    /// OPT-IMG-01 PNG lossless (in-house deflate re-encode; oxipng when present). W3-C.
    PngLossless,
    /// OPT-IMG-01 JPEG lossless (metadata/Huffman; jpegtran when present). W3-C.
    JpegLossless,
    /// OPT-IMG-02/03/04/05 lossy recompression, conversion, resize (external encoders). W3-C.
    ImageLossy,
    /// OPT-PPT-* PowerPoint. W3-A.
    PowerPoint,
    /// OPT-XLS-* Excel. W3-A.
    Excel,
    /// OPT-DOC-* Word. W3-A.
    Word,
    /// OPT-OFC-01/03 generic ZIP-based repack (EPUB, CBZ, JAR, ODF). W3-A.
    ZipRepack,
    /// OPT-PDF-03/05 lossless structural PDF pass. W3-B.
    PdfLossless,
    /// OPT-PDF-01/02/04 PDF image downsampling and presets. W3-B.
    PdfLossy,
    /// OPT-VID-* video re-encode via ffmpeg. W3-C.
    Video,
    /// OPT-SCR-* screen recordings via ffmpeg. W3-C.
    ScreenRecordings,
    /// OPT-AUD-* meeting recordings and audio via ffmpeg. W3-C.
    Audio,
    /// OPT-ARC-01/04 archive cold folders. W3-B.
    ArchiveFolders,
    /// OPT-ARC-03 recompress existing archives. W3-B.
    ArchiveRecompress,
    /// OPT-FSC-01..06 transparent filesystem compression (WOF / Btrfs). W3-B.
    FsCompression,
}

/// Static description of an optimiser (drives the opportunity card and step panel).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimiserInfo {
    pub id: OptimiserId,
    pub title: String,
    pub description: String,
    pub category: Category,
    /// Safety class with default parameters (lossless optimisers: `Reversible` or `LightCopy`).
    pub safety: SafetyClass,
    pub default_action: ActionKind,
    pub available_actions: Vec<ActionKind>,
    /// Preset tiles shown first in the step panel.
    pub presets: Vec<StepPreset>,
    /// Toggles and inputs shown after the presets.
    pub settings: Vec<SettingSpec>,
    pub platform: PlatformSupport,
    /// External tools used (`["ffmpeg"]`); empty for in-house optimisers.
    pub tools: Vec<String>,
    pub feature_ids: Vec<String>,
    pub unit: String,
}

/// One optimisation request.
pub struct OptimiseJob<'a> {
    /// The file (or folder, for archives) to read.
    pub src: &'a Path,
    /// Where to write the result. Chosen by the action engine (same directory, temporary name);
    /// it does not exist yet. Must not be written when the optimiser fails.
    pub dst: &'a Path,
    pub params: &'a ParamValues,
    pub tools: &'a [ExternalTool],
    /// Scratch directory the optimiser may use (AUT-07); cleaned by the caller.
    pub temp_dir: &'a Path,
    pub cancel: &'a CancelToken,
    pub progress: &'a dyn ProgressSink,
}

/// What an optimiser produced.
///
/// Build it with `..OptimiseOutput::default()` so additive fields never break callers.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OptimiseOutput {
    pub bytes_before: u64,
    /// Size of `dst` (equal to `bytes_before` when skipped).
    pub bytes_after: u64,
    /// True when the output is not equivalent to the input.
    pub lossy: bool,
    /// When set, nothing was written and the reason says why (signed, encrypted, not smaller...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Extension (lower case, no dot) of the format `dst` holds when the optimiser converted
    /// the file to another type (WebP, Opus, MP4...). `None`: same type as the source. The
    /// action engine names the result with it (W4-INT; previously a `notes` entry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_extension: Option<String>,
    /// Stored bytes of the *input* per part (`images`, `fonts`, `content`...), largest first
    /// (OPT-ENG-09). Empty when the optimiser does not know.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breakdown: Vec<SizePart>,
    /// The rewrite exists for privacy (GPS / metadata strip): an output of the *same* size is
    /// kept even though keep-if-not-smaller would drop it. A larger output is never kept.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub privacy: bool,
}

/// One part of a file's size (OPT-ENG-09).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizePart {
    /// `images`, `fonts`, `media`, `objects`, `caches`, `content`, `styles`, `metadata`, `other`.
    pub part: String,
    pub bytes: u64,
}

impl OptimiseOutput {
    /// The converted type's extension: the field, else the legacy `output_extension=<ext>`
    /// note. Only short ASCII alphanumeric extensions are accepted (never a path).
    pub fn converted_extension(&self) -> Option<String> {
        let raw = self.output_extension.clone().or_else(|| {
            self.notes.iter().find_map(|n| n.strip_prefix("output_extension=").map(|s| s.trim().to_string()))
        })?;
        let ext = raw.trim_start_matches('.').to_ascii_lowercase();
        let ok = !ext.is_empty() && ext.len() <= 10 && ext.bytes().all(|b| b.is_ascii_alphanumeric());
        ok.then_some(ext)
    }
}

/// An optimiser. Implementations live in `mj-optimise`.
pub trait Optimiser: Send + Sync {
    /// Static description.
    fn info(&self) -> OptimiserInfo;

    /// Whether it can run here (platform, external tools found).
    fn availability(&self, tools: &[ExternalTool]) -> Availability;

    /// Cheap test on metadata (extension, size) whether `entry` is a target.
    fn applies_to(&self, entry: &FileEntry) -> bool;

    /// Cheap estimate for one entry (no full decode). `Confidence::Exact` only from a dry run.
    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate>;

    /// Transform `job.src` into `job.dst`.
    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput>;

    /// Build candidates from the index. Typical implementation: every file that
    /// `applies_to`, sized by `estimate`, badged by the engine afterwards.
    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput>;
}

/// One archive-creation request.
pub struct ArchiveJob<'a> {
    /// Files or folders to store, recorded relative to `base`.
    pub sources: &'a [PathBuf],
    pub base: &'a Path,
    /// Archive to create (does not exist yet).
    pub dst: &'a Path,
    pub format: ArchiveFormat,
    /// 0-9.
    pub level: u8,
    pub tools: &'a [ExternalTool],
    pub cancel: &'a CancelToken,
    pub progress: &'a dyn ProgressSink,
}

/// Archive creation and verification (W3-B), used by `Action::Archive`.
pub trait Archiver: Send + Sync {
    /// Formats this archiver can write here (ZIP always; 7z/zstd need external tools).
    fn formats(&self, tools: &[ExternalTool]) -> Vec<ArchiveFormat>;

    /// Write `job.sources` into a new archive at `job.dst`. Returns the archive size.
    fn create(&self, job: &ArchiveJob<'_>) -> Result<u64>;

    /// Integrity test (OPT-ARC-02): every source file is present with identical content.
    fn verify(&self, archive: &Path, sources: &[PathBuf], base: &Path, cancel: &CancelToken) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::OptimiseOutput;

    #[test]
    fn converted_extension_reads_the_field_then_the_legacy_note_and_rejects_paths() {
        let mut o = OptimiseOutput { output_extension: Some(".WebP".into()), ..OptimiseOutput::default() };
        assert_eq!(o.converted_extension().as_deref(), Some("webp"));
        o.output_extension = None;
        o.notes = vec!["output_extension=opus".into()];
        assert_eq!(o.converted_extension().as_deref(), Some("opus"));
        o.notes = vec!["output_extension=../../x".into()];
        assert_eq!(o.converted_extension(), None);
        assert_eq!(OptimiseOutput::default().converted_extension(), None);
        // Old JSON (no new fields) still loads.
        let old: OptimiseOutput =
            serde_json::from_str(r#"{"bytes_before":1,"bytes_after":1,"lossy":false,"notes":[]}"#).unwrap();
        assert_eq!(old, OptimiseOutput { bytes_before: 1, bytes_after: 1, ..OptimiseOutput::default() });
    }
}
