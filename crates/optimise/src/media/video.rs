//! Video re-encode (OPT-VID-01..06) through external ffmpeg.
//! Owner: W3-C.
//!
//! | feature | how |
//! |---|---|
//! | OPT-VID-01 | `codec` hevc or av1, `quality` (CRF) or the advanced `crf` |
//! | OPT-VID-02 | `encoder` nvenc, qsv or amf; [`detect_hardware`] parses `ffmpeg -encoders` and trial-encodes; [`encode_time_estimate`] |
//! | OPT-VID-03 | `max_height` and `max_fps` |
//! | OPT-VID-04 | `drop_extra_audio` and `drop_subtitles` |
//! | OPT-VID-05 | `remux_only`: stream copy, junk streams dropped, lossless |
//! | OPT-VID-06 | [`compatibility_warning`], also shown as the codec option's help |

use std::ffi::OsString;
use std::path::Path;

use mj_core::{
    ActionKind, Availability, CancelToken, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool,
    FileEntry, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport,
    Result, SafetyClass, SettingSpec, StepPreset,
};
use mj_formats::media::MediaInfo;

use super::estimate::{encode_seconds, info_of, video_after, VideoPlan};
use super::ffmpeg::{base_args, clock, encoder_accepts, input_args, list_encoders, run_encode};
use crate::external::{args, path_arg};
use crate::images::common::{self, spec};

/// Videos: Re-encode to HEVC or AV1
#[derive(Debug, Default, Clone, Copy)]
pub struct Video;

/// Extensions (lower case) this optimiser targets.
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "avi", "wmv", "m4v"];

const MIN_SIZE: u64 = 4 * 1024 * 1024;

/// Target video codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VCodec {
    Hevc,
    Av1,
}

impl VCodec {
    fn name(self) -> &'static str {
        match self {
            VCodec::Hevc => "hevc",
            VCodec::Av1 => "av1",
        }
    }
}

/// Encoder effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    Fast,
    Medium,
    Slow,
}

impl Speed {
    /// Time multiplier against Medium for encode-time estimates.
    pub fn time_factor(self) -> f64 {
        match self {
            Speed::Fast => 2.0,
            Speed::Medium => 1.0,
            Speed::Slow => 0.4,
        }
    }
}

/// Everything the argument builder needs.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoSettings {
    pub codec: VCodec,
    pub crf: u32,
    pub speed: Speed,
    /// `auto` (software), `software`, `nvenc`, `qsv` or `amf`.
    pub encoder_pref: String,
    pub max_height: u32,
    pub max_fps: f64,
    pub drop_extra_audio: bool,
    pub drop_subtitles: bool,
    pub audio_kbps: u32,
    pub remux_only: bool,
    /// AV1 screen-content tools (OPT-SCR-03).
    pub screen_tools: bool,
    /// Keep `[start, end]` seconds only (OPT-SCR-04).
    pub trim: Option<(f64, Option<f64>)>,
    /// Content is a screen capture (cheaper per pixel in estimates).
    pub screen: bool,
}

impl VideoSettings {
    /// Read the parameters (`quality` picks a CRF per codec unless `crf` overrides it).
    pub fn from_params(p: &ParamValues) -> VideoSettings {
        let codec = if p.text("codec", "hevc") == "av1" { VCodec::Av1 } else { VCodec::Hevc };
        let quality = p.text("quality", "balanced");
        let by_quality = match (codec, quality) {
            (VCodec::Hevc, "high") => 24,
            (VCodec::Hevc, "small") => 32,
            (VCodec::Hevc, _) => 28,
            (VCodec::Av1, "high") => 28,
            (VCodec::Av1, "small") => 38,
            (VCodec::Av1, _) => 33,
        };
        let crf = p.number("crf", 0.0);
        VideoSettings {
            codec,
            crf: if crf >= 1.0 { crf.min(51.0) as u32 } else { by_quality },
            speed: match p.text("speed", "medium") {
                "fast" => Speed::Fast,
                "slow" => Speed::Slow,
                _ => Speed::Medium,
            },
            encoder_pref: p.text("encoder", "auto").to_string(),
            max_height: p.text("max_height", "0").parse().unwrap_or(0),
            max_fps: p.text("max_fps", "0").parse().unwrap_or(0.0),
            drop_extra_audio: p.bool("drop_extra_audio", false),
            drop_subtitles: p.bool("drop_subtitles", false),
            audio_kbps: p.number("audio_kbps", 128.0).clamp(32.0, 512.0) as u32,
            remux_only: p.bool("remux_only", false),
            screen_tools: false,
            trim: None,
            screen: false,
        }
    }

    /// The size-model view of these settings.
    pub fn plan(&self) -> VideoPlan {
        VideoPlan {
            av1: self.codec == VCodec::Av1,
            crf: f64::from(self.crf),
            max_height: self.max_height,
            max_fps: self.max_fps,
            screen: self.screen,
            remux: self.remux_only,
            audio_only: false,
        }
    }
}

/// OPT-VID-06: who can play the result.
pub fn compatibility_warning(codec: VCodec) -> &'static str {
    match codec {
        VCodec::Hevc => "HEVC plays on most devices from 2016 on; older PCs, some browsers and some TVs need a codec pack or cannot play it.",
        VCodec::Av1 => "AV1 needs a recent device or player (2020 or newer phones and GPUs, current browsers); older TVs and PCs cannot play it.",
    }
}

/// Hardware encoder family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwKind {
    Nvenc,
    Qsv,
    Amf,
}

impl HwKind {
    fn suffix(self) -> &'static str {
        match self {
            HwKind::Nvenc => "nvenc",
            HwKind::Qsv => "qsv",
            HwKind::Amf => "amf",
        }
    }
}

/// A hardware encoder listed by ffmpeg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HwEncoder {
    pub name: String,
    pub kind: HwKind,
    pub codec: VCodec,
    /// A trial encode succeeded (the hardware is really there).
    pub verified: bool,
}

/// The encoder chosen for a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    pub name: String,
    pub hw: Option<HwKind>,
}

/// The names `ffmpeg -encoders` lists.
#[derive(Debug, Clone, Default)]
pub struct Encoders(pub Vec<String>);

impl Encoders {
    pub fn has(&self, n: &str) -> bool {
        self.0.iter().any(|x| x == n)
    }

    /// Listed hardware encoders for HEVC and AV1 (not yet verified).
    pub fn hardware(&self) -> Vec<HwEncoder> {
        let mut v = Vec::new();
        for codec in [VCodec::Hevc, VCodec::Av1] {
            for kind in [HwKind::Nvenc, HwKind::Qsv, HwKind::Amf] {
                let name = format!("{}_{}", codec.name(), kind.suffix());
                if self.has(&name) {
                    v.push(HwEncoder { name, kind, codec, verified: false });
                }
            }
        }
        v
    }

    /// Pick the encoder for `codec` and the user's preference; hardware only when asked for.
    pub fn choose(&self, codec: VCodec, pref: &str) -> Option<Chosen> {
        let hw = match pref {
            "nvenc" => Some(HwKind::Nvenc),
            "qsv" => Some(HwKind::Qsv),
            "amf" => Some(HwKind::Amf),
            _ => None,
        };
        if let Some(kind) = hw {
            let name = format!("{}_{}", codec.name(), kind.suffix());
            if self.has(&name) {
                return Some(Chosen { name, hw: Some(kind) });
            }
        }
        let software: &[&str] = match codec {
            VCodec::Hevc => &["libx265"],
            VCodec::Av1 => &["libsvtav1", "libaom-av1", "librav1e"],
        };
        software.iter().find(|n| self.has(n)).map(|n| Chosen { name: (*n).to_string(), hw: None })
    }
}

/// Query ffmpeg and trial-encode each listed hardware encoder (OPT-VID-02).
pub fn detect_hardware(ffmpeg: &Path, cancel: &CancelToken) -> Result<Vec<HwEncoder>> {
    let enc = Encoders(list_encoders(ffmpeg, cancel)?);
    Ok(enc
        .hardware()
        .into_iter()
        .map(|mut h| {
            h.verified = encoder_accepts(ffmpeg, &h.name, &[], cancel);
            h
        })
        .collect())
}

/// Rough encode time in seconds for a file (needs metadata; `None` without it).
pub fn encode_time_estimate(info: &MediaInfo, s: &VideoSettings, encoder: &str) -> Option<f64> {
    let (w, h) = (info.width?, info.height?);
    let (w, h) = if s.max_height > 0 && h > s.max_height { (w * s.max_height / h, s.max_height) } else { (w, h) };
    let fps = f64::from(info.frame_rate.unwrap_or(30.0));
    let fps = if s.max_fps > 0.0 { fps.min(s.max_fps) } else { fps };
    Some(encode_seconds(encoder, s.speed.time_factor(), info.duration_s()?, w, h, fps))
}

/// Output container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Container {
    pub ext: &'static str,
    pub muxer: &'static str,
}

/// Output container for a source: MP4 where possible, Matroska where MP4 cannot hold the streams.
pub fn container_for(src_ext: &str, remux: bool, has_subtitles: bool) -> Container {
    const MP4: Container = Container { ext: "mp4", muxer: "mp4" };
    const MKV: Container = Container { ext: "mkv", muxer: "matroska" };
    match src_ext {
        "mkv" | "webm" => MKV,
        "mov" if remux => Container { ext: "mov", muxer: "mov" },
        "avi" | "wmv" if remux || has_subtitles => MKV,
        "mp4" | "m4v" | "mov" | "avi" | "wmv" => MP4,
        _ => MKV,
    }
}

/// What the builder needs to know about the source.
#[derive(Debug, Clone, Default)]
pub struct SourceFacts {
    pub ext: String,
    pub audio_codec: Option<String>,
    pub fps: Option<f32>,
    pub height: Option<u32>,
    pub subtitle_tracks: u32,
}

impl SourceFacts {
    /// From a path and (optional) metadata.
    pub fn of(path: &Path, info: Option<&MediaInfo>) -> SourceFacts {
        SourceFacts {
            ext: mj_core::pathutil::extension_lower(path),
            audio_codec: info.and_then(|i| i.audio_codec.clone()),
            fps: info.and_then(|i| i.frame_rate),
            height: info.and_then(|i| i.height),
            subtitle_tracks: info.map_or(0, |i| i.subtitle_tracks),
        }
    }
}

fn mp4_audio_ok(codec: Option<&str>) -> bool {
    matches!(codec, Some("aac" | "mp3" | "ac3" | "eac3" | "alac" | "opus"))
}

/// Encoder options for one chosen encoder.
fn encoder_options(c: &Chosen, codec: VCodec, s: &VideoSettings) -> Vec<String> {
    let crf = s.crf.to_string();
    let v = |x: &[&str]| x.iter().map(|y| y.to_string()).collect::<Vec<_>>();
    let mut o = match (c.name.as_str(), c.hw) {
        (_, Some(HwKind::Nvenc)) => v(&[
            "-c:v",
            &c.name,
            "-rc",
            "vbr",
            "-cq",
            &crf,
            "-b:v",
            "0",
            "-preset",
            match s.speed {
                Speed::Fast => "p3",
                Speed::Medium => "p5",
                Speed::Slow => "p7",
            },
        ]),
        (_, Some(HwKind::Qsv)) => v(&[
            "-c:v",
            &c.name,
            "-global_quality",
            &crf,
            "-preset",
            match s.speed {
                Speed::Fast => "fast",
                Speed::Medium => "medium",
                Speed::Slow => "slow",
            },
        ]),
        (_, Some(HwKind::Amf)) => v(&["-c:v", &c.name, "-rc", "cqp", "-qp_i", &crf, "-qp_p", &crf]),
        ("libx265", _) => v(&[
            "-c:v",
            "libx265",
            "-crf",
            &crf,
            "-preset",
            match s.speed {
                Speed::Fast => "fast",
                Speed::Medium => "medium",
                Speed::Slow => "slow",
            },
            "-x265-params",
            "log-level=error",
        ]),
        ("libsvtav1", _) => {
            let mut x = v(&[
                "-c:v",
                "libsvtav1",
                "-crf",
                &crf,
                "-preset",
                match s.speed {
                    Speed::Fast => "10",
                    Speed::Medium => "8",
                    Speed::Slow => "5",
                },
            ]);
            if s.screen_tools {
                x.extend(v(&["-svtav1-params", "scm=1"]));
            }
            x
        }
        ("libaom-av1", _) => {
            let mut x = v(&[
                "-c:v",
                "libaom-av1",
                "-crf",
                &crf,
                "-b:v",
                "0",
                "-row-mt",
                "1",
                "-cpu-used",
                match s.speed {
                    Speed::Fast => "8",
                    Speed::Medium => "6",
                    Speed::Slow => "4",
                },
            ]);
            if s.screen_tools {
                x.extend(v(&["-aom-params", "tune-content=screen"]));
            }
            x
        }
        (name, _) => v(&["-c:v", name, "-qp", &(s.crf * 4).min(255).to_string()]),
    };
    if codec == VCodec::Hevc {
        o.extend(v(&["-tag:v", "hvc1"])); // Apple players need the hvc1 tag
    }
    o
}

/// The extra encoder arguments that enable screen-content coding, for the support probe.
pub fn screen_tool_args(encoder: &str) -> Option<&'static [&'static str]> {
    match encoder {
        "libsvtav1" => Some(&["-svtav1-params", "scm=1"]),
        "libaom-av1" => Some(&["-aom-params", "tune-content=screen"]),
        _ => None,
    }
}

/// Full ffmpeg argument vector for one file.
pub fn build_args(
    s: &VideoSettings,
    chosen: Option<&Chosen>,
    f: &SourceFacts,
    src: &Path,
    out: &Path,
) -> Vec<OsString> {
    let mut a = base_args();
    a.extend(input_args(src));
    if let Some((start, end)) = s.trim {
        a.extend(args(["-ss", &format!("{start:.3}")]));
        if let Some(e) = end {
            a.extend(args(["-to", &format!("{e:.3}")]));
        }
    }
    let container = container_for(&f.ext, s.remux_only, f.subtitle_tracks > 0 && !s.drop_subtitles);
    a.extend(args(["-map", "0:v:0"]));
    a.extend(args(["-map", if s.drop_extra_audio { "0:a:0?" } else { "0:a?" }]));
    let subs = !s.drop_subtitles && (s.remux_only || f.subtitle_tracks > 0);
    if subs {
        a.extend(args(["-map", "0:s?"]));
    }
    a.extend(args(["-map_metadata", "0", "-map_chapters", "0"]));
    if s.remux_only {
        a.extend(args(["-c", "copy"]));
    } else {
        let mut filters = Vec::new();
        if s.max_height > 0 && f.height.is_none_or(|h| h > s.max_height) {
            filters.push(format!("scale=-2:min(ih\\,{})", s.max_height));
        }
        if s.max_fps > 0.0 && f.fps.is_none_or(|r| f64::from(r) > s.max_fps + 0.5) {
            filters.push(format!("fps={}", s.max_fps));
        }
        if !filters.is_empty() {
            a.extend(args(["-vf", &filters.join(",")]));
        }
        if let Some(c) = chosen {
            a.extend(args(encoder_options(c, s.codec, s)));
        }
        if container.ext == "mkv" || mp4_audio_ok(f.audio_codec.as_deref()) {
            a.extend(args(["-c:a", "copy"]));
        } else {
            a.extend(args(["-c:a", "aac", "-b:a", &format!("{}k", s.audio_kbps)]));
        }
        if subs {
            a.extend(args(["-c:s", if container.ext == "mkv" { "copy" } else { "mov_text" }]));
        }
    }
    if container.ext == "mp4" {
        a.extend(args(["-movflags", "+faststart"]));
    }
    a.extend(args(["-f", container.muxer]));
    a.push(path_arg(out));
    a
}

/// Shared by [`Video`] and the screen-recording optimiser: choose the encoder, run ffmpeg, verify,
/// move the result into place.
pub(crate) fn run_video_job(
    job: &OptimiseJob<'_>,
    s: &mut VideoSettings,
    mut notes: Vec<String>,
) -> Result<OptimiseOutput> {
    job.cancel.checkpoint()?;
    let exe = crate::tools::require(job.tools, "ffmpeg")?;
    let before = std::fs::metadata(job.src).map_err(|e| Error::io(job.src, e))?.len();
    let info = info_of(job.src);
    let facts = SourceFacts::of(job.src, info.as_ref());
    let mut chosen = None;
    if !s.remux_only {
        let enc = Encoders(list_encoders(exe, job.cancel)?);
        let mut c = enc.choose(s.codec, &s.encoder_pref).ok_or_else(|| Error::unsupported(format!("{} encoding", s.codec.name()), "this ffmpeg has no encoder for it; pick the other codec or install an ffmpeg build with libx265 or libsvtav1"))?;
        if c.hw.is_some() && !encoder_accepts(exe, &c.name, &[], job.cancel) {
            notes.push(format!("{} could not start on this PC; the software encoder was used", c.name));
            c = enc
                .choose(s.codec, "software")
                .ok_or_else(|| Error::unsupported("encoding", "no software encoder available"))?;
        }
        if s.screen_tools {
            match screen_tool_args(&c.name) {
                Some(extra) if encoder_accepts(exe, &c.name, extra, job.cancel) => {
                    notes.push("AV1 screen-content tools on".into())
                }
                _ => {
                    s.screen_tools = false;
                    notes.push(format!("{} has no screen-content tools; standard settings used", c.name));
                }
            }
        }
        notes.push(format!("encoder: {}", c.name));
        notes.push(compatibility_warning(s.codec).to_string());
        chosen = Some(c);
    }
    let container = container_for(&facts.ext, s.remux_only, facts.subtitle_tracks > 0 && !s.drop_subtitles);
    notes.push(format!("output_extension={}", container.ext));
    let out = job.temp_dir.join(format!("mj-video-out.{}", container.ext));
    let _ = std::fs::remove_file(&out);
    let dur = info.as_ref().and_then(|i| i.duration_ms);
    run_encode(job, exe, &build_args(s, chosen.as_ref(), &facts, job.src, &out), dur)?;
    let after = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    if after == 0 {
        return Err(super::ffmpeg::empty_output("ffmpeg"));
    }
    if let (None, Some(d_in), Some(d_out)) = (s.trim, dur, info_of(&out).and_then(|i| i.duration_ms)) {
        if d_in.abs_diff(d_out) > (d_in / 25).max(2000) {
            let _ = std::fs::remove_file(&out);
            return Err(Error::Internal(format!(
                "the new video is {} long but the original is {}; nothing was changed",
                clock(d_out as f64 / 1000.0),
                clock(d_in as f64 / 1000.0)
            )));
        }
    }
    if after >= before {
        let _ = std::fs::remove_file(&out);
        return Ok(common::skipped(before, "The new video would not be smaller"));
    }
    common::move_into_place(&out, job.dst)?;
    Ok(common::done(before, after, !s.remux_only, notes))
}

fn settings() -> Vec<SettingSpec> {
    let mut codec = spec::choice(
        "codec",
        "Codec",
        "hevc",
        &[
            ("hevc", "HEVC (H.265)", Some(compatibility_warning(VCodec::Hevc))),
            ("av1", "AV1", Some(compatibility_warning(VCodec::Av1))),
        ],
        false,
    );
    codec.help = Some("Choose how new the player may need to be".into());
    vec![
        codec,
        spec::choice(
            "quality",
            "Quality",
            "balanced",
            &[("high", "High", None), ("balanced", "Balanced", None), ("small", "Smaller", None)],
            false,
        ),
        spec::choice(
            "max_height",
            "Largest height",
            "0",
            &[("0", "Keep size", None), ("1080", "1080p", None), ("720", "720p", None), ("480", "480p", None)],
            false,
        ),
        spec::choice(
            "max_fps",
            "Frame rate cap",
            "0",
            &[("0", "Keep", None), ("30", "30 fps", None), ("24", "24 fps", None), ("15", "15 fps", None)],
            false,
        ),
        spec::toggle("drop_extra_audio", "Keep only the first audio track", None, false, false, true),
        spec::toggle("drop_subtitles", "Drop subtitles", None, false, false, true),
        spec::toggle(
            "remux_only",
            "Remux only (no re-encode)",
            Some("Keeps the picture as is and drops junk streams"),
            false,
            false,
            false,
        ),
        spec::choice(
            "encoder",
            "Encoder",
            "auto",
            &[
                ("auto", "Software (best quality)", None),
                ("nvenc", "NVIDIA NVENC", Some("Faster; larger files")),
                ("qsv", "Intel Quick Sync", Some("Faster; larger files")),
                ("amf", "AMD AMF", Some("Faster; larger files")),
            ],
            true,
        ),
        spec::choice(
            "speed",
            "Speed",
            "medium",
            &[("fast", "Fast", None), ("medium", "Balanced", None), ("slow", "Slow, smaller", None)],
            true,
        ),
        spec::number("crf", "Custom CRF (0 = from quality)", 0.0, 0.0, 51.0, 1.0, None, true),
        spec::number("audio_kbps", "Audio bitrate when converted", 128.0, 32.0, 512.0, 16.0, Some("kbps"), true),
    ]
}

fn presets() -> Vec<StepPreset> {
    use spec::{b, t};
    vec![
        spec::preset(
            "efficient",
            "Efficient",
            "HEVC · looks the same, much smaller",
            true,
            &[("codec", t("hevc")), ("quality", t("balanced")), ("remux_only", b(false))],
        ),
        spec::preset(
            "smallest",
            "Smallest",
            "AV1 · newest players only",
            true,
            &[("codec", t("av1")), ("quality", t("balanced")), ("remux_only", b(false))],
        ),
        spec::preset("remux", "Remux only", "No re-encode · no quality loss", false, &[("remux_only", b(true))]),
    ]
}

impl Optimiser for Video {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::Video,
            title: "Videos".into(),
            description: "Re-encode to HEVC or AV1".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: presets(),
            settings: settings(),
            platform: PlatformSupport::Both,
            tools: vec!["ffmpeg".into()],
            feature_ids: (1..=6).map(|i| format!("OPT-VID-0{i}")).collect(),
            unit: "videos".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        crate::tools::availability(tools, &["ffmpeg"])
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && VIDEO_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let s = VideoSettings::from_params(params);
        let after = video_after(info_of(&entry.path).as_ref(), entry.size, &s.plan());
        Ok(Estimate {
            files: 1,
            bytes_before: entry.size,
            bytes_after: after,
            confidence: mj_core::Confidence::Estimated,
            skipped: 0,
        })
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let mut s = VideoSettings::from_params(job.params);
        run_video_job(job, &mut s, Vec::new())
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        common::analyse_entries(
            ctx,
            OptimiserId::Video,
            MIN_SIZE,
            &|e| self.applies_to(e),
            &|e, p| self.estimate(e, p),
            &|e, _| {
                let info = info_of(&e.path);
                let mut columns = std::collections::BTreeMap::new();
                if let Some(i) = &info {
                    if let Some(d) = i.duration_s() {
                        columns.insert("length".to_string(), clock(d));
                    }
                    if let (Some(w), Some(h)) = (i.width, i.height) {
                        columns.insert("resolution".to_string(), format!("{w}x{h}"));
                    }
                    if let Some(c) = &i.video_codec {
                        columns.insert("codec".to_string(), c.clone());
                    }
                }
                common::Described {
                    reason: "Video can be re-encoded at a lower bitrate".into(),
                    columns,
                    sub_group: None,
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(f: impl FnOnce(&mut ParamValues)) -> VideoSettings {
        let mut p = ParamValues::new();
        f(&mut p);
        VideoSettings::from_params(&p)
    }

    fn s(v: &[OsString]) -> Vec<String> {
        v.iter().map(|x| x.to_string_lossy().into_owned()).collect()
    }

    fn x265() -> Chosen {
        Chosen { name: "libx265".into(), hw: None }
    }

    fn facts(ext: &str) -> SourceFacts {
        SourceFacts {
            ext: ext.into(),
            audio_codec: Some("aac".into()),
            fps: Some(30.0),
            height: Some(1080),
            subtitle_tracks: 0,
        }
    }

    #[test]
    fn hevc_default_arguments() {
        let a =
            s(&build_args(&st(|_| {}), Some(&x265()), &facts("mov"), Path::new("/in/a.mov"), Path::new("/t/o.mp4")));
        let tail: Vec<&str> = a.iter().skip(8).map(String::as_str).collect();
        assert_eq!(
            tail,
            [
                "-i",
                "/in/a.mov",
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-map_metadata",
                "0",
                "-map_chapters",
                "0",
                "-c:v",
                "libx265",
                "-crf",
                "28",
                "-preset",
                "medium",
                "-x265-params",
                "log-level=error",
                "-tag:v",
                "hvc1",
                "-c:a",
                "copy",
                "-movflags",
                "+faststart",
                "-f",
                "mp4",
                "/t/o.mp4"
            ]
        );
    }

    #[test]
    fn av1_quality_scaling_fps_and_audio_conversion() {
        let set = st(|p| {
            p.set("codec", spec::t("av1"));
            p.set("quality", spec::t("small"));
            p.set("max_height", spec::t("720"));
            p.set("max_fps", spec::t("24"));
            p.set("drop_extra_audio", spec::b(true));
        });
        assert_eq!(set.crf, 38);
        let svt = Chosen { name: "libsvtav1".into(), hw: None };
        let mut f = facts("avi");
        f.audio_codec = Some("wmav2".into());
        f.fps = Some(60.0);
        let a = s(&build_args(&set, Some(&svt), &f, Path::new("a.avi"), Path::new("o.mp4")));
        assert!(a.windows(2).any(|w| w == ["-vf", "scale=-2:min(ih\\,720),fps=24"]), "{a:?}");
        assert!(
            a.windows(2).any(|w| w == ["-map", "0:a:0?"])
                && a.windows(2).any(|w| w == ["-c:a", "aac"])
                && a.contains(&"128k".to_string())
        );
        assert!(
            a.windows(2).any(|w| w == ["-c:v", "libsvtav1"])
                && a.windows(2).any(|w| w == ["-preset", "8"])
                && !a.contains(&"hvc1".to_string())
        );
        // Already small enough: no filters.
        let mut small = facts("mp4");
        small.height = Some(540);
        small.fps = Some(24.0);
        assert!(!s(&build_args(&set, Some(&svt), &small, Path::new("a"), Path::new("o"))).contains(&"-vf".to_string()));
    }

    #[test]
    fn remux_copies_streams_and_drops_the_rest() {
        let a = s(&build_args(
            &st(|p| p.set("remux_only", spec::b(true))),
            None,
            &facts("mkv"),
            Path::new("a.mkv"),
            Path::new("o.mkv"),
        ));
        assert!(
            a.windows(2).any(|w| w == ["-c", "copy"])
                && a.windows(2).any(|w| w == ["-map", "0:s?"])
                && a.contains(&"matroska".to_string())
        );
        assert!(!a.contains(&"-vf".to_string()) && !a.iter().any(|x| x.starts_with("-c:v")));
        let drop = s(&build_args(
            &st(|p| {
                p.set("remux_only", spec::b(true));
                p.set("drop_subtitles", spec::b(true));
            }),
            None,
            &facts("mp4"),
            Path::new("a"),
            Path::new("o"),
        ));
        assert!(!drop.contains(&"0:s?".to_string()) && drop.contains(&"+faststart".to_string()));
    }

    #[test]
    fn hardware_encoder_options() {
        let nv = Chosen { name: "hevc_nvenc".into(), hw: Some(HwKind::Nvenc) };
        let a = s(&build_args(&st(|_| {}), Some(&nv), &facts("mp4"), Path::new("a"), Path::new("o")));
        assert!(
            a.windows(2).any(|w| w == ["-c:v", "hevc_nvenc"])
                && a.windows(2).any(|w| w == ["-cq", "28"])
                && a.windows(2).any(|w| w == ["-preset", "p5"])
        );
        let q = Chosen { name: "av1_qsv".into(), hw: Some(HwKind::Qsv) };
        let a = s(&build_args(
            &st(|p| p.set("codec", spec::t("av1"))),
            Some(&q),
            &facts("mp4"),
            Path::new("a"),
            Path::new("o"),
        ));
        assert!(a.windows(2).any(|w| w == ["-global_quality", "33"]));
        let amf = Chosen { name: "hevc_amf".into(), hw: Some(HwKind::Amf) };
        assert!(s(&build_args(&st(|_| {}), Some(&amf), &facts("mp4"), Path::new("a"), Path::new("o")))
            .windows(2)
            .any(|w| w == ["-qp_i", "28"]));
    }

    #[test]
    fn trim_and_screen_tools() {
        let mut set = st(|p| p.set("codec", spec::t("av1")));
        set.trim = Some((12.5, Some(300.0)));
        set.screen_tools = true;
        let aom = Chosen { name: "libaom-av1".into(), hw: None };
        let a = s(&build_args(&set, Some(&aom), &facts("mp4"), Path::new("a"), Path::new("o")));
        assert!(
            a.windows(4).any(|w| w == ["-ss", "12.500", "-to", "300.000"])
                && a.windows(2).any(|w| w == ["-aom-params", "tune-content=screen"])
        );
        assert_eq!(screen_tool_args("libx265"), None);
    }

    #[test]
    fn encoder_selection() {
        let all = Encoders(["libx265", "libsvtav1", "libaom-av1", "hevc_nvenc", "av1_qsv"].map(String::from).to_vec());
        assert_eq!(all.choose(VCodec::Hevc, "auto").unwrap().name, "libx265");
        assert_eq!(
            all.choose(VCodec::Hevc, "nvenc").unwrap(),
            Chosen { name: "hevc_nvenc".into(), hw: Some(HwKind::Nvenc) }
        );
        assert_eq!(all.choose(VCodec::Hevc, "amf").unwrap().name, "libx265", "missing hardware falls back to software");
        assert_eq!(all.choose(VCodec::Av1, "auto").unwrap().name, "libsvtav1");
        assert_eq!(Encoders(vec!["libaom-av1".into()]).choose(VCodec::Av1, "auto").unwrap().name, "libaom-av1");
        assert!(Encoders(vec![]).choose(VCodec::Hevc, "auto").is_none());
        let hw: Vec<_> = all.hardware().into_iter().map(|h| h.name).collect();
        assert_eq!(hw, ["hevc_nvenc", "av1_qsv"]);
    }

    #[test]
    fn containers() {
        assert_eq!(container_for("mkv", false, false).ext, "mkv");
        assert_eq!(container_for("mov", false, false).ext, "mp4");
        assert_eq!(container_for("mov", true, false).ext, "mov");
        assert_eq!(container_for("avi", false, true).ext, "mkv");
        assert_eq!(container_for("wmv", false, false).ext, "mp4");
    }

    #[test]
    fn warnings_and_availability() {
        assert!(
            compatibility_warning(VCodec::Hevc).contains("HEVC") && compatibility_warning(VCodec::Av1).contains("AV1")
        );
        let codec = Video.info().settings.into_iter().find(|x| x.key == "codec").unwrap();
        match codec.kind {
            mj_core::SettingKind::Choice { options, .. } => assert!(options.iter().all(|o| o.help.is_some())),
            _ => panic!(),
        }
        assert_eq!(Video.availability(&[]), Availability::Disabled { reason: "Requires ffmpeg on PATH".into() });
        assert_eq!(Video.info().feature_ids.len(), 6);
    }

    #[test]
    fn encode_time_uses_metadata() {
        use mj_formats::media::build::mp4::*;
        let tail = moov(600_000, &[trak(b"vide", b"avc1", 1920, 1080, 30_000, 18_000_000, 18_000)]);
        let buf = file(b"isom", 1000, &[tail]);
        let n = buf.len() as u64;
        let i = mj_formats::media::probe_reader(std::io::Cursor::new(buf), n).unwrap();
        let base = st(|_| {});
        let half = st(|p| p.set("max_height", spec::t("540")));
        assert!(
            encode_time_estimate(&i, &half, "libx265").unwrap()
                < encode_time_estimate(&i, &base, "libx265").unwrap() / 3.0
        );
        assert!(encode_time_estimate(&i, &base, "hevc_nvenc").unwrap() < 200.0);
    }
}
