//! PNG lossless recompression (OPT-IMG-01) and metadata control (OPT-IMG-05): in-house re-deflate of
//! the image data (`mj_formats::png::recompress`, pixel-verified), and `oxipng` as a stronger engine
//! when it is on PATH, verified by decoding both files and comparing every pixel.
//! Owner: W3-C.

use mj_core::{
    ActionKind, Availability, Category, Confidence, DetectContext, DetectorOutput, Error, Estimate, ExternalTool,
    FileEntry, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass, SettingSpec, StepPreset,
};
use mj_formats::png::{self, MetadataPolicy};

use super::adapters::oxipng_args;
use super::common::{self, spec};
use crate::external::{run, RunOptions};

/// PNG images: Lossless recompression
#[derive(Debug, Default, Clone, Copy)]
pub struct PngLossless;

/// Extensions (lower case) this optimiser targets.
pub const PNGLOSSLESS_EXTENSIONS: &[&str] = &["png"];

/// Files smaller than this are not worth a candidate row.
const MIN_SIZE: u64 = 4 * 1024;

/// Read the metadata policy from the parameters.
pub fn policy(p: &ParamValues) -> MetadataPolicy {
    match p.text("metadata", "keep_all") {
        "strip_text" => MetadataPolicy::StripText,
        "strip_all" => MetadataPolicy::StripAll { colour: p.bool("strip_colour", false) },
        _ => MetadataPolicy::KeepAll,
    }
}

fn settings() -> Vec<SettingSpec> {
    vec![
        spec::choice(
            "metadata",
            "Metadata",
            "keep_all",
            &[
                ("keep_all", "Keep everything", Some("Only the image data is recompressed")),
                ("strip_text", "Remove text notes", Some("Drops tEXt, zTXt and iTXt chunks")),
                ("strip_all", "Remove all metadata", Some("Keeps transparency, colour profile, gamma and pixel size")),
            ],
            false,
        ),
        spec::toggle("strip_colour", "Also remove colour profile and gamma", Some("Colours can look different afterwards"), false, true, true),
        spec::toggle("refilter", "Try adaptive row filters", Some("Slower, usually smaller"), true, true, false),
        spec::toggle("use_oxipng", "Use oxipng when installed", Some("Stronger recompression, checked pixel by pixel"), true, true, false),
    ]
}

fn presets() -> Vec<StepPreset> {
    vec![
        spec::preset("safe", "Safe", "Keep metadata · pixel-identical", false, &[("metadata", spec::t("keep_all"))]),
        spec::preset("tidy", "Tidy", "Drop text notes · pixel-identical", false, &[("metadata", spec::t("strip_text"))]),
        spec::preset("smallest", "Smallest", "Drop all metadata · pixel-identical", false, &[("metadata", spec::t("strip_all"))]),
    ]
}

/// Compare the decoded pixels of two PNG byte buffers.
fn same_pixels(a: &[u8], b: &[u8]) -> bool {
    matches!((png::decode_rgba16(a), png::decode_rgba16(b)), (Ok(x), Ok(y)) if x == y)
}

impl Optimiser for PngLossless {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::PngLossless,
            title: "PNG images".into(),
            description: "Lossless recompression".into(),
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
        Availability::Available // the in-house engine needs no tool; oxipng is a bonus
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && PNGLOSSLESS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let mut f = std::fs::File::open(&entry.path).map_err(|e| Error::io(&entry.path, e))?;
        let chunks = png::summarise(&mut f, entry.size)?;
        let pol = policy(params);
        let removed: u64 = chunks.iter().filter(|(k, _)| png::chunk_removed(k, pol)).map(|(_, l)| l + 12).sum();
        let idat: u64 = chunks.iter().filter(|(k, _)| k == b"IDAT").map(|(_, l)| *l).sum();
        let gain = idat as f64 * if params.bool("refilter", true) { 0.08 } else { 0.03 };
        let after = (entry.size as f64 - removed as f64 - gain).max(0.0) as u64;
        Ok(Estimate { files: 1, bytes_before: entry.size, bytes_after: after.min(entry.size), confidence: Confidence::Estimated, skipped: 0 })
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        job.cancel.checkpoint()?;
        let src = common::read_bounded(job.src, common::MAX_IN_MEMORY)?;
        let before = src.len() as u64;
        let pol = policy(job.params);
        let level = 10;
        let mut best = match png::recompress(&src, pol, level, job.params.bool("refilter", true)) {
            Ok(r) => r,
            Err(Error::Unsupported { reason, .. }) => return Ok(common::skipped(before, reason)),
            Err(e) => return Err(e),
        };
        let mut notes = Vec::new();
        if !best.removed.is_empty() {
            notes.push(format!("removed {}", best.removed.join(", ")));
        }
        job.cancel.checkpoint()?;
        if let (true, Ok(exe)) = (job.params.bool("use_oxipng", true), crate::tools::require(job.tools, "oxipng")) {
            let out = job.temp_dir.join("oxipng-out.png");
            let o = run(exe, &oxipng_args(job.src, &out, pol, 4), &RunOptions { cancel: Some(job.cancel), cwd: Some(job.temp_dir), ..Default::default() })?;
            // oxipng writes nothing when it cannot improve the file: that is not a failure.
            if o.success() && out.is_file() {
                let cand = std::fs::read(&out).map_err(|e| Error::io(&out, e))?;
                if cand.len() < best.bytes.len() && same_pixels(&src, &cand) {
                    best.bytes = cand;
                    notes.push("oxipng produced the smaller file (pixels verified)".into());
                }
            }
            let _ = std::fs::remove_file(&out);
        }
        if best.bytes.len() as u64 >= before {
            return Ok(common::skipped(before, "Recompression would not make the file smaller"));
        }
        common::write_new(job.dst, &best.bytes)?;
        Ok(common::done(before, best.bytes.len() as u64, best.colour_changed, notes))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        common::analyse_entries(
            ctx,
            OptimiserId::PngLossless,
            MIN_SIZE,
            &|e| self.applies_to(e),
            &|e, p| self.estimate(e, p),
            &|_, _| common::Described { reason: "PNG can be recompressed without changing a pixel".into(), columns: Default::default(), sub_group: None },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::{CancelToken, NullProgress};
    use mj_formats::png::build::{chunk, rgb_png};

    fn job<'a>(src: &'a std::path::Path, dst: &'a std::path::Path, tmp: &'a std::path::Path, params: &'a ParamValues, tools: &'a [ExternalTool], c: &'a CancelToken) -> OptimiseJob<'a> {
        OptimiseJob { src, dst, params, tools, temp_dir: tmp, cancel: c, progress: &NullProgress }
    }

    #[test]
    fn optimises_verifies_and_reports() {
        let d = TempDir::new("png-opt").unwrap();
        let src = d.write("a.png", &rgb_png(96, 64, &[chunk(b"tEXt", b"k\0v"), chunk(b"gAMA", &[0, 0, 1, 0])], 2)).unwrap();
        let dst = d.join("out.png");
        let p = ParamValues::new().with("metadata", spec::t("strip_text"));
        let c = CancelToken::new();
        let r = PngLossless.optimise(&job(&src, &dst, d.path(), &p, &[], &c)).unwrap();
        assert!(r.skipped.is_none() && !r.lossy && r.bytes_after < r.bytes_before);
        assert_eq!(r.bytes_after, std::fs::metadata(&dst).unwrap().len());
        assert!(same_pixels(&std::fs::read(&src).unwrap(), &std::fs::read(&dst).unwrap()));
        let out = png::parse(&std::fs::read(&dst).unwrap()).unwrap();
        assert!(out.chunks.iter().any(|c| &c.kind == b"gAMA") && !out.chunks.iter().any(|c| &c.kind == b"tEXt"));
        // The estimate sees the text chunk and the deflate gain.
        let e = FileEntry::new(&src, mj_core::EntryKind::File, std::fs::metadata(&src).unwrap().len());
        let est = PngLossless.estimate(&e, &p).unwrap();
        assert!(est.bytes_after < est.bytes_before && est.confidence == Confidence::Estimated);
    }

    #[test]
    fn apng_and_corrupt_files() {
        let d = TempDir::new("png-opt2").unwrap();
        let c = CancelToken::new();
        let p = ParamValues::new();
        let apng = d.write("an.png", &rgb_png(16, 16, &[chunk(b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0])], 1)).unwrap();
        let dst = d.join("o1.png");
        let r = PngLossless.optimise(&job(&apng, &dst, d.path(), &p, &[], &c)).unwrap();
        assert!(r.skipped.unwrap().contains("APNG") && !dst.exists());
        let bad = d.write("bad.png", b"\x89PNG\r\n\x1a\ngarbage garbage").unwrap();
        let dst2 = d.join("o2.png");
        assert!(PngLossless.optimise(&job(&bad, &dst2, d.path(), &p, &[], &c)).is_err() && !dst2.exists());
    }

    #[test]
    fn already_small_png_is_skipped() {
        let d = TempDir::new("png-opt3").unwrap();
        let first = {
            let src = d.write("a.png", &rgb_png(32, 32, &[], 1)).unwrap();
            let dst = d.join("b.png");
            let c = CancelToken::new();
            PngLossless.optimise(&job(&src, &dst, d.path(), &ParamValues::new(), &[], &c)).unwrap();
            dst
        };
        let dst = d.join("c.png");
        let c = CancelToken::new();
        let r = PngLossless.optimise(&job(&first, &dst, d.path(), &ParamValues::new(), &[], &c)).unwrap();
        assert!(r.skipped.is_some() && !dst.exists());
    }

    #[test]
    fn strip_colour_marks_the_output_lossy() {
        let d = TempDir::new("png-opt4").unwrap();
        let src = d.write("a.png", &rgb_png(32, 32, &[chunk(b"iCCP", b"p\0\0xxxxxxxx")], 1)).unwrap();
        let p = ParamValues::new().with("metadata", spec::t("strip_all")).with("strip_colour", spec::b(true));
        let dst = d.join("o.png");
        let r = PngLossless.optimise(&job(&src, &dst, d.path(), &p, &[], &CancelToken::new())).unwrap();
        assert!(r.lossy);
    }

    #[cfg(unix)]
    #[test]
    fn oxipng_adapter_runs_and_its_output_is_verified() {
        use std::os::unix::fs::PermissionsExt;
        let d = TempDir::new("png-oxi").unwrap();
        let src = d.write("a.png", &rgb_png(64, 64, &[], 1)).unwrap();
        // A fake oxipng that "optimises" by writing a valid, smaller, pixel-identical PNG.
        let good = png::recompress(&std::fs::read(&src).unwrap(), MetadataPolicy::KeepAll, 10, true).unwrap().bytes;
        let better = d.write("better.png", &good).unwrap();
        let script = format!("#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = \"--out\" ]; then out=\"$2\"; fi; shift; done\ncp '{}' \"$out\"\n", better.display());
        let exe = d.write("oxipng", script.as_bytes()).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tools = vec![crate::tools::tool_at("oxipng", &exe)];
        // Make the in-house result worse than the fake's by disabling re-filter and using a worse source... both are
        // equal here, so only check the flow: output exists, pixels identical.
        let dst = d.join("o.png");
        let c = CancelToken::new();
        let r = PngLossless.optimise(&job(&src, &dst, d.path(), &ParamValues::new(), &tools, &c)).unwrap();
        assert!(r.skipped.is_none());
        assert!(same_pixels(&std::fs::read(&src).unwrap(), &std::fs::read(&dst).unwrap()));
        // A fake that emits a different picture must be rejected.
        d.write("evil.png", &rgb_png(1, 1, &[], 1)).unwrap();
        let script = format!("#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = \"--out\" ]; then out=\"$2\"; fi; shift; done\ncp '{}' \"$out\"\n", d.join("evil.png").display());
        std::fs::write(&exe, script).unwrap();
        let dst2 = d.join("o2.png");
        let r2 = PngLossless.optimise(&job(&src, &dst2, d.path(), &ParamValues::new(), &tools, &c)).unwrap();
        let out2 = std::fs::read(&dst2).unwrap();
        assert!(same_pixels(&std::fs::read(&src).unwrap(), &out2) && r2.notes.iter().all(|n| !n.contains("oxipng")));
    }
}
