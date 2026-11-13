//! JPEG lossless optimisation (OPT-IMG-01) and metadata control (OPT-IMG-05): segment-level rewrite
//! in-house (scan data untouched, verified), plus `jpegtran` Huffman optimisation / progressive when
//! it is on PATH, with its output checked against the in-house result.
//! Owner: W3-C.

use mj_core::{
    ActionKind, Availability, Category, Confidence, DetectContext, DetectorOutput, Error, Estimate, ExternalTool,
    FileEntry, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass, SettingSpec, StepPreset,
};

use super::adapters::jpegtran_args;
use super::common::{self, spec};
use super::jpeg_rewrite::{header_signature, rewrite, JpegMetadata};
use crate::external::{run, RunOptions};

/// JPEG images: Lossless optimisation and metadata control
#[derive(Debug, Default, Clone, Copy)]
pub struct JpegLossless;

/// Extensions (lower case) this optimiser targets.
pub const JPEGLOSSLESS_EXTENSIONS: &[&str] = &["jpg", "jpeg"];

const MIN_SIZE: u64 = 8 * 1024;
/// Header window read for estimates.
const PREFIX: usize = 256 * 1024;

/// Metadata policy from the parameters.
pub fn policy(p: &ParamValues) -> JpegMetadata {
    JpegMetadata::parse(p.text("metadata", "strip_thumbnail"), p.bool("strip_icc", false))
}

fn settings() -> Vec<SettingSpec> {
    vec![
        spec::choice(
            "metadata",
            "Metadata",
            "strip_thumbnail",
            &[
                ("keep_all", "Keep everything", None),
                ("strip_gps", "Remove GPS position only", Some("Also removes XMP packets that mention GPS")),
                ("strip_thumbnail", "Remove embedded thumbnails", Some("Previews inside the file; the photo is untouched")),
                ("strip_all", "Remove all metadata", Some("Keeps orientation and colour profile")),
            ],
            false,
        ),
        spec::toggle("strip_icc", "Also remove colour profile", Some("Colours can look different afterwards"), false, true, true),
        spec::toggle("optimise_huffman", "Optimise Huffman tables (jpegtran)", Some("Needs jpegtran on PATH; pixels stay identical"), true, true, false),
        spec::toggle("progressive", "Make progressive (jpegtran)", Some("Usually smaller for large photos"), true, true, false),
    ]
}

fn presets() -> Vec<StepPreset> {
    vec![
        spec::preset("safe", "Safe", "Drop embedded thumbnails · pixel-identical", false, &[("metadata", spec::t("strip_thumbnail"))]),
        spec::preset("privacy", "Privacy", "Remove GPS position · pixel-identical", false, &[("metadata", spec::t("strip_gps"))]),
        spec::preset("smallest", "Smallest", "Drop all metadata · pixel-identical", false, &[("metadata", spec::t("strip_all"))]),
    ]
}

fn ends_with_eoi(b: &[u8]) -> bool {
    let end = b.len() - b.iter().rev().take_while(|&&x| x == 0).count();
    b[..end].ends_with(&[0xFF, 0xD9])
}

impl Optimiser for JpegLossless {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::JpegLossless,
            title: "JPEG images".into(),
            description: "Lossless optimisation and metadata control".into(),
            category: Category::Optimise,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: presets(),
            settings: settings(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-IMG-01".into(), "OPT-IMG-05".into(), "OPT-IMG-06".into()],
            unit: "images".into(),
        }
    }

    fn availability(&self, _tools: &[ExternalTool]) -> Availability {
        Availability::Available // jpegtran is optional
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && JPEGLOSSLESS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let prefix = mj_formats::sniff::read_prefix(&entry.path, PREFIX).map_err(|e| Error::io(&entry.path, e))?;
        let saved = rewrite(&prefix, policy(params)).map_or(0, |r| prefix.len().saturating_sub(r.bytes.len())) as u64;
        // Multi-picture files are refused by the rewrite; they have no estimate.
        if matches!(rewrite(&prefix, JpegMetadata::KeepAll), Err(Error::Unsupported { .. })) {
            return Err(Error::unsupported("JPEG", "multi-picture JPEG"));
        }
        Ok(Estimate { files: 1, bytes_before: entry.size, bytes_after: entry.size.saturating_sub(saved), confidence: Confidence::Estimated, skipped: 0 })
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        job.cancel.checkpoint()?;
        let src = common::read_bounded(job.src, common::MAX_IN_MEMORY)?;
        let before = src.len() as u64;
        let r = match rewrite(&src, policy(job.params)) {
            Ok(r) => r,
            Err(Error::Unsupported { reason, .. }) => return Ok(common::skipped(before, reason)),
            Err(e) => return Err(e),
        };
        let (mut best, mut notes) = (r.bytes, r.notes);
        job.cancel.checkpoint()?;
        if let (true, Ok(exe)) = (job.params.bool("optimise_huffman", true), crate::tools::require(job.tools, "jpegtran")) {
            let (stage1, stage2) = (job.temp_dir.join("stage1.jpg"), job.temp_dir.join("stage2.jpg"));
            std::fs::write(&stage1, &best).map_err(|e| Error::io(&stage1, e))?;
            let args = jpegtran_args(&stage1, &stage2, job.params.bool("progressive", true));
            let o = run(exe, &args, &RunOptions { cancel: Some(job.cancel), cwd: Some(job.temp_dir), ..Default::default() })?;
            if o.success() {
                let cand = std::fs::read(&stage2).map_err(|e| Error::io(&stage2, e))?;
                // Same image, same kept markers, complete stream: otherwise the tool's output is ignored.
                let same = header_signature(&cand).zip(header_signature(&best)).is_some_and(|((fa, sa), (fb, sb))| {
                    (fa.width, fa.height, fa.components, fa.precision) == (fb.width, fb.height, fb.components, fb.precision) && sa == sb
                });
                if same && ends_with_eoi(&cand) && cand.len() < best.len() {
                    best = cand;
                    notes.push("jpegtran optimised the entropy coding".into());
                }
            }
            let _ = std::fs::remove_file(&stage1);
            let _ = std::fs::remove_file(&stage2);
        }
        // Removing a GPS position zeroes bytes in place, so the file may keep its size; that is still
        // the point of the run (privacy), so an equal-size, changed file is written for it.
        let privacy_rewrite = policy(job.params) == JpegMetadata::StripGps && best.len() as u64 == before && best != src;
        if best.len() as u64 > before || (best.len() as u64 == before && !privacy_rewrite) {
            return Ok(common::skipped(before, "Nothing to remove or optimise"));
        }
        common::write_new(job.dst, &best)?;
        Ok(common::done(before, best.len() as u64, r.icc_removed, notes))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        common::analyse_entries(
            ctx,
            OptimiserId::JpegLossless,
            MIN_SIZE,
            &|e| self.applies_to(e),
            &|e, p| self.estimate(e, p),
            &|_, _| common::Described { reason: "Metadata can be removed without touching the picture".into(), columns: Default::default(), sub_group: None },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::{CancelToken, NullProgress};
    use mj_formats::exif::build::{app1, tiff, Value};

    fn seg(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    fn photo() -> Vec<u8> {
        let thumb = [vec![0xFF, 0xD8, 0xFF, 0xDB, 0, 4, 1, 2], vec![9; 3000], vec![0xFF, 0xD9]].concat();
        let t = tiff(&[(0x0112, Value::Short(6)), (0x8825, Value::SubIfd(vec![(1, Value::Ascii("N".into())), (2, Value::Rational(vec![(1, 1), (2, 1), (3, 1)]))]))], Some(&[(0, Value::Thumbnail(thumb))]));
        let mut f = vec![0xFF, 0xD8];
        f.extend(app1(&t));
        f.extend(seg(0xDB, &[0; 65]));
        f.extend(seg(0xC0, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        f.extend(seg(0xC4, &[0; 20]));
        f.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
        f.extend(vec![0x55u8; 9000]);
        f.extend([0xFF, 0xD9]);
        f
    }

    fn run_job(src: &std::path::Path, dst: &std::path::Path, d: &TempDir, p: &ParamValues, tools: &[ExternalTool]) -> OptimiseOutput {
        let c = CancelToken::new();
        JpegLossless.optimise(&OptimiseJob { src, dst, params: p, tools, temp_dir: d.path(), cancel: &c, progress: &NullProgress }).unwrap()
    }

    #[test]
    fn thumbnail_strip_keeps_scan_bytes_identical() {
        let d = TempDir::new("jpg-opt").unwrap();
        let src = d.write("a.jpg", &photo()).unwrap();
        let dst = d.join("o.jpg");
        let r = run_job(&src, &dst, &d, &ParamValues::new(), &[]);
        assert!(r.skipped.is_none() && !r.lossy && r.bytes_after + 3000 < r.bytes_before, "{r:?}");
        let (a, b) = (std::fs::read(&src).unwrap(), std::fs::read(&dst).unwrap());
        let scan = |v: &[u8]| v[v.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap()..].to_vec();
        assert_eq!(scan(&a), scan(&b));
        let est = JpegLossless.estimate(&FileEntry::new(&src, mj_core::EntryKind::File, a.len() as u64), &ParamValues::new()).unwrap();
        assert!(est.bytes_after + 3000 < est.bytes_before);
    }

    #[test]
    fn gps_only_strip() {
        let d = TempDir::new("jpg-gps").unwrap();
        let src = d.write("a.jpg", &photo()).unwrap();
        let dst = d.join("o.jpg");
        run_job(&src, &dst, &d, &ParamValues::new().with("metadata", spec::t("strip_gps")), &[]);
        let e = mj_formats::exif::parse(mj_formats::jpeg::exif_tiff(&std::fs::read(&dst).unwrap()).unwrap()).unwrap();
        assert!(!e.has_gps && e.thumbnail.is_some() && e.orientation == Some(6));
    }

    #[test]
    fn keep_all_on_a_clean_file_is_skipped_and_writes_nothing() {
        let d = TempDir::new("jpg-skip").unwrap();
        let src = d.write("a.jpg", &photo()).unwrap();
        let dst = d.join("o.jpg");
        let r = run_job(&src, &dst, &d, &ParamValues::new().with("metadata", spec::t("keep_all")), &[]);
        assert!(r.skipped.is_some() && !dst.exists());
    }

    #[cfg(unix)]
    #[test]
    fn jpegtran_output_is_accepted_only_when_headers_match() {
        use std::os::unix::fs::PermissionsExt;
        let d = TempDir::new("jpg-tran").unwrap();
        let src = d.write("a.jpg", &photo()).unwrap();
        let mk = |body: &str| {
            let exe = d.join("jpegtran");
            std::fs::write(&exe, format!("#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = \"-outfile\" ]; then out=\"$2\"; fi; in=\"$1\"; shift; done\n{body}\n")).unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            vec![crate::tools::tool_at("jpegtran", exe)]
        };
        // Good fake: same file minus 500 scan bytes (still ends in EOI).
        let tools = mk("head -c $(( $(wc -c < \"$in\") - 502 )) \"$in\" > \"$out\"; printf '\\377\\331' >> \"$out\"");
        let plain = run_job(&src, &d.join("p.jpg"), &d, &ParamValues::new().with("optimise_huffman", spec::b(false)), &tools);
        let with = run_job(&src, &d.join("w.jpg"), &d, &ParamValues::new(), &tools);
        assert_eq!(with.bytes_after + 500, plain.bytes_after);
        assert!(with.notes.iter().any(|n| n.contains("jpegtran")));
        // Bad fake: drops the frame header; must be ignored.
        let tools = mk("printf '\\377\\330\\377\\331' > \"$out\"");
        let bad = run_job(&src, &d.join("b.jpg"), &d, &ParamValues::new(), &tools);
        assert_eq!(bad.bytes_after, plain.bytes_after);
    }

    #[test]
    fn mpf_corrupt_and_missing_files() {
        let d = TempDir::new("jpg-edge").unwrap();
        let mut mpf = vec![0xFF, 0xD8];
        mpf.extend(seg(0xE2, b"MPF\0abcdefgh"));
        mpf.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
        mpf.extend([1, 2, 0xFF, 0xD9]);
        let src = d.write("m.jpg", &mpf).unwrap();
        let r = run_job(&src, &d.join("o.jpg"), &d, &ParamValues::new(), &[]);
        assert!(r.skipped.is_some());
        let bad = d.write("b.jpg", b"not a jpeg at all").unwrap();
        let c = CancelToken::new();
        let p = ParamValues::new();
        let dst = d.join("o2.jpg");
        let res = JpegLossless.optimise(&OptimiseJob { src: &bad, dst: &dst, params: &p, tools: &[], temp_dir: d.path(), cancel: &c, progress: &NullProgress });
        assert!(res.is_err() && !dst.exists());
    }
}
