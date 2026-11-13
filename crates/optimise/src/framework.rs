//! Optimisation framework (W3-A), as far as the frozen contract places it in
//! `mj-optimise`. Every write to a user path (replacing the original, the
//! originals policy, light copies beside the original, journaling) is done by
//! `mj-actions`; this module provides everything around the pure
//! `src -> dst` transform:
//!
//! - **dry run / apply over a batch** ([`run_batch`], OPT-ENG-03): every file is
//!   optimised into a caller-supplied working directory, measured, and kept or
//!   discarded by the minimum-gain rule ([`MinGain`], OPT-ENG-04). Each
//!   optimiser call runs inside `catch_unwind`; a failing or panicking file is
//!   reported and never aborts the batch; cancel / pause are honoured between
//!   files and progress is reported per file;
//! - **cheap estimates** ([`estimate_batch`]) and an analysis helper
//!   ([`analyse_by_content`]) that routes files by content, not by extension
//!   (OPT-ENG-02, [`route`]);
//! - **light-copy naming** ([`light_copy_path`], OPT-ENG-07) and the **hand-off
//!   request** to the action engine ([`optimise_item`], OPT-ENG-06/07);
//! - **metadata preservation** for files this crate writes ([`preserve_metadata`],
//!   OPT-ENG-05);
//! - **fit to a target size** ([`fit_to_target`], OPT-ENG-08);
//! - **lossless / lossy switch** global and per type ([`LossyPolicy`], OPT-ENG-01);
//! - **per-file size breakdown** ([`SizeBreakdown`], OPT-ENG-09).

use std::collections::BTreeMap;
use std::io::{Read, Seek};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use mj_core::{
    Action, ActionItem, CancelToken, Candidate, CandidateId, Confidence, DetectContext, DetectorOutput, Error,
    Estimate, ExternalTool, FileEntry, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo,
    OriginalsPolicy, OutputMode, ParamValue, ParamValues, Phase, Progress, ProgressSink, Result, SettingKind, Source,
    TargetKind,
};
use mj_formats::sniff::{self, FileKind};

/// Default light-copy suffix (`report.pptx` -> `report_light.pptx`).
pub const LIGHT_SUFFIX: &str = "_light";

// ---------------------------------------------------------------- panics

/// Run `f`, turning a panic into `Error::Internal` (hostile input must never kill a worker).
pub fn guarded<T>(what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            Err(Error::Internal(format!("{what} crashed: {msg}")))
        }
    }
}

// ---------------------------------------------------------------- keep-if-not-smaller

/// Minimum gain an output must bring to replace the original (OPT-ENG-04). Both
/// conditions must hold: at least `bytes` saved and at least `percent` of the original.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinGain {
    pub bytes: u64,
    pub percent: f64,
}

impl Default for MinGain {
    fn default() -> Self {
        MinGain { bytes: 512, percent: 0.5 }
    }
}

impl MinGain {
    /// Any strictly smaller output.
    pub const ANY: MinGain = MinGain { bytes: 1, percent: 0.0 };

    /// True when `after` is worth keeping over `before`.
    pub fn accepts(&self, before: u64, after: u64) -> bool {
        if after >= before {
            return false;
        }
        let saved = before - after;
        saved >= self.bytes.max(1) && (saved as f64) * 100.0 >= self.percent * before as f64
    }

    /// `None` when worth it, else the reason the original is kept.
    pub fn reject_reason(&self, before: u64, after: u64) -> Option<String> {
        if self.accepts(before, after) {
            None
        } else if after >= before {
            Some("the optimised file was not smaller; the original was kept".into())
        } else {
            Some(format!(
                "saving {} is below the minimum gain; the original was kept",
                mj_core::size::format_bytes(before - after)
            ))
        }
    }
}

// ---------------------------------------------------------------- light copies and hand-off

/// `<dir>/<stem><suffix>.<ext>` beside `original`, never an existing path: on a
/// collision ` (n)` is appended (`a_light (1).pptx`). An empty suffix means [`LIGHT_SUFFIX`].
pub fn light_copy_path(original: &Path, suffix: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let suffix = if suffix.is_empty() { LIGHT_SUFFIX } else { suffix };
    let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = original.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let wanted = original.with_file_name(format!("{stem}{suffix}{ext}"));
    mj_core::pathutil::unique_name(&wanted, exists)
}

/// The request the action engine executes for one optimised file (OPT-ENG-06/07):
/// the originals policy (Recycle Bin / quarantine, backup folder, delete) applies only
/// in replace mode; a light copy removes nothing. Keep-if-not-smaller is always on.
pub fn optimise_item(
    path: &Path,
    root: &Path,
    expected: mj_core::FileStamp,
    optimiser: OptimiserId,
    params: ParamValues,
    output: OutputMode,
    originals: OriginalsPolicy,
) -> ActionItem {
    ActionItem {
        path: path.to_path_buf(),
        target: TargetKind::File,
        action: Action::Optimise { optimiser, params, output, originals, keep_if_not_smaller: true },
        link_source: None,
        root: root.to_path_buf(),
        expected,
    }
}

/// Light-copy output mode with the default suffix.
pub fn light_copy_mode() -> OutputMode {
    OutputMode::LightCopy { suffix: LIGHT_SUFFIX.into() }
}

// ---------------------------------------------------------------- metadata

/// Copy the modification / access times and the permissions of `src` onto `dst`
/// (OPT-ENG-05). Used for files this crate writes itself (working-directory
/// outputs); user paths are handled by the action engine with the same rules.
pub fn preserve_metadata(src: &Path, dst: &Path, times: bool, permissions: bool) -> Result<()> {
    let md = std::fs::metadata(src).map_err(|e| Error::io(src, e))?;
    if permissions {
        std::fs::set_permissions(dst, md.permissions()).map_err(|e| Error::io(dst, e))?;
    }
    if times {
        let mut t = std::fs::FileTimes::new();
        if let Ok(m) = md.modified() {
            t = t.set_modified(m);
        }
        if let Ok(a) = md.accessed() {
            t = t.set_accessed(a);
        }
        let f = std::fs::OpenOptions::new().write(true).open(dst).map_err(|e| Error::io(dst, e))?;
        f.set_times(t).map_err(|e| Error::io(dst, e))?;
    }
    Ok(())
}

// ---------------------------------------------------------------- lossless / lossy switch

/// OPT-ENG-01: lossless-only globally, with per-type overrides.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LossyPolicy {
    /// Global switch (`Settings::optimise::lossless_only`).
    pub lossless_only: bool,
    /// Per optimiser: `true` allows lossy settings even when the global switch is on,
    /// `false` forbids them even when it is off.
    pub per_type: BTreeMap<OptimiserId, bool>,
}

impl LossyPolicy {
    /// Whether lossy settings are allowed for `id`.
    pub fn allows_lossy(&self, id: OptimiserId) -> bool {
        self.per_type.get(&id).copied().unwrap_or(!self.lossless_only)
    }
}

fn is_spec_default(kind: &SettingKind, v: &ParamValue) -> bool {
    match (kind, v) {
        (SettingKind::Toggle { default }, ParamValue::Bool(b)) => b == default,
        (SettingKind::Number { default, .. }, ParamValue::Number(n)) => n == default,
        (SettingKind::Choice { default, .. }, ParamValue::Text(t))
        | (SettingKind::Text { default, .. }, ParamValue::Text(t))
        | (SettingKind::Folder { default }, ParamValue::Text(t)) => t == default,
        _ => false,
    }
}

/// True when `params` turn on any lossy setting (a lossy toggle set to true, or any
/// other lossy setting moved away from its default).
pub fn params_are_lossy(info: &OptimiserInfo, params: &ParamValues) -> bool {
    info.settings.iter().filter(|s| s.lossy).any(|s| match params.0.get(&s.key) {
        None => matches!(s.kind, SettingKind::Toggle { default: true }),
        Some(ParamValue::Bool(b)) if matches!(s.kind, SettingKind::Toggle { .. }) => *b,
        Some(v) => !is_spec_default(&s.kind, v),
    })
}

/// Parameters with every lossy setting reset (toggles off, others to their default)
/// when the policy forbids lossy output for this optimiser; disabled settings are
/// always reset to their default.
pub fn effective_params(info: &OptimiserInfo, params: &ParamValues, policy: &LossyPolicy) -> ParamValues {
    let lossy_ok = policy.allows_lossy(info.id);
    let mut out = params.clone();
    for s in &info.settings {
        if s.disabled_reason.is_some() {
            out.set(&s.key, s.default_value());
        } else if s.lossy && !lossy_ok {
            let v = match s.kind {
                SettingKind::Toggle { .. } => ParamValue::Bool(false),
                _ => s.default_value(),
            };
            out.set(&s.key, v);
        }
    }
    out
}

/// Presets visible under the policy (lossy presets hidden when lossy output is not allowed).
pub fn visible_presets(info: &OptimiserInfo, policy: &LossyPolicy) -> Vec<mj_core::StepPreset> {
    let ok = policy.allows_lossy(info.id);
    info.presets.iter().filter(|p| ok || !p.lossy).cloned().collect()
}

// ---------------------------------------------------------------- content routing

/// What a file really is, and which optimiser should handle it (OPT-ENG-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub kind: Option<FileKind>,
    pub optimiser: Option<OptimiserId>,
    /// The content does not fit the extension (a renamed file).
    pub mismatch: bool,
}

/// Refine a ZIP container whose first bytes were not conclusive by looking at its
/// central directory: OOXML main parts, EPUB / ODF `mimetype`, JAR manifest.
pub fn refine_zip<R: Read + Seek>(r: R) -> Option<FileKind> {
    let z = mj_formats::zip::ZipArchive::new(r).ok()?;
    let has = |n: &str| z.entries().iter().any(|e| e.name.eq_ignore_ascii_case(n));
    let prefix =
        |p: &str| z.entries().iter().any(|e| e.name.len() >= p.len() && e.name[..p.len()].eq_ignore_ascii_case(p));
    if has("[Content_Types].xml") {
        if prefix("ppt/") {
            return Some(FileKind::Pptx);
        }
        if prefix("xl/") {
            return Some(FileKind::Xlsx);
        }
        if prefix("word/") {
            return Some(FileKind::Docx);
        }
    }
    if has("mimetype") {
        if has("META-INF/container.xml") {
            return Some(FileKind::Epub);
        }
        return Some(FileKind::Odf);
    }
    if has("META-INF/MANIFEST.MF") {
        return Some(FileKind::Jar);
    }
    Some(FileKind::Zip)
}

/// Detect the content kind of a file (bounded prefix, plus the ZIP directory when needed).
pub fn content_kind(path: &Path) -> Result<Option<FileKind>> {
    let head = sniff::read_prefix(path, sniff::SNIFF_LEN).map_err(|e| Error::io(path, e))?;
    let kind = sniff::sniff(&head);
    if matches!(kind, Some(FileKind::Zip) | Some(FileKind::Jar)) {
        let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
        return Ok(refine_zip(std::io::BufReader::new(f)).or(kind));
    }
    Ok(kind)
}

/// The optimiser for a content kind (`ext` disambiguates plain ZIP containers).
pub fn optimiser_for(kind: FileKind, ext: &str) -> Option<OptimiserId> {
    Some(match kind {
        FileKind::Pptx => OptimiserId::PowerPoint,
        FileKind::Xlsx => OptimiserId::Excel,
        FileKind::Docx => OptimiserId::Word,
        FileKind::Epub | FileKind::Odf | FileKind::Jar => OptimiserId::ZipRepack,
        FileKind::Zip if ext == "cbz" => OptimiserId::ZipRepack,
        FileKind::Png => OptimiserId::PngLossless,
        FileKind::Jpeg => OptimiserId::JpegLossless,
        FileKind::Pdf => OptimiserId::PdfLossless,
        _ => return None,
    })
}

/// Route a file by its content.
pub fn route(path: &Path) -> Result<Route> {
    let ext = mj_core::pathutil::extension_lower(path);
    let kind = content_kind(path)?;
    let expected = sniff::expected_kinds(&ext);
    let mismatch = !expected.is_empty() && kind.is_some_and(|k| !expected.contains(&k));
    Ok(Route { kind, optimiser: kind.and_then(|k| optimiser_for(k, &ext)), mismatch })
}

// ---------------------------------------------------------------- size breakdown

/// What part of a file the bytes belong to (OPT-ENG-09, OPT-XLS-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BreakdownCategory {
    Images,
    Fonts,
    Media,
    /// Embedded objects (OLE, embedded workbooks, ActiveX).
    Objects,
    /// Caches (pivot caches, thumbnails).
    Caches,
    /// Worksheets / slides / document body.
    Content,
    Styles,
    Other,
}

impl BreakdownCategory {
    /// Short label.
    pub fn label(self) -> &'static str {
        match self {
            BreakdownCategory::Images => "images",
            BreakdownCategory::Fonts => "fonts",
            BreakdownCategory::Media => "media",
            BreakdownCategory::Objects => "objects",
            BreakdownCategory::Caches => "caches",
            BreakdownCategory::Content => "content",
            BreakdownCategory::Styles => "styles",
            BreakdownCategory::Other => "other",
        }
    }
}

/// Stored (compressed) bytes per category.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SizeBreakdown {
    pub bytes: BTreeMap<BreakdownCategory, u64>,
}

impl SizeBreakdown {
    /// Add bytes to a category.
    pub fn add(&mut self, c: BreakdownCategory, n: u64) {
        *self.bytes.entry(c).or_default() += n;
    }

    /// Sum over all categories.
    pub fn total(&self) -> u64 {
        self.bytes.values().sum()
    }

    /// Categories, largest first (ties by category order).
    pub fn sorted(&self) -> Vec<(BreakdownCategory, u64)> {
        let mut v: Vec<_> = self.bytes.iter().map(|(c, n)| (*c, *n)).filter(|(_, n)| *n > 0).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// One line for a table column: `images 4.2 MB · fonts 1.1 MB · other 3 KB`.
    pub fn summary(&self) -> String {
        self.sorted()
            .iter()
            .map(|(c, n)| format!("{} {}", c.label(), mj_core::size::format_bytes(*n)))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

// ---------------------------------------------------------------- batch

/// What happened to one file of a batch.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Output kept: it is at `output` in the working directory (removed after a dry run).
    Smaller { before: u64, after: u64, lossy: bool, output: Option<PathBuf>, notes: Vec<String> },
    /// Optimised but not worth it; nothing kept.
    Kept { before: u64, after: u64, reason: String },
    /// The optimiser skipped the file (signed, encrypted, unsupported content...).
    Skipped { reason: String },
    /// The optimiser failed or panicked.
    Failed { error: Error },
    /// Not processed: the batch was cancelled.
    Cancelled,
}

/// One file's result.
#[derive(Debug, Clone, PartialEq)]
pub struct FileResult {
    pub path: PathBuf,
    pub outcome: Outcome,
}

/// Options of [`run_batch`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchOptions {
    /// Measure only: outputs are deleted after measuring (OPT-ENG-03).
    pub dry_run: bool,
    pub min_gain: MinGain,
}

impl Default for BatchOptions {
    fn default() -> Self {
        BatchOptions { dry_run: true, min_gain: MinGain::default() }
    }
}

/// Batch results and their total.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatchReport {
    pub results: Vec<FileResult>,
    /// Exact numbers (measured), files counted only when kept.
    pub estimate: Estimate,
    pub cancelled: bool,
}

/// Everything a batch needs besides the files.
pub struct BatchContext<'a> {
    pub params: &'a ParamValues,
    pub tools: &'a [ExternalTool],
    /// Caller-supplied working directory (created if missing; outputs go here).
    pub work_dir: &'a Path,
    pub cancel: &'a CancelToken,
    pub progress: &'a dyn ProgressSink,
}

fn output_name(work_dir: &Path, index: usize, src: &Path) -> PathBuf {
    let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    mj_core::pathutil::unique_name(&work_dir.join(format!("mj-opt-{index}{ext}")), |p| p.exists())
}

/// Optimise one file into `dst` with every guard: panics caught, a skipped or
/// failed run leaves no file behind, the output must exist.
pub fn optimise_one(
    opt: &dyn Optimiser,
    src: &Path,
    dst: &Path,
    ctx: &BatchContext<'_>,
    params: &ParamValues,
) -> Result<OptimiseOutput> {
    let scratch = ctx.work_dir.join("scratch");
    let _ = std::fs::create_dir_all(&scratch);
    let job = OptimiseJob {
        src,
        dst,
        params,
        tools: ctx.tools,
        temp_dir: &scratch,
        cancel: ctx.cancel,
        progress: ctx.progress,
    };
    let res = guarded("optimiser", || opt.optimise(&job));
    match res {
        Ok(o) if o.skipped.is_some() => {
            let _ = std::fs::remove_file(dst);
            Ok(o)
        }
        Ok(o) => match std::fs::metadata(dst) {
            Ok(m) if m.is_file() => Ok(OptimiseOutput { bytes_after: m.len(), ..o }),
            _ => Err(Error::Internal("the optimiser reported success but wrote no file".into())),
        },
        Err(e) => {
            let _ = std::fs::remove_file(dst);
            Err(e)
        }
    }
}

/// Dry run (or apply into the working directory) over `files` (OPT-ENG-03/04/11).
/// Never aborts on one file; stops between files when cancelled.
pub fn run_batch(opt: &dyn Optimiser, files: &[PathBuf], ctx: &BatchContext<'_>, opts: BatchOptions) -> BatchReport {
    let mut rep = BatchReport::default();
    if let Err(e) = std::fs::create_dir_all(ctx.work_dir) {
        let err = Error::io(ctx.work_dir, e);
        rep.results = files
            .iter()
            .map(|p| FileResult { path: p.clone(), outcome: Outcome::Failed { error: err.clone() } })
            .collect();
        return rep;
    }
    let total = files.len() as u64;
    let mut bytes = 0u64;
    rep.estimate.confidence = Confidence::Exact;
    for (i, src) in files.iter().enumerate() {
        if rep.cancelled || ctx.cancel.checkpoint().is_err() {
            rep.cancelled = true;
            rep.results.push(FileResult { path: src.clone(), outcome: Outcome::Cancelled });
            continue;
        }
        let mut p =
            Progress::new(if opts.dry_run { Phase::Estimating } else { Phase::Applying }, i as u64, Some(total));
        p.current = Some(src.clone());
        p.bytes = bytes;
        ctx.progress.report(&p);
        let dst = output_name(ctx.work_dir, i, src);
        let before = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
        let outcome = match optimise_one(opt, src, &dst, ctx, ctx.params) {
            Err(Error::Cancelled) => {
                rep.cancelled = true;
                Outcome::Cancelled
            }
            Err(error) => Outcome::Failed { error },
            Ok(o) if o.skipped.is_some() => Outcome::Skipped { reason: o.skipped.unwrap_or_default() },
            Ok(o) => {
                let before = if o.bytes_before > 0 { o.bytes_before } else { before };
                match opts.min_gain.reject_reason(before, o.bytes_after) {
                    Some(reason) => {
                        let _ = std::fs::remove_file(&dst);
                        Outcome::Kept { before, after: o.bytes_after, reason }
                    }
                    None => {
                        let output = if opts.dry_run {
                            let _ = std::fs::remove_file(&dst);
                            None
                        } else {
                            Some(dst.clone())
                        };
                        bytes += before;
                        rep.estimate.files += 1;
                        rep.estimate.bytes_before += before;
                        rep.estimate.bytes_after += o.bytes_after;
                        Outcome::Smaller { before, after: o.bytes_after, lossy: o.lossy, output, notes: o.notes }
                    }
                }
            }
        };
        if !matches!(outcome, Outcome::Smaller { .. }) {
            rep.estimate.skipped += 1;
        }
        rep.results.push(FileResult { path: src.clone(), outcome });
    }
    let mut p = Progress::new(Phase::Estimating, total, Some(total));
    p.bytes = bytes;
    ctx.progress.report(&p);
    rep
}

/// Cheap estimates for `entries` (no full decode): the sum, and per-entry results.
/// Panics and errors are per entry; an error counts the entry as skipped.
pub fn estimate_batch(
    opt: &dyn Optimiser,
    entries: &[FileEntry],
    params: &ParamValues,
) -> (Estimate, Vec<Result<Estimate>>) {
    let mut sum = Estimate::default();
    let mut each = Vec::with_capacity(entries.len());
    for e in entries {
        let r = guarded("estimate", || opt.estimate(e, params));
        match &r {
            Ok(x) => sum = sum.add(x),
            Err(_) => {
                sum.skipped += 1;
                sum.confidence = sum.confidence.max(Confidence::Estimated);
            }
        }
        each.push(r);
    }
    (sum, each)
}

// ---------------------------------------------------------------- fit to target

/// One rung of a quality ladder (best quality first).
#[derive(Debug, Clone, PartialEq)]
pub struct Rung {
    pub label: String,
    pub params: ParamValues,
}

/// The ladder from an optimiser's presets, in declared order (best quality first).
pub fn ladder_from_presets(info: &OptimiserInfo, base: &ParamValues) -> Vec<Rung> {
    info.presets.iter().map(|p| Rung { label: p.label.clone(), params: base.merged(&p.values) }).collect()
}

/// Result of [`fit_to_target`].
#[derive(Debug, Clone, PartialEq)]
pub struct FitResult {
    /// Index and label of the rung chosen (the first under the target), if any.
    pub chosen: Option<(usize, String)>,
    /// Output of the chosen rung (or of the smallest one when none fit), in the work dir.
    pub output: Option<PathBuf>,
    pub size: Option<u64>,
    /// True when the chosen output is under the target.
    pub reached: bool,
    /// Every attempt: label and size or error text.
    pub attempts: Vec<(String, std::result::Result<u64, String>)>,
}

/// OPT-ENG-08: try the rungs in order until an output is at most `target` bytes or the
/// ladder is exhausted. The first fitting rung wins (highest quality that fits); when
/// none fits, the smallest output is kept and `reached` is false. Other outputs are deleted.
pub fn fit_to_target(
    opt: &dyn Optimiser,
    src: &Path,
    ladder: &[Rung],
    target: u64,
    ctx: &BatchContext<'_>,
) -> Result<FitResult> {
    std::fs::create_dir_all(ctx.work_dir).map_err(|e| Error::io(ctx.work_dir, e))?;
    let mut res = FitResult { chosen: None, output: None, size: None, reached: false, attempts: Vec::new() };
    let mut best: Option<(usize, PathBuf, u64)> = None;
    for (i, rung) in ladder.iter().enumerate() {
        ctx.cancel.checkpoint()?;
        let dst = output_name(ctx.work_dir, 10_000 + i, src);
        match optimise_one(opt, src, &dst, ctx, &rung.params) {
            Ok(o) if o.skipped.is_some() => res.attempts.push((rung.label.clone(), Err(o.skipped.unwrap_or_default()))),
            Ok(o) => {
                res.attempts.push((rung.label.clone(), Ok(o.bytes_after)));
                let better = best.as_ref().is_none_or(|(_, _, s)| o.bytes_after < *s);
                if o.bytes_after <= target {
                    if let Some((_, p, _)) = best.take() {
                        let _ = std::fs::remove_file(p);
                    }
                    res.chosen = Some((i, rung.label.clone()));
                    res.output = Some(dst);
                    res.size = Some(o.bytes_after);
                    res.reached = true;
                    return Ok(res);
                }
                if better {
                    if let Some((_, p, _)) = best.replace((i, dst, o.bytes_after)) {
                        let _ = std::fs::remove_file(p);
                    }
                } else {
                    let _ = std::fs::remove_file(&dst);
                }
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => res.attempts.push((rung.label.clone(), Err(e.to_string()))),
        }
    }
    if let Some((i, p, s)) = best {
        res.chosen = Some((i, ladder[i].label.clone()));
        res.output = Some(p);
        res.size = Some(s);
    }
    Ok(res)
}

// ---------------------------------------------------------------- analysis

/// Typical `Optimiser::analyse`: every usable file that the optimiser `applies_to`
/// (or, by content, `accept`s even under another extension) becomes a candidate
/// sized by `estimate`. Files whose content belongs to another optimiser are
/// skipped and counted (`content_mismatch`); unreadable files are counted.
pub fn analyse_by_content(
    opt: &dyn Optimiser,
    ctx: &DetectContext<'_>,
    kinds: &[FileKind],
    other_exts: &[&str],
    reason: impl Fn(&FileEntry, &Estimate) -> String,
    columns: impl Fn(&FileEntry) -> BTreeMap<String, String>,
) -> Result<DetectorOutput> {
    let id = opt.info().id;
    let mut out = DetectorOutput::default();
    let files: Vec<&FileEntry> = ctx
        .index
        .files()
        .filter(|e| !e.is_online_only() && e.depth > 0)
        .filter(|e| opt.applies_to(e) || other_exts.contains(&e.extension().as_str()))
        .collect();
    let total = files.len() as u64;
    for (n, e) in files.into_iter().enumerate() {
        if n % 16 == 0 {
            ctx.cancel.checkpoint()?;
            ctx.progress.report(&Progress::new(Phase::Detecting, n as u64, Some(total)));
        }
        let kind = match content_kind(&e.path) {
            Ok(k) => k,
            Err(_) => {
                out.diagnostics.unreadable(&e.path);
                continue;
            }
        };
        if !kind.is_some_and(|k| kinds.contains(&k)) {
            if opt.applies_to(e) {
                out.diagnostics.skip("content_mismatch");
            }
            continue;
        }
        let est = match guarded("estimate", || opt.estimate(e, ctx.params)) {
            Ok(x) => x,
            Err(_) => {
                out.diagnostics.unreadable(&e.path);
                continue;
            }
        };
        if est.skipped > 0 || est.saving() == 0 {
            out.diagnostics.skip(if est.skipped > 0 { "not_optimisable" } else { "nothing_to_gain" });
            continue;
        }
        out.candidates.push(Candidate {
            id: CandidateId(0),
            path: e.path.clone(),
            target: TargetKind::File,
            source: Source::Optimiser(id),
            size: e.size,
            size_after: est.bytes_after,
            confidence: est.confidence,
            reason: reason(e, &est),
            badges: mj_core::Badges::empty(),
            default_selected: true,
            group: None,
            sub_group: None,
            modified: e.modified,
            stamp: e.stamp(),
            file_count: None,
            columns: columns(e),
        });
    }
    ctx.progress.report(&Progress::new(Phase::Detecting, total, Some(total)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::{
        ActionKind, Availability, Category, NullProgress, PlatformSupport, SafetyClass, SettingSpec, StepPreset,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fake optimiser: output size = input * `quality` / 100 (param "q", default 100);
    /// panics on files named `panic*`, fails on `fail*`, skips `skip*`, cancels on `cancel*`.
    struct Fake {
        calls: AtomicUsize,
        cancel: Option<CancelToken>,
    }

    impl Fake {
        fn new() -> Self {
            Fake { calls: AtomicUsize::new(0), cancel: None }
        }
    }

    impl Optimiser for Fake {
        fn info(&self) -> OptimiserInfo {
            OptimiserInfo {
                id: OptimiserId::ImageLossy,
                title: "fake".into(),
                description: "fake".into(),
                category: Category::Optimise,
                safety: SafetyClass::Reversible,
                default_action: ActionKind::Optimise,
                available_actions: vec![ActionKind::Optimise],
                presets: [90.0, 70.0, 50.0, 30.0]
                    .iter()
                    .map(|q| StepPreset {
                        id: format!("q{q}"),
                        label: format!("quality {q}"),
                        description: String::new(),
                        values: ParamValues::new().with("q", ParamValue::Number(*q)),
                        lossy: true,
                    })
                    .collect(),
                settings: vec![
                    SettingSpec {
                        key: "q".into(),
                        label: "q".into(),
                        help: None,
                        kind: SettingKind::Number { default: 100.0, min: 1.0, max: 100.0, step: 1.0, unit: None },
                        advanced: false,
                        lossy: true,
                        disabled_reason: None,
                    },
                    SettingSpec {
                        key: "strip".into(),
                        label: "strip".into(),
                        help: None,
                        kind: SettingKind::Toggle { default: false },
                        advanced: false,
                        lossy: true,
                        disabled_reason: None,
                    },
                    SettingSpec {
                        key: "magic".into(),
                        label: "magic".into(),
                        help: None,
                        kind: SettingKind::Toggle { default: false },
                        advanced: false,
                        lossy: false,
                        disabled_reason: Some("Needs a codec".into()),
                    },
                ],
                platform: PlatformSupport::Both,
                tools: vec![],
                feature_ids: vec![],
                unit: "files".into(),
            }
        }
        fn availability(&self, _: &[ExternalTool]) -> Availability {
            Availability::Available
        }
        fn applies_to(&self, e: &FileEntry) -> bool {
            e.extension() == "bin"
        }
        fn estimate(&self, e: &FileEntry, _: &ParamValues) -> Result<Estimate> {
            if e.name().starts_with("panic") {
                panic!("estimate boom");
            }
            Ok(Estimate { files: 1, bytes_before: e.size, bytes_after: e.size / 2, ..Default::default() })
        }
        fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let name = job.src.file_name().unwrap().to_string_lossy().into_owned();
            if name.starts_with("panic") {
                panic!("hostile input");
            }
            if name.starts_with("fail") {
                return Err(Error::parse("fake", "bad"));
            }
            if name.starts_with("cancel") {
                if let Some(c) = &self.cancel {
                    c.cancel();
                }
            }
            let data = std::fs::read(job.src).map_err(|e| Error::io(job.src, e))?;
            if name.starts_with("skip") {
                return Ok(OptimiseOutput {
                    bytes_before: data.len() as u64,
                    bytes_after: data.len() as u64,
                    lossy: false,
                    skipped: Some("signed".into()),
                    notes: vec![],
                });
            }
            let q = job.params.number("q", 100.0);
            let n = (data.len() as f64 * q / 100.0) as usize;
            std::fs::write(job.dst, &data[..n.min(data.len())]).map_err(|e| Error::io(job.dst, e))?;
            Ok(OptimiseOutput {
                bytes_before: data.len() as u64,
                bytes_after: n as u64,
                lossy: q < 100.0,
                skipped: None,
                notes: vec![],
            })
        }
        fn analyse(&self, _: &DetectContext<'_>) -> Result<DetectorOutput> {
            Ok(DetectorOutput::default())
        }
    }

    fn ctx<'a>(params: &'a ParamValues, work: &'a Path, cancel: &'a CancelToken) -> BatchContext<'a> {
        BatchContext { params, tools: &[], work_dir: work, cancel, progress: &NullProgress }
    }

    #[test]
    fn min_gain_rule() {
        let g = MinGain::default();
        assert!(!g.accepts(1000, 1000));
        assert!(!g.accepts(1000, 1200));
        assert!(!g.accepts(100_000, 99_800), "200 bytes is below 512");
        assert!(!g.accepts(10_000_000, 9_990_000), "0.1% is below 0.5%");
        assert!(g.accepts(100_000, 90_000));
        assert!(MinGain::ANY.accepts(10, 9));
        assert!(g.reject_reason(10, 10).unwrap().contains("not smaller"));
        assert!(g.reject_reason(100_000, 99_900).unwrap().contains("minimum gain"));
    }

    #[test]
    fn light_copy_naming_never_collides() {
        let t = TempDir::new("light").unwrap();
        let orig = t.write("deck.pptx", b"x").unwrap();
        assert_eq!(light_copy_path(&orig, "", |p| p.exists()), t.join("deck_light.pptx"));
        t.write("deck_light.pptx", b"y").unwrap();
        t.write("deck_light (1).pptx", b"y").unwrap();
        assert_eq!(light_copy_path(&orig, "_light", |p| p.exists()), t.join("deck_light (2).pptx"));
        assert_eq!(light_copy_path(Path::new("/d/noext"), "_s", |_| false), PathBuf::from("/d/noext_s"));
        let item = optimise_item(
            &orig,
            t.path(),
            mj_core::FileStamp { size: 1, mtime: mj_core::Timestamp(0) },
            OptimiserId::PowerPoint,
            ParamValues::new(),
            light_copy_mode(),
            OriginalsPolicy::Quarantine,
        );
        assert_eq!(item.action.safety(false), mj_core::SafetyClass::LightCopy);
        match item.action {
            Action::Optimise { keep_if_not_smaller, output: OutputMode::LightCopy { suffix }, .. } => {
                assert!(keep_if_not_smaller);
                assert_eq!(suffix, "_light");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn metadata_is_preserved() {
        let t = TempDir::new("meta").unwrap();
        let a = t.write("a", b"1").unwrap();
        let b = t.write("b", b"2").unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        mj_core::testutil::set_mtime(&a, old).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        preserve_metadata(&a, &b, true, true).unwrap();
        assert_eq!(std::fs::metadata(&b).unwrap().modified().unwrap(), old);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&b).unwrap().permissions().mode() & 0o777, 0o640);
        }
    }

    #[test]
    fn lossy_switch_global_and_per_type() {
        let f = Fake::new();
        let info = f.info();
        let p = ParamValues::new()
            .with("q", ParamValue::Number(50.0))
            .with("strip", ParamValue::Bool(true))
            .with("magic", ParamValue::Bool(true));
        assert!(params_are_lossy(&info, &p));
        assert!(!params_are_lossy(&info, &ParamValues::new()));
        let strict = LossyPolicy { lossless_only: true, per_type: BTreeMap::new() };
        let e = effective_params(&info, &p, &strict);
        assert_eq!(e.number("q", 0.0), 100.0);
        assert!(!e.bool("strip", true));
        assert!(!e.bool("magic", true), "disabled settings are reset");
        assert!(!params_are_lossy(&info, &e));
        assert!(visible_presets(&info, &strict).is_empty());
        let mut per = strict.clone();
        per.per_type.insert(OptimiserId::ImageLossy, true);
        assert_eq!(effective_params(&info, &p, &per).number("q", 0.0), 50.0);
        assert_eq!(visible_presets(&info, &per).len(), 4);
        let open = LossyPolicy { lossless_only: false, per_type: [(OptimiserId::ImageLossy, false)].into() };
        assert!(!open.allows_lossy(OptimiserId::ImageLossy));
        assert!(open.allows_lossy(OptimiserId::Video));
    }

    #[test]
    fn batch_reports_every_file_and_survives_panics() {
        let t = TempDir::new("batch").unwrap();
        let files = vec![
            t.write("good.bin", &[7u8; 10_000]).unwrap(),
            t.write("panic.bin", &[1u8; 100]).unwrap(),
            t.write("fail.bin", &[1u8; 100]).unwrap(),
            t.write("skip.bin", &[1u8; 100]).unwrap(),
            t.write("same.bin", &[1u8; 100]).unwrap(),
            t.write("good2.bin", &[7u8; 20_000]).unwrap(),
        ];
        let work = t.join("work");
        let cancel = CancelToken::new();
        let params = ParamValues::new().with("q", ParamValue::Number(50.0));
        let f = Fake::new();
        let c = ctx(&params, &work, &cancel);
        let r = run_batch(&f, &files, &c, BatchOptions::default());
        assert_eq!(r.results.len(), 6);
        assert!(matches!(r.results[0].outcome, Outcome::Smaller { before: 10_000, after: 5_000, output: None, .. }));
        assert!(
            matches!(&r.results[1].outcome, Outcome::Failed { error: Error::Internal(m) } if m.contains("hostile"))
        );
        assert!(matches!(r.results[2].outcome, Outcome::Failed { .. }));
        assert!(matches!(&r.results[3].outcome, Outcome::Skipped { reason } if reason == "signed"));
        assert!(matches!(r.results[4].outcome, Outcome::Kept { .. }), "100 -> 50 is below the minimum gain");
        assert!(matches!(r.results[5].outcome, Outcome::Smaller { .. }));
        assert_eq!(r.estimate.files, 2);
        assert_eq!(r.estimate.saving(), 15_000);
        assert_eq!(r.estimate.skipped, 4);
        assert_eq!(r.estimate.confidence, Confidence::Exact);
        // Dry run leaves nothing behind but the scratch folder.
        let left: Vec<_> = std::fs::read_dir(&work).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("scratch")]);
        // Apply mode keeps the outputs in the working directory.
        let r = run_batch(&f, &files[..1], &c, BatchOptions { dry_run: false, min_gain: MinGain::default() });
        match &r.results[0].outcome {
            Outcome::Smaller { output: Some(p), .. } => assert_eq!(std::fs::metadata(p).unwrap().len(), 5_000),
            o => panic!("{o:?}"),
        }
        // Estimates: a panicking estimate is per entry.
        let entries = vec![
            FileEntry::new(t.join("a.bin"), mj_core::EntryKind::File, 100),
            FileEntry::new(t.join("panic.bin"), mj_core::EntryKind::File, 100),
        ];
        let (sum, each) = estimate_batch(&f, &entries, &params);
        assert_eq!(sum.saving(), 50);
        assert_eq!(sum.skipped, 1);
        assert!(each[1].is_err());
    }

    #[test]
    fn batch_stops_on_cancel_between_files() {
        let t = TempDir::new("cancel").unwrap();
        let files: Vec<_> =
            ["a.bin", "cancel.bin", "c.bin", "d.bin"].iter().map(|n| t.write(n, &[3u8; 5000]).unwrap()).collect();
        let work = t.join("w");
        let cancel = CancelToken::new();
        let mut f = Fake::new();
        f.cancel = Some(cancel.clone());
        let params = ParamValues::new().with("q", ParamValue::Number(10.0));
        let r = run_batch(&f, &files, &ctx(&params, &work, &cancel), BatchOptions::default());
        assert!(r.cancelled);
        assert_eq!(f.calls.load(Ordering::SeqCst), 2, "the file in progress finishes, the rest never start");
        assert!(matches!(r.results[2].outcome, Outcome::Cancelled));
        assert!(matches!(r.results[3].outcome, Outcome::Cancelled));
        // Pause blocks until resumed.
        let cancel2 = CancelToken::new();
        cancel2.pause();
        let c2 = cancel2.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            c2.resume();
        });
        let f2 = Fake::new();
        let r = run_batch(&f2, &files[..1], &ctx(&params, &work, &cancel2), BatchOptions::default());
        h.join().unwrap();
        assert!(!r.cancelled);
        assert_eq!(f2.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fit_to_target_picks_the_best_rung_that_fits() {
        let t = TempDir::new("fit").unwrap();
        let src = t.write("big.bin", &[9u8; 10_000]).unwrap();
        let work = t.join("w");
        let cancel = CancelToken::new();
        let base = ParamValues::new();
        let c = ctx(&base, &work, &cancel);
        let f = Fake::new();
        let ladder = ladder_from_presets(&f.info(), &base);
        let r = fit_to_target(&f, &src, &ladder, 6_000, &c).unwrap();
        assert!(r.reached);
        assert_eq!(r.chosen, Some((2, "quality 50".into())));
        assert_eq!(r.size, Some(5_000));
        assert_eq!(r.attempts.len(), 3);
        assert_eq!(std::fs::read_dir(&work).unwrap().filter(|e| e.as_ref().unwrap().path().is_file()).count(), 1);
        // Unreachable target: the smallest output is reported.
        let r = fit_to_target(&f, &src, &ladder, 100, &c).unwrap();
        assert!(!r.reached);
        assert_eq!(r.chosen, Some((3, "quality 30".into())));
        assert_eq!(r.size, Some(3_000));
        assert_eq!(r.attempts.len(), 4);
        // A panicking optimiser is reported per attempt.
        let p = t.write("panic.bin", &[1u8; 100]).unwrap();
        let r = fit_to_target(&f, &p, &ladder, 10, &c).unwrap();
        assert!(r.chosen.is_none());
        assert!(r.attempts.iter().all(|(_, a)| a.is_err()));
    }

    #[test]
    fn routing_by_content() {
        let t = TempDir::new("route").unwrap();
        let mut w = mj_formats::zip::ZipWriter::new(Vec::new());
        w.add("[Content_Types].xml", b"<Types/>", 6, 0, 0).unwrap();
        w.add("word/document.xml", b"<w/>", 6, 0, 0).unwrap();
        let docx = w.finish().unwrap();
        let p = t.write("really-a-docx.pptx", &docx).unwrap();
        let r = route(&p).unwrap();
        assert_eq!(r.kind, Some(FileKind::Docx));
        assert_eq!(r.optimiser, Some(OptimiserId::Word));
        assert!(r.mismatch);
        let p = t.write("x.png", b"\x89PNG\r\n\x1a\n0000").unwrap();
        assert_eq!(route(&p).unwrap().optimiser, Some(OptimiserId::PngLossless));
        let p = t.write("x.pptx", b"<html>not a deck</html>").unwrap();
        let r = route(&p).unwrap();
        assert_eq!(r.optimiser, None);
        let mut w = mj_formats::zip::ZipWriter::new(Vec::new());
        w.add("mimetype", b"application/epub+zip", 0, 0, 0).unwrap();
        w.add("META-INF/container.xml", b"<c/>", 6, 0, 0).unwrap();
        assert_eq!(refine_zip(std::io::Cursor::new(w.finish().unwrap())), Some(FileKind::Epub));
    }

    #[test]
    fn breakdown_summary() {
        let mut b = SizeBreakdown::default();
        b.add(BreakdownCategory::Fonts, 2000);
        b.add(BreakdownCategory::Images, 5_000_000);
        b.add(BreakdownCategory::Images, 1);
        b.add(BreakdownCategory::Other, 0);
        assert_eq!(b.total(), 5_002_001);
        assert_eq!(b.sorted()[0].0, BreakdownCategory::Images);
        assert_eq!(b.sorted().len(), 2);
        assert!(b.summary().starts_with("images "));
        assert!(b.summary().contains(" · fonts "));
    }
}
