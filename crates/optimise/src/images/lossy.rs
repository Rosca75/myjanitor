//! Lossy image recompression, conversion and resize (OPT-IMG-02/03/04) through external encoders
//! (cwebp, avifenc, cjxl). Every run builds an exact argument vector ([`super::adapters`]), encodes
//! into the scratch directory and moves the checked result to `dst`.
//!
//! The output has a different file type from the source (`.webp`, `.avif`, `.jxl`): the new extension
//! is reported in `notes` as `output_extension=<ext>` so the action engine can name the result.
//! Owner: W3-C.

use std::io::Read;
use std::path::Path;

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass, SettingSpec, StepPreset,
};

use super::adapters::{encoder_args, resize_target, EncodeSpec, Target};
use super::common::{self, spec};
use crate::external::{run, RunOptions};

/// Photos (lossy): Re-encode to WebP, AVIF or JPEG XL
#[derive(Debug, Default, Clone, Copy)]
pub struct ImageLossy;

/// Extensions (lower case) this optimiser targets.
pub const IMAGELOSSY_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

const MIN_SIZE: u64 = 64 * 1024;

fn settings() -> Vec<SettingSpec> {
    vec![
        spec::choice(
            "format",
            "Format",
            "webp",
            &[
                ("webp", "WebP", Some("Opens almost everywhere; needs cwebp")),
                ("avif", "AVIF", Some("Smallest; newer apps only; needs avifenc")),
                ("jxl", "JPEG XL", Some("A JPEG converts back bit for bit; few apps open it yet; needs cjxl")),
            ],
            false,
        ),
        spec::number("quality", "Quality", 80.0, 1.0, 100.0, 1.0, None, false),
        spec::toggle(
            "lossless",
            "Keep every pixel (lossless)",
            Some("JPEG to JPEG XL is reversible; other conversions are lossless but larger"),
            false,
            false,
            false,
        ),
        spec::number("max_edge", "Shrink pictures larger than", 0.0, 0.0, 12000.0, 100.0, Some("px (0 = off)"), false),
        spec::choice(
            "edge",
            "Measured on the",
            "long",
            &[("long", "Long edge", None), ("short", "Short edge", None)],
            true,
        ),
        spec::toggle(
            "strip_metadata",
            "Remove EXIF and XMP",
            Some("A colour profile is always kept"),
            false,
            true,
            false,
        ),
    ]
}

fn presets() -> Vec<StepPreset> {
    vec![
        spec::preset(
            "balanced",
            "Balanced",
            "WebP quality 80 · looks the same",
            true,
            &[("format", spec::t("webp")), ("quality", spec::n(80.0)), ("lossless", spec::b(false))],
        ),
        spec::preset(
            "smallest",
            "Smallest",
            "AVIF quality 60",
            true,
            &[("format", spec::t("avif")), ("quality", spec::n(60.0)), ("lossless", spec::b(false))],
        ),
        spec::preset(
            "reversible",
            "Reversible",
            "JPEG XL from JPEG · no quality loss",
            false,
            &[("format", spec::t("jxl")), ("lossless", spec::b(true))],
        ),
    ]
}

/// Pixel size from the file header (PNG IHDR or JPEG SOF).
pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.by_ref().take(256 * 1024).read_to_end(&mut buf).ok()?;
    if buf.starts_with(&mj_formats::png::SIGNATURE) {
        let h = mj_formats::png::Ihdr::parse(buf.get(16..29)?).ok()?;
        return Some((h.width, h.height));
    }
    let fr = mj_formats::jpeg::walk(&buf)?.frame?;
    Some((u32::from(fr.width), u32::from(fr.height)))
}

fn is_jpeg_name(p: &Path) -> bool {
    matches!(mj_core::pathutil::extension_lower(p).as_str(), "jpg" | "jpeg")
}

/// Read the encode settings; `None` when the format parameter is unknown.
fn encode_spec(p: &ParamValues, src: &Path) -> Option<EncodeSpec> {
    let target = Target::parse(p.text("format", "webp"))?;
    let dims = dimensions(src);
    let resize = dims.and_then(|(w, h)| {
        resize_target(w, h, p.number("max_edge", 0.0).max(0.0) as u32, p.text("edge", "long") == "long")
    });
    Some(EncodeSpec {
        target,
        quality: p.number("quality", 80.0).clamp(1.0, 100.0) as u8,
        lossless: p.bool("lossless", false),
        input_is_jpeg: is_jpeg_name(src),
        strip_metadata: p.bool("strip_metadata", false),
        resize,
        effort: 6,
    })
}

fn magic_ok(t: Target, head: &[u8]) -> bool {
    match t {
        Target::Webp => head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP",
        Target::Avif => {
            head.len() >= 12 && &head[4..8] == b"ftyp" && matches!(&head[8..12], b"avif" | b"avis" | b"mif1")
        }
        Target::Jxl => head.starts_with(&[0xFF, 0x0A]) || head.starts_with(b"\0\0\0\x0cJXL "),
    }
}

/// Typical size ratio of the output to the source (estimated, never measured).
fn ratio(s: &EncodeSpec, jpeg_in: bool) -> f64 {
    let q = f64::from(s.quality);
    let r = match (s.target, jpeg_in, s.lossless) {
        (Target::Jxl, true, true) => 0.78,
        (Target::Webp, true, false) => 0.72 + (q - 80.0) * 0.006,
        (Target::Avif, true, false) => 0.50 + (q - 60.0) * 0.006,
        (Target::Jxl, true, false) => 0.55 + (q - 80.0) * 0.006,
        (Target::Webp, false, false) => 0.28 + (q - 80.0) * 0.004,
        (Target::Avif, false, false) => 0.15 + (q - 60.0) * 0.004,
        (Target::Jxl, false, false) => 0.22 + (q - 80.0) * 0.004,
        (Target::Webp, false, true) => 0.75,
        (Target::Avif, false, true) => 0.70,
        (Target::Jxl, false, true) => 0.60,
        (_, true, true) => 1.3, // lossless re-encode of a JPEG's pixels grows
    };
    r.clamp(0.03, 1.5)
}

impl Optimiser for ImageLossy {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ImageLossy,
            title: "Photos (lossy)".into(),
            description: "Re-encode to WebP, AVIF or JPEG XL".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: presets(),
            settings: settings(),
            platform: PlatformSupport::Both,
            tools: vec!["cwebp".into(), "avifenc".into(), "cjxl".into()],
            feature_ids: vec!["OPT-IMG-02".into(), "OPT-IMG-03".into(), "OPT-IMG-04".into()],
            unit: "images".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        crate::tools::availability_any(tools, &["cwebp", "avifenc", "cjxl"])
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && IMAGELOSSY_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let s = encode_spec(params, &entry.path).ok_or_else(|| Error::InvalidInput("unknown image format".into()))?;
        let mut r = ratio(&s, is_jpeg_name(&entry.path));
        if let (Some((nw, nh)), Some((w, h))) = (s.resize, dimensions(&entry.path)) {
            r *= (f64::from(nw) * f64::from(nh)) / (f64::from(w) * f64::from(h));
        }
        Ok(common::ratio_estimate(entry, r))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        job.cancel.checkpoint()?;
        let before = std::fs::metadata(job.src).map_err(|e| Error::io(job.src, e))?.len();
        let s = encode_spec(job.params, job.src).ok_or_else(|| Error::InvalidInput("unknown image format".into()))?;
        if s.resize.is_some() && s.target != Target::Webp {
            return Err(Error::unsupported(
                "resize",
                "only WebP output can resize (cwebp); choose WebP or turn resizing off",
            ));
        }
        let exe = crate::tools::require(job.tools, s.target.tool())?;
        let out = job.temp_dir.join(format!("mj-image-out.{}", s.target.extension()));
        let _ = std::fs::remove_file(&out);
        let o = run(
            exe,
            &encoder_args(&s, job.src, &out),
            &RunOptions {
                cancel: Some(job.cancel),
                cwd: Some(job.temp_dir),
                timeout: Some(std::time::Duration::from_secs(900)),
                ..Default::default()
            },
        )?;
        o.ensure_success(s.target.tool())?;
        let head =
            std::fs::read(&out).map(|b| b.into_iter().take(16).collect::<Vec<u8>>()).map_err(|e| Error::io(&out, e))?;
        if !magic_ok(s.target, &head) {
            let _ = std::fs::remove_file(&out);
            return Err(Error::Internal(format!(
                "{} did not write a valid .{} file",
                s.target.tool(),
                s.target.extension()
            )));
        }
        let after = std::fs::metadata(&out).map_err(|e| Error::io(&out, e))?.len();
        if after >= before {
            let _ = std::fs::remove_file(&out);
            return Ok(common::skipped(before, "The converted picture would not be smaller"));
        }
        common::move_into_place(&out, job.dst)?;
        // Lossless modes keep every pixel (the JPEG XL transcode of a JPEG is reversible); a resize never does.
        let lossy = !s.lossless || s.resize.is_some();
        let mut notes = vec![format!("output_extension={}", s.target.extension())];
        if let Some((w, h)) = s.resize {
            notes.push(format!("resized to {w}x{h}"));
        }
        Ok(common::done(before, after, lossy, notes))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        common::analyse_entries(
            ctx,
            OptimiserId::ImageLossy,
            MIN_SIZE,
            &|e| self.applies_to(e),
            &|e, p| self.estimate(e, p),
            &|e, est| common::Described {
                reason: format!("Re-encoding saves about {} %", 100 - est.bytes_after * 100 / est.bytes_before.max(1)),
                columns: std::iter::once(("type".to_string(), e.extension().to_uppercase())).collect(),
                sub_group: None,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::{CancelToken, NullProgress};
    use mj_formats::png::build::rgb_png;

    #[test]
    fn availability_names_every_missing_tool() {
        assert_eq!(
            ImageLossy.availability(&[]),
            Availability::Disabled { reason: "Requires cwebp, avifenc or cjxl on PATH".into() }
        );
        assert!(ImageLossy.availability(&[crate::tools::tool_at("cjxl", "/x/cjxl")]).is_available());
    }

    #[test]
    fn estimate_scales_with_resize_and_quality() {
        let d = TempDir::new("lossy-est").unwrap();
        let p = d.write("a.png", &rgb_png(400, 200, &[], 1)).unwrap();
        let e = FileEntry::new(&p, mj_core::EntryKind::File, 1_000_000);
        let base = ImageLossy.estimate(&e, &ParamValues::new()).unwrap();
        let small = ImageLossy.estimate(&e, &ParamValues::new().with("max_edge", spec::n(200.0))).unwrap();
        let low_q = ImageLossy.estimate(&e, &ParamValues::new().with("quality", spec::n(40.0))).unwrap();
        assert!(small.bytes_after < base.bytes_after / 3 && low_q.bytes_after < base.bytes_after);
        assert_eq!(base.confidence, mj_core::Confidence::Estimated);
        assert_eq!(dimensions(&p), Some((400, 200)));
    }

    #[test]
    fn resize_with_avif_is_refused_and_missing_tool_reported() {
        let d = TempDir::new("lossy-err").unwrap();
        let src = d.write("a.png", &rgb_png(400, 200, &[], 1)).unwrap();
        let dst = d.join("o.avif");
        let c = CancelToken::new();
        let go = |p: &ParamValues, tools: &[ExternalTool]| {
            ImageLossy.optimise(&OptimiseJob {
                src: &src,
                dst: &dst,
                params: p,
                tools,
                temp_dir: d.path(),
                cancel: &c,
                progress: &NullProgress,
            })
        };
        let p = ParamValues::new().with("format", spec::t("avif")).with("max_edge", spec::n(100.0));
        assert!(matches!(go(&p, &[]), Err(Error::Unsupported { .. })));
        let p = ParamValues::new().with("format", spec::t("avif"));
        assert!(matches!(go(&p, &[]), Err(Error::ToolMissing(t)) if t == "avifenc"));
        assert!(matches!(go(&ParamValues::new().with("format", spec::t("bmp")), &[]), Err(Error::InvalidInput(_))));
        assert!(!dst.exists());
    }

    #[cfg(unix)]
    #[test]
    fn fake_encoder_flow_checks_output_and_records_arguments() {
        use std::os::unix::fs::PermissionsExt;
        let d = TempDir::new("lossy-fake").unwrap();
        let src = d.write("a.png", &rgb_png(400, 200, &[], 1)).unwrap();
        let log = d.join("args.txt");
        let exe = d.join("cwebp");
        let body = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nwhile [ $# -gt 0 ]; do if [ \"$1\" = \"-o\" ]; then out=\"$2\"; fi; shift; done\nprintf 'RIFF\\0\\0\\0\\0WEBPVP8 tiny' > \"$out\"\n", log.display());
        std::fs::write(&exe, body).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tools = vec![crate::tools::tool_at("cwebp", &exe)];
        let (dst, c) = (d.join("o.webp"), CancelToken::new());
        let p = ParamValues::new().with("quality", spec::n(70.0)).with("max_edge", spec::n(200.0));
        let r = ImageLossy
            .optimise(&OptimiseJob {
                src: &src,
                dst: &dst,
                params: &p,
                tools: &tools,
                temp_dir: d.path(),
                cancel: &c,
                progress: &NullProgress,
            })
            .unwrap();
        assert!(r.lossy && r.notes.contains(&"output_extension=webp".to_string()) && dst.is_file());
        let args = std::fs::read_to_string(&log).unwrap();
        assert!(args.contains("-q\n70\n") && args.contains("-resize\n200\n100\n"), "{args}");
        // An encoder that writes garbage is an error and leaves no destination.
        std::fs::write(&exe, "#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = \"-o\" ]; then out=\"$2\"; fi; shift; done\necho nonsense > \"$out\"\n").unwrap();
        let dst2 = d.join("o2.webp");
        let r = ImageLossy.optimise(&OptimiseJob {
            src: &src,
            dst: &dst2,
            params: &p,
            tools: &tools,
            temp_dir: d.path(),
            cancel: &c,
            progress: &NullProgress,
        });
        assert!(r.is_err() && !dst2.exists());
    }

    /// Runs only where the real encoders exist; reports which ones it exercised.
    #[test]
    fn real_encoders_when_present() {
        let tools = crate::tools::detect(&Default::default());
        let present: Vec<&str> =
            ["cwebp", "avifenc", "cjxl"].into_iter().filter(|t| mj_core::tools::find(&tools, t).is_some()).collect();
        eprintln!("real image encoders present: {present:?}");
        // A flat synthetic PNG is tiny, so only check the run completes or is skipped as not smaller.
        let d = TempDir::new("lossy-real").unwrap();
        let src = d.write("a.png", &rgb_png(512, 512, &[], 1)).unwrap();
        for (i, t) in present.iter().enumerate() {
            let fmt = match *t {
                "cwebp" => "webp",
                "avifenc" => "avif",
                _ => "jxl",
            };
            let dst = d.join(format!("o{i}"));
            let p = ParamValues::new().with("format", spec::t(fmt));
            let c = CancelToken::new();
            let r = ImageLossy
                .optimise(&OptimiseJob {
                    src: &src,
                    dst: &dst,
                    params: &p,
                    tools: &tools,
                    temp_dir: d.path(),
                    cancel: &c,
                    progress: &NullProgress,
                })
                .unwrap();
            assert!(r.skipped.is_some() || dst.is_file());
        }
    }
}
