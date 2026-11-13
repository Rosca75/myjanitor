//! PDF optimisation: lossless structural pass (OPT-PDF-03/05) and image downsampling presets (OPT-PDF-01/02/04).
//! Owner: W3-B.
//!
//! [`PdfLossless`] wraps `mj_formats::pdf::structural` with the file-level
//! safety rules: the pass runs inside `catch_unwind`, the output is written
//! only to `job.dst` (never over anything), read back from disk and verified
//! against the original; any failure removes `dst` and reports "skipped".
//!
//! [`PdfLossy`] is visible but disabled: image downsampling needs a JPEG/JPX
//! encoder and font subsetting a TrueType/CFF subsetter, neither of which is
//! bundled (decision D-3B05).

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use mj_core::params::ChoiceOption;
use mj_core::{
    ActionKind, Availability, Candidate, CandidateId, Category, Confidence, DetectContext, DetectorOutput, Error,
    Estimate, ExternalTool, FileEntry, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues,
    PlatformSupport, Result, SafetyClass, SettingKind, SettingSpec, Source, TargetKind,
};
use mj_formats::pdf::{self as fpdf, Options, Outcome};

#[cfg(test)]
mod tests;

/// PDF files: Lossless structural optimisation
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfLossless;

/// Extensions (lower case) this optimiser targets.
pub const PDFLOSSLESS_EXTENSIONS: &[&str] = &["pdf"];

/// Files up to this size are estimated by running the pass in memory.
pub const SAMPLE_ESTIMATE_MAX: u64 = 4 * 1024 * 1024;

/// Saving assumed for larger files when the tail shows a classic layout.
const MODEL_RATIO_CLASSIC: f64 = 0.90;
/// ... and when it already uses cross-reference streams.
const MODEL_RATIO_MODERN: f64 = 0.97;

fn toggle(key: &str, label: &str, help: &str, default: bool, lossy: bool, advanced: bool) -> SettingSpec {
    SettingSpec {
        key: key.into(),
        label: label.into(),
        help: Some(help.into()),
        kind: SettingKind::Toggle { default },
        advanced,
        lossy,
        disabled_reason: None,
    }
}

/// The step-panel settings (keys are the wire contract of the params).
pub fn lossless_settings() -> Vec<SettingSpec> {
    vec![
        toggle(
            "remove_thumbnails",
            "Remove page thumbnails",
            "Viewers regenerate them; nothing visible changes",
            true,
            false,
            false,
        ),
        toggle("remove_xmp", "Remove XMP metadata", "Author, tool and history metadata streams", false, true, false),
        toggle("remove_info", "Remove document information", "Title, author, producer and dates", false, true, false),
        toggle("remove_attachments", "Remove attachments", "Files embedded in the PDF", false, true, false),
        toggle("remove_javascript", "Remove JavaScript", "Document and page scripts", false, true, false),
        toggle(
            "remove_piece_info",
            "Remove private application data",
            "Editing data kept by Illustrator and others",
            false,
            true,
            false,
        ),
        toggle(
            "object_streams",
            "Use object streams (PDF 1.5)",
            "Smaller files; very old readers cannot open them",
            true,
            false,
            true,
        ),
    ]
}

/// Pass options from step parameters.
pub fn options_from(params: &ParamValues) -> Options {
    let d = Options::default();
    Options {
        object_streams: params.bool("object_streams", d.object_streams),
        remove_thumbnails: params.bool("remove_thumbnails", d.remove_thumbnails),
        remove_xmp: params.bool("remove_xmp", d.remove_xmp),
        remove_info: params.bool("remove_info", d.remove_info),
        remove_attachments: params.bool("remove_attachments", d.remove_attachments),
        remove_javascript: params.bool("remove_javascript", d.remove_javascript),
        remove_piece_info: params.bool("remove_piece_info", d.remove_piece_info),
        ..d
    }
}

/// Result of the whole file-level operation, before it is mapped to the contract types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileResult {
    /// `dst` was written and verified.
    Written { before: u64, after: u64, notes: Vec<String> },
    /// Nothing written (`dst` absent). `code` is a stable diagnostics key.
    Skipped { before: u64, code: &'static str, reason: String },
}

/// Run the pass on `src` and write a verified, smaller result to `dst`.
pub fn optimise_file(src: &Path, dst: &Path, opts: &Options) -> Result<FileResult> {
    let input = read_bounded(src)?;
    let before = input.len() as u64;
    let outcome = match catch_unwind(AssertUnwindSafe(|| fpdf::optimise(&input, opts))) {
        Ok(o) => o,
        Err(_) => {
            return Ok(FileResult::Skipped {
                before,
                code: "internal",
                reason: "Unexpected error while reading this PDF: left unchanged".into(),
            })
        }
    };
    let (bytes, report) = match outcome {
        Outcome::Skipped(r) => return Ok(FileResult::Skipped { before, code: r.code(), reason: r.message() }),
        Outcome::Optimised { bytes, report } => (bytes, report),
    };
    write_new(dst, &bytes)?;
    // Verify what is really on disk, not the buffer we meant to write.
    let verdict = match fs::read(dst) {
        Ok(back) => catch_unwind(AssertUnwindSafe(|| fpdf::verify(&input, &back, opts)))
            .unwrap_or_else(|_| Err("verification panicked".into())),
        Err(e) => Err(format!("output could not be read back: {e}")),
    };
    if let Err(why) = verdict {
        let _ = fs::remove_file(dst);
        return Ok(FileResult::Skipped {
            before,
            code: "verification_failed",
            reason: format!("Not optimised: verification failed ({why})"),
        });
    }
    let after = fs::metadata(dst).map(|m| m.len()).unwrap_or(bytes.len() as u64);
    if after >= before {
        let _ = fs::remove_file(dst);
        return Ok(FileResult::Skipped { before, code: "not_smaller", reason: fpdf::SkipReason::NotSmaller.message() });
    }
    Ok(FileResult::Written { before, after, notes: notes(&report) })
}

fn read_bounded(src: &Path) -> Result<Vec<u8>> {
    let len = fs::symlink_metadata(src).map_err(|e| Error::io(src, e))?.len();
    if len > fpdf::document::MAX_FILE as u64 {
        return Err(Error::unsupported("PDF optimisation", "files over 512 MB are not rewritten"));
    }
    fs::read(src).map_err(|e| Error::io(src, e))
}

/// Create `dst` (must not exist), write and flush it. On any error nothing is left behind.
fn write_new(dst: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(dst).map_err(|e| Error::io(dst, e))?;
    let r = f.write_all(bytes).and_then(|_| f.sync_all());
    if let Err(e) = r {
        drop(f);
        let _ = fs::remove_file(dst);
        return Err(Error::io(dst, e));
    }
    Ok(())
}

/// Report lines: a machine-readable breakdown (OPT-ENG-09) and a human summary.
fn notes(r: &fpdf::Report) -> Vec<String> {
    let b = r.breakdown;
    let mut out = vec![format!(
        "breakdown images={} fonts={} content={} metadata={} other={}",
        b.images, b.fonts, b.content, b.metadata, b.other
    )];
    out.push(format!(
        "{} streams recompressed, {} compressed, {} duplicates merged, objects {} -> {}{}",
        r.streams_recompressed,
        r.streams_compressed,
        r.streams_merged,
        r.objects_before,
        r.objects_after,
        if r.object_streams { ", object streams" } else { "" }
    ));
    for (what, bytes) in &r.removed {
        out.push(format!("removed {what}={bytes}"));
    }
    out.extend(r.notes.iter().cloned());
    out
}

/// Head (first 1 KiB) and tail (last 64 KiB) of a file.
fn head_tail(path: &Path) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let mut f = fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut head = vec![0u8; len.min(1024) as usize];
    f.read_exact(&mut head)?;
    let tail_len = len.min(64 * 1024);
    f.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0u8; tail_len as usize];
    f.read_exact(&mut tail)?;
    Ok((head, tail))
}

/// Estimate for one file, with the reason when it will be skipped.
pub fn estimate_file(entry: &FileEntry, opts: &Options) -> (Estimate, Option<&'static str>) {
    let before = entry.size;
    let skip = |code| {
        (
            Estimate {
                files: 1,
                bytes_before: before,
                bytes_after: before,
                confidence: Confidence::Estimated,
                skipped: 1,
            },
            Some(code),
        )
    };
    if entry.is_online_only() {
        return skip("online_only");
    }
    if before <= SAMPLE_ESTIMATE_MAX {
        let Ok(input) = fs::read(&entry.path) else { return skip("unreadable") };
        return match catch_unwind(AssertUnwindSafe(|| fpdf::optimise(&input, opts))) {
            Ok(Outcome::Optimised { bytes, .. }) => (
                Estimate {
                    files: 1,
                    bytes_before: before,
                    bytes_after: bytes.len() as u64,
                    confidence: Confidence::Estimated,
                    skipped: 0,
                },
                None,
            ),
            Ok(Outcome::Skipped(r)) => skip(r.code()),
            Err(_) => skip("internal"),
        };
    }
    let Ok((head, tail)) = head_tail(&entry.path) else { return skip("unreadable") };
    let (is_pdf, encrypted, signed) = fpdf::quick_flags(&head, &tail);
    if !is_pdf {
        return skip("unparseable");
    }
    if encrypted {
        return skip("encrypted");
    }
    if signed {
        return skip("signed");
    }
    let modern = fpdf::document::contains(&tail, b"/XRef") || fpdf::document::contains(&tail, b"/ObjStm");
    let ratio = if modern { MODEL_RATIO_MODERN } else { MODEL_RATIO_CLASSIC };
    (
        Estimate {
            files: 1,
            bytes_before: before,
            bytes_after: (before as f64 * ratio) as u64,
            confidence: Confidence::Estimated,
            skipped: 0,
        },
        None,
    )
}

/// A file candidate with the usual defaults.
pub(crate) fn file_candidate(entry: &FileEntry, source: OptimiserId, size_after: u64, reason: String) -> Candidate {
    Candidate {
        id: CandidateId(0),
        path: entry.path.clone(),
        target: TargetKind::File,
        source: Source::Optimiser(source),
        size: entry.size,
        size_after,
        confidence: Confidence::Estimated,
        reason,
        badges: Default::default(),
        default_selected: true,
        group: None,
        sub_group: None,
        modified: entry.modified,
        stamp: entry.stamp(),
        file_count: None,
        columns: Default::default(),
    }
}

impl Optimiser for PdfLossless {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::PdfLossless,
            title: "PDF files".into(),
            description: "Lossless structural optimisation".into(),
            category: Category::Optimise,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: lossless_settings(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-PDF-03".into(), "OPT-PDF-05".into()],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && PDFLOSSLESS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    /// Cheap estimate: files up to 4 MiB run the pass in memory (no verification,
    /// nothing written); larger files use a ratio model after a head/tail check.
    /// Both are `Estimated`; `Exact` comes only from the framework's dry run.
    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        Ok(estimate_file(entry, &options_from(params)).0)
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        job.cancel.checkpoint()?;
        let opts = options_from(job.params);
        match optimise_file(job.src, job.dst, &opts)? {
            FileResult::Written { before, after, notes } => Ok(OptimiseOutput {
                bytes_before: before,
                bytes_after: after,
                lossy: opts.is_lossy(),
                skipped: None,
                notes,
            }),
            FileResult::Skipped { before, reason, .. } => Ok(OptimiseOutput {
                bytes_before: before,
                bytes_after: before,
                lossy: false,
                skipped: Some(reason),
                notes: Vec::new(),
            }),
        }
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let opts = options_from(ctx.params);
        let entries: Vec<&FileEntry> = ctx.index.files().filter(|e| self.applies_to(e)).collect();
        let results = mj_core::pool::parallel_map(&entries, ctx.threads, ctx.cancel, |_, e| estimate_file(e, &opts));
        ctx.cancel.checkpoint()?;
        let mut out = DetectorOutput::default();
        for (e, r) in entries.iter().zip(results) {
            match r {
                None => out.diagnostics.unreadable(&e.path),
                Some((_, Some("unreadable"))) => out.diagnostics.unreadable(&e.path),
                Some((_, Some(code))) => out.diagnostics.skip(code),
                Some((est, None)) if est.saving() == 0 => out.diagnostics.skip("not_smaller"),
                Some((est, None)) => {
                    let pct = est.saving() * 100 / est.bytes_before.max(1);
                    out.candidates.push(file_candidate(
                        e,
                        OptimiserId::PdfLossless,
                        est.bytes_after,
                        format!("Lossless rewrite: about {pct}% smaller"),
                    ));
                }
            }
        }
        Ok(out)
    }
}

/// PDF images: Downsample images to a target DPI
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfLossy;

/// Extensions (lower case) this optimiser targets.
pub const PDFLOSSY_EXTENSIONS: &[&str] = &["pdf"];

/// Why [`PdfLossy`] is disabled.
pub const PDF_LOSSY_REASON: &str =
    "Image downsampling and font subsetting need an image codec and a font subsetter that are not bundled";

impl Optimiser for PdfLossy {
    fn info(&self) -> OptimiserInfo {
        let dpi = ["72", "150", "300"]
            .iter()
            .map(|v| ChoiceOption { value: (*v).into(), label: format!("{v} dpi"), help: None })
            .collect();
        OptimiserInfo {
            id: OptimiserId::PdfLossy,
            title: "PDF images".into(),
            description: "Downsample images to a target DPI".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: vec![
                SettingSpec {
                    key: "preset".into(),
                    label: "Preset".into(),
                    help: Some("Screen 72 dpi, eBook 150 dpi, Prepress 300 dpi".into()),
                    kind: SettingKind::Choice {
                        default: "ebook".into(),
                        options: vec![
                            ChoiceOption { value: "screen".into(), label: "Screen".into(), help: None },
                            ChoiceOption { value: "ebook".into(), label: "eBook".into(), help: None },
                            ChoiceOption { value: "prepress".into(), label: "Prepress".into(), help: None },
                        ],
                    },
                    advanced: false,
                    lossy: true,
                    disabled_reason: Some(PDF_LOSSY_REASON.into()),
                },
                SettingSpec {
                    key: "dpi".into(),
                    label: "Image resolution".into(),
                    help: None,
                    kind: SettingKind::Choice { default: "150".into(), options: dpi },
                    advanced: false,
                    lossy: true,
                    disabled_reason: Some(PDF_LOSSY_REASON.into()),
                },
                SettingSpec {
                    key: "subset_fonts".into(),
                    label: "Subset embedded fonts".into(),
                    help: Some("Byte-identical duplicate fonts are already merged by the lossless pass".into()),
                    kind: SettingKind::Toggle { default: false },
                    advanced: false,
                    lossy: false,
                    disabled_reason: Some(PDF_LOSSY_REASON.into()),
                },
            ],
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-PDF-01".into(), "OPT-PDF-02".into(), "OPT-PDF-04".into()],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Disabled { reason: PDF_LOSSY_REASON.into() }
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && PDFLOSSY_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::unsupported("PDF image downsampling", PDF_LOSSY_REASON))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::unsupported("PDF image downsampling", PDF_LOSSY_REASON))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::unsupported("PDF image downsampling", PDF_LOSSY_REASON))
    }
}
