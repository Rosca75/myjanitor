//! Exact argument vectors for the external image tools (oxipng, jpegtran, gifsicle, cwebp, avifenc,
//! cjxl). Pure functions: nothing here starts a process, so every preset is unit-tested without the
//! tool. Paths go through [`path_arg`]; there is never a shell string.

use std::ffi::OsString;
use std::path::Path;

use mj_formats::png::MetadataPolicy;

use crate::external::{args, path_arg};

/// Target of a lossy or converting encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Webp,
    Avif,
    Jxl,
}

impl Target {
    /// Parse the `format` parameter.
    pub fn parse(s: &str) -> Option<Target> {
        match s {
            "webp" => Some(Target::Webp),
            "avif" => Some(Target::Avif),
            "jxl" => Some(Target::Jxl),
            _ => None,
        }
    }

    /// Tool id that writes it.
    pub fn tool(self) -> &'static str {
        match self {
            Target::Webp => "cwebp",
            Target::Avif => "avifenc",
            Target::Jxl => "cjxl",
        }
    }

    /// File extension of the output.
    pub fn extension(self) -> &'static str {
        match self {
            Target::Webp => "webp",
            Target::Avif => "avif",
            Target::Jxl => "jxl",
        }
    }
}

/// Everything an encode needs besides the paths.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodeSpec {
    pub target: Target,
    /// 1-100 (higher is better); ignored when `lossless`.
    pub quality: u8,
    /// Pixel-exact output (WebP lossless, AVIF lossless, JXL distance 0 or lossless JPEG transcode).
    pub lossless: bool,
    /// Input is a JPEG (enables the reversible JPEG XL transcode).
    pub input_is_jpeg: bool,
    /// Drop EXIF/XMP (a colour profile is always kept).
    pub strip_metadata: bool,
    /// Resize to exactly this size (WebP only).
    pub resize: Option<(u32, u32)>,
    /// Encoder effort 0-9 (higher = smaller and slower).
    pub effort: u8,
}

/// `oxipng`: optimisation level 1-6 and the metadata policy.
pub fn oxipng_args(src: &Path, dst: &Path, policy: MetadataPolicy, level: u8) -> Vec<OsString> {
    let mut a = args(["-o", &level.clamp(1, 6).to_string(), "-q"]);
    match policy {
        MetadataPolicy::KeepAll => {}
        MetadataPolicy::StripText => a.extend(args(["--strip", "tEXt,zTXt,iTXt"])),
        MetadataPolicy::StripAll { colour: false } => a.extend(args(["--strip", "safe"])),
        MetadataPolicy::StripAll { colour: true } => a.extend(args(["--strip", "all"])),
    }
    a.push("--out".into());
    a.push(path_arg(dst));
    a.push(path_arg(src));
    a
}

/// `jpegtran`: lossless Huffman optimisation, optionally progressive. `-copy all` keeps the markers
/// the in-house rewrite already decided to keep.
pub fn jpegtran_args(src: &Path, dst: &Path, progressive: bool) -> Vec<OsString> {
    let mut a = args(["-copy", "all", "-optimize"]);
    if progressive {
        a.push("-progressive".into());
    }
    a.push("-outfile".into());
    a.push(path_arg(dst));
    a.push(path_arg(src));
    a
}

/// `gifsicle`: lossless optimisation (`-O3`), optionally dropping comments and extensions.
pub fn gifsicle_args(src: &Path, dst: &Path, strip_metadata: bool) -> Vec<OsString> {
    let mut a = args(["-O3"]);
    if strip_metadata {
        a.extend(args(["--no-comments", "--no-extensions", "--no-names"]));
    }
    a.push("-o".into());
    a.push(path_arg(dst));
    a.push(path_arg(src));
    a
}

/// `cwebp`, `avifenc` or `cjxl` for `spec`.
pub fn encoder_args(spec: &EncodeSpec, src: &Path, dst: &Path) -> Vec<OsString> {
    let q = spec.quality.clamp(1, 100).to_string();
    let effort = spec.effort.min(9);
    match spec.target {
        Target::Webp => {
            let mut a = args(["-quiet", "-mt", "-m", &effort.min(6).to_string()]);
            if spec.lossless {
                a.extend(args(["-lossless", "-z", &effort.to_string()]));
            } else {
                a.extend(args(["-q", &q]));
            }
            if let Some((w, h)) = spec.resize {
                a.extend(args(["-resize", &w.to_string(), &h.to_string()]));
            }
            a.extend(args(["-metadata", if spec.strip_metadata { "icc" } else { "all" }]));
            a.extend(args(["-o"]));
            a.push(path_arg(dst));
            a.push(path_arg(src));
            a
        }
        Target::Avif => {
            let speed = (10 - effort.min(9)).min(10).to_string();
            let mut a = args(["-s", &speed, "-j", "all"]);
            if spec.lossless {
                a.push("--lossless".into());
            } else {
                a.extend(args(["-q", &q]));
            }
            if spec.strip_metadata {
                a.extend(args(["--ignore-exif", "--ignore-xmp"]));
            }
            a.push("-o".into());
            a.push(path_arg(dst));
            a.push(path_arg(src));
            a
        }
        Target::Jxl => {
            let mut a = vec![path_arg(src), path_arg(dst)];
            a.extend(args(["--quiet", &format!("-e{}", effort.clamp(1, 9).max(3))]));
            if spec.lossless && spec.input_is_jpeg {
                a.push("--lossless_jpeg=1".into()); // reversible: the JPEG can be rebuilt bit for bit
            } else if spec.lossless {
                a.extend(args(["-d", "0", "--lossless_jpeg=0"]));
            } else {
                a.extend(args(["-q", &q, "--lossless_jpeg=0"]));
            }
            a
        }
    }
}

/// Size to resize to so the long (or short) edge is at most `max_edge`; `None` when already small
/// enough (never upscales).
pub fn resize_target(w: u32, h: u32, max_edge: u32, long_edge: bool) -> Option<(u32, u32)> {
    if max_edge == 0 || w == 0 || h == 0 {
        return None;
    }
    let edge = if long_edge { w.max(h) } else { w.min(h) };
    if edge <= max_edge {
        return None;
    }
    let f = f64::from(max_edge) / f64::from(edge);
    let s = |v: u32| ((f64::from(v) * f).round() as u32).max(1);
    Some((s(w), s(h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[OsString]) -> Vec<String> {
        v.iter().map(|x| x.to_string_lossy().into_owned()).collect()
    }

    fn spec(target: Target) -> EncodeSpec {
        EncodeSpec {
            target,
            quality: 80,
            lossless: false,
            input_is_jpeg: true,
            strip_metadata: false,
            resize: None,
            effort: 6,
        }
    }

    #[test]
    fn oxipng_policies() {
        let (a, b) = (Path::new("/in/a.png"), Path::new("/t/o.png"));
        assert_eq!(
            s(&oxipng_args(a, b, MetadataPolicy::KeepAll, 4)),
            ["-o", "4", "-q", "--out", "/t/o.png", "/in/a.png"]
        );
        assert_eq!(s(&oxipng_args(a, b, MetadataPolicy::StripText, 9))[3..5], ["--strip", "tEXt,zTXt,iTXt"]);
        assert_eq!(s(&oxipng_args(a, b, MetadataPolicy::StripAll { colour: false }, 4))[3..5], ["--strip", "safe"]);
        assert_eq!(s(&oxipng_args(a, b, MetadataPolicy::StripAll { colour: true }, 0))[..2], ["-o", "1"]);
        assert!(s(&oxipng_args(Path::new("-x.png"), b, MetadataPolicy::KeepAll, 4)).contains(&"./-x.png".to_string()));
    }

    #[test]
    fn jpegtran_and_gifsicle() {
        let (a, b) = (Path::new("/in/a.jpg"), Path::new("/t/o.jpg"));
        assert_eq!(
            s(&jpegtran_args(a, b, true)),
            ["-copy", "all", "-optimize", "-progressive", "-outfile", "/t/o.jpg", "/in/a.jpg"]
        );
        assert_eq!(s(&jpegtran_args(a, b, false)), ["-copy", "all", "-optimize", "-outfile", "/t/o.jpg", "/in/a.jpg"]);
        assert_eq!(s(&gifsicle_args(Path::new("a.gif"), Path::new("o.gif"), false)), ["-O3", "-o", "o.gif", "a.gif"]);
        assert!(s(&gifsicle_args(Path::new("a.gif"), Path::new("o.gif"), true)).contains(&"--no-comments".to_string()));
    }

    #[test]
    fn cwebp_presets() {
        let (a, b) = (Path::new("/in/a.jpg"), Path::new("/t/o.webp"));
        assert_eq!(
            s(&encoder_args(&spec(Target::Webp), a, b)),
            ["-quiet", "-mt", "-m", "6", "-q", "80", "-metadata", "all", "-o", "/t/o.webp", "/in/a.jpg"]
        );
        let mut l = spec(Target::Webp);
        (l.lossless, l.strip_metadata, l.resize) = (true, true, Some((1600, 900)));
        assert_eq!(
            s(&encoder_args(&l, Path::new("/in/a.png"), b)),
            [
                "-quiet",
                "-mt",
                "-m",
                "6",
                "-lossless",
                "-z",
                "6",
                "-resize",
                "1600",
                "900",
                "-metadata",
                "icc",
                "-o",
                "/t/o.webp",
                "/in/a.png"
            ]
        );
    }

    #[test]
    fn avifenc_presets() {
        let (a, b) = (Path::new("/in/a.jpg"), Path::new("/t/o.avif"));
        assert_eq!(
            s(&encoder_args(&spec(Target::Avif), a, b)),
            ["-s", "4", "-j", "all", "-q", "80", "-o", "/t/o.avif", "/in/a.jpg"]
        );
        let mut l = spec(Target::Avif);
        (l.lossless, l.strip_metadata, l.effort) = (true, true, 9);
        assert_eq!(
            s(&encoder_args(&l, a, b)),
            ["-s", "1", "-j", "all", "--lossless", "--ignore-exif", "--ignore-xmp", "-o", "/t/o.avif", "/in/a.jpg"]
        );
    }

    #[test]
    fn cjxl_presets() {
        let (a, b) = (Path::new("/in/a.jpg"), Path::new("/t/o.jxl"));
        let mut rev = spec(Target::Jxl);
        rev.lossless = true;
        assert_eq!(s(&encoder_args(&rev, a, b)), ["/in/a.jpg", "/t/o.jxl", "--quiet", "-e6", "--lossless_jpeg=1"]);
        rev.input_is_jpeg = false;
        assert_eq!(s(&encoder_args(&rev, Path::new("/in/a.png"), b))[4..], ["-d", "0", "--lossless_jpeg=0"]);
        let lossy = spec(Target::Jxl);
        assert_eq!(s(&encoder_args(&lossy, a, b))[4..], ["-q", "80", "--lossless_jpeg=0"]);
    }

    #[test]
    fn quality_is_clamped_and_target_parsed() {
        let mut q = spec(Target::Webp);
        q.quality = 0;
        assert!(s(&encoder_args(&q, Path::new("a"), Path::new("b"))).windows(2).any(|w| w == ["-q", "1"]));
        q.quality = 250;
        assert!(s(&encoder_args(&q, Path::new("a"), Path::new("b"))).windows(2).any(|w| w == ["-q", "100"]));
        assert_eq!(Target::parse("avif"), Some(Target::Avif));
        assert_eq!(Target::parse("bmp"), None);
        assert_eq!((Target::Jxl.tool(), Target::Jxl.extension()), ("cjxl", "jxl"));
    }

    #[test]
    fn resize_never_upscales_and_keeps_aspect() {
        assert_eq!(resize_target(4000, 3000, 2000, true), Some((2000, 1500)));
        assert_eq!(resize_target(3000, 4000, 2000, true), Some((1500, 2000)));
        assert_eq!(resize_target(4000, 3000, 1500, false), Some((2000, 1500)));
        assert_eq!(resize_target(1000, 800, 2000, true), None);
        assert_eq!(resize_target(1000, 800, 0, true), None);
        assert_eq!(resize_target(10_000, 1, 100, true), Some((100, 1)));
    }
}
