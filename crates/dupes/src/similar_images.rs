//! Similar images (CLN-DET-02): dHash + pHash from the same decode, BK-tree, Union-Find candidates, leader refinement, diameter confidence (research §4-§7).
//! Owner: W2-B.
//!
//! Pipeline per scan:
//! 1. JPEG and PNG files (by extension, confirmed by signature), local content only.
//! 2. Byte-identical copies are collapsed first (staged exact hashing, cached):
//!    only the first path of an exact group takes part, so nothing is reported
//!    twice (the exact detector reports the copies).
//! 3. Fingerprints come from the cache, else from the pixels, cheapest first:
//!    the EXIF embedded thumbnail found in a bounded 128 KiB header window
//!    (widened to 1 MiB when the IFD points just past it), then, when allowed
//!    (`decode_full`, on by default, D-2B04), a full decode bounded by
//!    [`MAX_FULL_READ`] bytes and `decode::MAX_PIXELS`. Both fingerprints are
//!    always computed and cached with the dimensions, the EXIF summary (only
//!    when really read) and the source flag; the match mode is applied only
//!    when grouping.
//! 4. Files that yield no fingerprint are counted in `Diagnostics::skipped`
//!    (`no_thumbnail`, `decode_failed`, `too_large`, `unsupported`,
//!    `flat_image`): they stay exact-matched only, and the UI says so.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::sync::{Condvar, Mutex};

use mj_core::fingerprint::{ExifSummary, FingerprintFlags};
use mj_core::params::ChoiceOption;
use mj_core::{
    lock, ActionKind, Badges, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Diagnostics,
    FileEntry, Fingerprints, GroupId, GroupKind, KeepRule, PlatformSupport, Result, SafetyClass, SettingKind,
    SettingSpec,
};
use mj_formats::{exif, jpeg};

use crate::decode::{self, png, DecodeError, Decoded};
use crate::keep::{self, GroupSpec, Selection};
use crate::perceptual::{self, Fp, MatchMode, ThresholdPercent};
use crate::staged::{self, adaptive::ReadError};

/// Extensions considered (lower case).
pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "jpe", "jfif", "png"];
/// Header window read for every image (EXIF + thumbnail + SOF / IHDR).
pub const HEADER_WINDOW: u64 = 128 * 1024;
/// Widened window when the thumbnail lies just past the header window.
pub const THUMB_WINDOW: u64 = 1024 * 1024;
/// Largest file decoded in full.
pub const MAX_FULL_READ: u64 = 64 * 1024 * 1024;
/// Images above this many pixels decode one or two at a time (memory bound).
pub const BIG_IMAGE_PIXELS: u64 = 16_000_000;
/// Groups whose diameter is at most this many bits are pre-selected (re-saves,
/// resizes); wider groups are shown for review with nothing selected (D-2B05).
pub const PRESELECT_MAX_DIAMETER: u32 = 3;

/// Parameter keys.
pub const MATCH_MODE: &str = "match_mode";
pub const THRESHOLD: &str = "threshold_percent";
pub const DECODE_FULL: &str = "decode_full";

/// Skip reasons.
pub mod skip {
    pub const NO_THUMBNAIL: &str = "no_thumbnail";
    pub const DECODE_FAILED: &str = "decode_failed";
    pub const TOO_LARGE: &str = "too_large";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const FLAT: &str = "flat_image";
    pub const EXACT_COPY: &str = "exact_copy";
}

/// Similar images: Near-identical photos and re-saved copies
#[derive(Debug, Default, Clone, Copy)]
pub struct SimilarImages;

impl Detector for SimilarImages {
    fn info(&self) -> DetectorInfo {
        let opt = |v: &str, l: &str, h: &str| ChoiceOption { value: v.into(), label: l.into(), help: Some(h.into()) };
        let mut settings = vec![
            SettingSpec {
                key: MATCH_MODE.into(),
                label: "Algorithm".into(),
                help: None,
                kind: SettingKind::Choice {
                    default: "dhash".into(),
                    options: vec![
                        opt("dhash", "dHash", "Gradient hash, the default"),
                        opt("phash", "pHash", "DCT hash, robust to contrast changes"),
                        opt("both", "Both must agree", "Strictest: fewest false matches"),
                    ],
                },
                advanced: false,
                lossy: false,
                disabled_reason: None,
            },
            SettingSpec {
                key: THRESHOLD.into(),
                label: "Similarity threshold".into(),
                help: Some("Percent of the 64-bit hash that may differ (capped at 18 %)".into()),
                kind: SettingKind::Number {
                    default: perceptual::DEFAULT_THRESHOLD_PERCENT as f64,
                    min: 0.0,
                    max: perceptual::MAX_THRESHOLD_PERCENT as f64,
                    step: 1.0,
                    unit: Some("%".into()),
                },
                advanced: false,
                lossy: false,
                disabled_reason: None,
            },
            SettingSpec {
                key: DECODE_FULL.into(),
                label: "Decode images without a thumbnail".into(),
                help: Some("Off: such images are compared byte-for-byte only (faster)".into()),
                kind: SettingKind::Toggle { default: true },
                advanced: true,
                lossy: false,
                disabled_reason: None,
            },
        ];
        settings.extend(keep::settings("highest_resolution", true));
        DetectorInfo {
            id: DetectorId::SimilarImages,
            title: "Similar images".into(),
            description: "Near-identical photos and re-saved copies".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine, ActionKind::Move],
            settings,
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-02".into(), "CLN-ACT-01".into()],
            unit: "images".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let mode = MatchMode::parse(ctx.params.text(MATCH_MODE, "dhash"));
        // The one percent -> bits conversion (research §5.1, invariant 1).
        let radius = ThresholdPercent::new(ctx.params.number(THRESHOLD, perceptual::DEFAULT_THRESHOLD_PERCENT as f64))
            .to_bits();
        let decode_full = ctx.params.bool(DECODE_FULL, true);
        let rule = keep::rule_from_params(ctx.params, KeepRule::HighestResolution);
        let mut diag = Diagnostics::default();

        let images: Vec<&FileEntry> = ctx
            .index
            .files()
            .filter(|e| e.size > 0 && EXTENSIONS.contains(&e.extension().as_str()))
            .filter(|e| {
                if e.is_online_only() {
                    diag.skip(staged::skip::ONLINE_ONLY);
                    false
                } else {
                    true
                }
            })
            .collect();
        let kept = staged::dedup_hardlinks(&images, &mut diag);
        let images: Vec<&FileEntry> = kept.into_iter().map(|i| images[i]).collect();

        // Collapse byte-identical copies to their first path.
        let mut copies = HashSet::new();
        let mut exact_diag = Diagnostics::default();
        for g in staged::exact_groups(ctx, &images, &mut exact_diag)? {
            for &m in &g.members[1..] {
                copies.insert(m);
                diag.skip(skip::EXACT_COPY);
            }
        }
        // Unreadable files are reported once, by the fingerprint stage below.
        diag.notes.extend(exact_diag.notes);
        let images: Vec<&FileEntry> =
            images.iter().enumerate().filter(|(i, _)| !copies.contains(i)).map(|(_, e)| *e).collect();

        let fps = fingerprint_all(ctx, &images, decode_full, &mut diag)?;
        let mut items = Vec::new();
        let mut owners = Vec::new();
        for (i, fp) in fps.iter().enumerate() {
            if let Some(f) = fp {
                let p = Fp { dhash: f.dhash.unwrap_or(0), phash: f.phash.unwrap_or(0) };
                if mode.usable(p) {
                    items.push(p);
                    owners.push(i);
                } else {
                    diag.skip(skip::FLAT);
                }
            }
        }
        ctx.cancel.checkpoint()?;
        let clusters = perceptual::group(&items, mode, radius);

        let mut out = DetectorOutput::default();
        for (n, c) in clusters.iter().enumerate() {
            let idx: Vec<usize> = c.members.iter().map(|&m| owners[m]).collect();
            let members: Vec<&FileEntry> = idx.iter().map(|&i| images[i]).collect();
            let pixels: Vec<Option<u64>> = idx
                .iter()
                .map(|&i| fps[i].as_ref().and_then(|f| Some(f.width? as u64 * f.height? as u64)))
                .collect();
            let keeper = keep::pick(&rule, &members, &pixels);
            let confidence = c.confidence();
            let columns = idx
                .iter()
                .map(|&i| {
                    let f = fps[i].as_ref();
                    let mut m = BTreeMap::new();
                    if let Some(f) = f {
                        if let (Some(w), Some(h)) = (f.width, f.height) {
                            m.insert("resolution".to_string(), format!("{w}x{h}"));
                        }
                        let q = keep::quality_score(f.width, f.height, images[i].size, f.exif.as_ref());
                        m.insert("quality".to_string(), q.to_string());
                    }
                    m.insert("similarity".to_string(), format!("{confidence:.0}%"));
                    m
                })
                .collect();
            let selection =
                if c.diameter <= PRESELECT_MAX_DIAMETER { Selection::AllButKeeper } else { Selection::None };
            let others = members.len() - 1;
            keep::emit(
                GroupSpec {
                    id: GroupId(n as u64 + 1),
                    kind: GroupKind::Similar,
                    source: DetectorId::SimilarImages,
                    label: format!("{} similar images like {}", members.len(), members[keeper].name()),
                    confidence,
                    members,
                    keeper,
                    selection,
                    badges: Badges::DUPLICATE,
                    reason: format!(
                        "Looks like {others} other image{} ({confidence:.0}% similar)",
                        if others == 1 { "" } else { "s" }
                    ),
                    keeper_reason: "Image kept (keep rule)".into(),
                    columns,
                },
                &mut out,
            );
        }
        out.diagnostics = diag;
        Ok(out)
    }
}

/// Fingerprints for every image (cache first), `None` when skipped or unreadable.
pub fn fingerprint_all(
    ctx: &DetectContext<'_>,
    images: &[&FileEntry],
    decode_full: bool,
    diag: &mut Diagnostics,
) -> Result<Vec<Option<Fingerprints>>> {
    let mut out: Vec<Option<Fingerprints>> = vec![None; images.len()];
    let mut need = Vec::new();
    for (i, e) in images.iter().enumerate() {
        match ctx.fingerprints.get(&e.path, e.stamp()) {
            Some(fp) if fp.dhash.is_some() && fp.phash.is_some() => out[i] = Some(fp),
            // A cached decode failure is not retried every scan (research §1.7).
            Some(fp) if fp.flags.contains(FingerprintFlags::DECODE_FAILED) => diag.skip(skip::DECODE_FAILED),
            Some(fp) if fp.flags.contains(FingerprintFlags::NO_THUMBNAIL) && !decode_full => {
                diag.skip(skip::NO_THUMBNAIL)
            }
            _ => need.push(i),
        }
    }
    let gate = Gate::new(2);
    let results = staged::read_all(ctx, &need, images, diag, |e| fingerprint_file(e, decode_full, &gate))?;
    for (&i, r) in need.iter().zip(results) {
        let e = images[i];
        match r {
            Ok(Outcome::Hashed(fp)) => {
                ctx.fingerprints.put(&e.path, e.stamp(), fp.clone());
                out[i] = Some(fp);
            }
            Ok(Outcome::Skipped(reason, fp)) => {
                diag.skip(reason);
                if let Some(fp) = fp {
                    ctx.fingerprints.put(&e.path, e.stamp(), fp);
                }
            }
            Err(err) => staged::report(diag, &e.path, &err),
        }
    }
    Ok(out)
}

/// What fingerprinting one file produced.
pub enum Outcome {
    Hashed(Fingerprints),
    /// Skipped for perceptual matching, with what is worth caching.
    Skipped(&'static str, Option<Fingerprints>),
}

/// Counting gate limiting concurrent decodes of very large images.
struct Gate {
    free: Mutex<usize>,
    cv: Condvar,
}

impl Gate {
    fn new(n: usize) -> Self {
        Gate { free: Mutex::new(n), cv: Condvar::new() }
    }

    fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        {
            let mut free = lock(&self.free);
            while *free == 0 {
                free = self.cv.wait(free).unwrap_or_else(|e| e.into_inner());
            }
            *free -= 1;
        }
        struct Release<'a>(&'a Gate);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                *lock(&self.0.free) += 1;
                self.0.cv.notify_one();
            }
        }
        let _r = Release(self);
        f()
    }
}

fn read_prefix(file: &mut File, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(limit.min(THUMB_WINDOW) as usize);
    file.take(limit).read_to_end(&mut buf)?;
    Ok(buf)
}

fn read_whole(e: &FileEntry) -> std::result::Result<Vec<u8>, ReadError> {
    let mut f = File::open(&e.path)?;
    let mut buf = Vec::with_capacity(e.size as usize);
    (&mut f).take(e.size + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 != e.size {
        return Err(ReadError::Changed);
    }
    Ok(buf)
}

fn summary(x: &exif::Exif) -> ExifSummary {
    ExifSummary {
        date_taken: x.date_taken,
        camera: x.camera(),
        lens: x.lens.clone(),
        has_gps: x.has_gps,
        has_description: x.description.is_some(),
    }
}

fn hashed(d: Decoded, w: Option<u32>, h: Option<u32>, exif: Option<ExifSummary>, thumb: bool) -> Outcome {
    Outcome::Hashed(Fingerprints {
        dhash: Some(d.dhash),
        phash: Some(d.phash),
        width: w.or(Some(d.width)),
        height: h.or(Some(d.height)),
        exif,
        flags: if thumb { FingerprintFlags::FROM_THUMBNAIL } else { FingerprintFlags::empty() },
        ..Default::default()
    })
}

fn failed(e: &DecodeError, w: Option<u32>, h: Option<u32>, exif: Option<ExifSummary>) -> Outcome {
    match e {
        DecodeError::TooLarge => Outcome::Skipped(skip::TOO_LARGE, None),
        DecodeError::Unsupported(_) => Outcome::Skipped(skip::UNSUPPORTED, None),
        DecodeError::Corrupt(_) => Outcome::Skipped(
            skip::DECODE_FAILED,
            Some(Fingerprints { width: w, height: h, exif, flags: FingerprintFlags::DECODE_FAILED, ..Default::default() }),
        ),
    }
}

/// Fingerprint one image file (bounded reads; full decode only when allowed and within limits).
fn fingerprint_file(e: &FileEntry, decode_full: bool, gate: &Gate) -> std::result::Result<Outcome, ReadError> {
    let mut f = staged::open_checked(&e.path, e.size)?;
    let mut head = read_prefix(&mut f, HEADER_WINDOW)?;
    if png::is_png(&head) {
        let Some(h) = png::header(&head) else { return Ok(Outcome::Skipped(skip::DECODE_FAILED, None)) };
        if !decode::within_limits(h.width, h.height) || e.size > MAX_FULL_READ {
            return Ok(Outcome::Skipped(skip::TOO_LARGE, None));
        }
        if h.interlaced {
            return Ok(Outcome::Skipped(skip::UNSUPPORTED, None));
        }
        let data = if e.size as usize == head.len() { head } else { read_whole(e)? };
        let big = h.width as u64 * h.height as u64 > BIG_IMAGE_PIXELS;
        let r = if big { gate.run(|| png::decode(&data)) } else { png::decode(&data) };
        return Ok(match r {
            Ok(d) => hashed(d, None, None, None, false),
            Err(err) => failed(&err, Some(h.width), Some(h.height), None),
        });
    }
    let Some(hdr) = jpeg::walk(&head) else { return Ok(Outcome::Skipped(skip::UNSUPPORTED, None)) };
    let (w, h) = hdr.frame.map(|fr| (Some(fr.width as u32), Some(fr.height as u32))).unwrap_or((None, None));

    // EXIF from the bytes already in memory; widen once if the thumbnail lies just past the window.
    let mut parsed = hdr.exif.clone().and_then(|r| head.get(r.clone()).map(|t| (r.start, exif::parse(t).ok())));
    if let Some((start, Some(x))) = &parsed {
        if let Some(t) = x.thumbnail {
            let end = start + t.offset + t.len;
            if end > head.len() && (end as u64) <= THUMB_WINDOW && head.len() as u64 == HEADER_WINDOW {
                drop(f);
                let mut f2 = File::open(&e.path)?;
                head = read_prefix(&mut f2, THUMB_WINDOW)?;
                parsed = jpeg::walk(&head)
                    .and_then(|h2| h2.exif)
                    .and_then(|r| head.get(r.clone()).map(|t| (r.start, exif::parse(t).ok())));
            }
        }
    }
    let exif_summary = match &parsed {
        Some((_, Some(x))) => Some(summary(x)),
        // Walked to the image data without an EXIF segment: EXIF really is absent.
        None if hdr.reached_scan => Some(ExifSummary::default()),
        _ => None,
    };
    if let Some((start, Some(x))) = &parsed {
        if let Some(thumb) = head.get(*start..).and_then(|tiff| x.thumbnail_bytes(tiff)) {
            if let Ok(d) = decode::jpeg(thumb) {
                if d.dhash != 0 {
                    return Ok(hashed(d, w.or(x.width), h.or(x.height), exif_summary, true));
                }
            }
        }
    }
    if !decode_full {
        let fp = Fingerprints { width: w, height: h, exif: exif_summary, flags: FingerprintFlags::NO_THUMBNAIL, ..Default::default() };
        return Ok(Outcome::Skipped(skip::NO_THUMBNAIL, Some(fp)));
    }
    if e.size > MAX_FULL_READ {
        return Ok(Outcome::Skipped(skip::TOO_LARGE, None));
    }
    if let (Some(ww), Some(hh)) = (w, h) {
        if !decode::within_limits(ww, hh) {
            return Ok(Outcome::Skipped(skip::TOO_LARGE, None));
        }
    }
    let data = if e.size as usize == head.len() { head } else { read_whole(e)? };
    let big = w.zip(h).is_some_and(|(a, b)| a as u64 * b as u64 > BIG_IMAGE_PIXELS);
    let r = if big { gate.run(|| decode::jpeg(&data)) } else { decode::jpeg(&data) };
    Ok(match r {
        Ok(d) => hashed(d, w, h, exif_summary, false),
        Err(err) => failed(&err, w, h, exif_summary),
    })
}
