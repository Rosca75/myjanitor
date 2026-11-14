//! Screen recordings (OPT-SCR-01..05) through external ffmpeg.
//! Owner: W3-C.
//!
//! | feature | how |
//! |---|---|
//! | OPT-SCR-01 | [`detect_screen`]: metadata heuristics (bits per pixel, typical sizes, recorder names) |
//! | OPT-SCR-02 | `fps` 5 to 15, full resolution kept so text stays sharp |
//! | OPT-SCR-03 | `screen_tools`: AV1 screen-content coding, used only when the encoder accepts it |
//! | OPT-SCR-04 | `trim_dead_time`: [`suggest_cuts`] from `freezedetect` (and `silencedetect` when there is sound) |
//! | OPT-SCR-05 | `output = audio_only`: Opus voice derivative; key frames to a slide PDF is not offered (no PDF writer) |

use std::ffi::OsString;
use std::path::Path;

use mj_core::{
    ActionKind, Availability, CancelToken, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool,
    FileEntry, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport,
    Result, SafetyClass, SettingSpec, StepPreset,
};
use mj_formats::media::MediaInfo;

use super::estimate::{info_of, video_after, VideoPlan};
use super::ffmpeg::{base_args, clock, input_args, intersect, parse_freezes, parse_silences, run_analysis, run_encode};
use super::video::{run_video_job, VideoSettings};
use crate::external::{args, path_arg};
use crate::images::common::{self, spec};

/// Screen recordings: Low-motion video at a low frame rate, text stays sharp
#[derive(Debug, Default, Clone, Copy)]
pub struct ScreenRecordings;

/// Extensions (lower case) this optimiser targets.
pub const SCREENRECORDINGS_EXTENSIONS: &[&str] = &["mp4", "mkv", "webm", "mov"];

const MIN_SIZE: u64 = 2 * 1024 * 1024;

/// Screen resolutions that phones and cameras rarely produce.
const SCREEN_SIZES: &[(u32, u32)] = &[
    (1280, 720),
    (1366, 768),
    (1440, 900),
    (1536, 864),
    (1600, 900),
    (1680, 1050),
    (1920, 1080),
    (1920, 1200),
    (2048, 1152),
    (2560, 1080),
    (2560, 1440),
    (2560, 1600),
    (2880, 1800),
    (3024, 1964),
    (3440, 1440),
    (3456, 2234),
    (3840, 2160),
    (3840, 2400),
    (5120, 1440),
    (5120, 2880),
];

/// Recorder names in file names and encoder tags: `(needle, label)`.
const RECORDERS: &[(&str, &str)] = &[
    ("screen recording", "Screen Recording"),
    ("screenrecording", "Screen Recording"),
    ("screencast", "Screencast"),
    ("screen capture", "Screen Capture"),
    ("teams", "Teams"),
    ("zoom", "Zoom"),
    ("webex", "Webex"),
    ("obs", "OBS"),
    ("loom", "Loom"),
    ("camtasia", "Camtasia"),
    ("snagit", "Snagit"),
    ("kazam", "Kazam"),
    ("simplescreenrecorder", "SimpleScreenRecorder"),
    ("xbox game bar", "Game Bar"),
    ("capture", "Capture"),
];

/// Outcome of the screen-content heuristic.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenDetection {
    pub is_screen: bool,
    pub score: f32,
    pub reasons: Vec<String>,
    /// Recorder named by the file name or the encoder tag (`Teams`, `OBS`...).
    pub source_app: Option<String>,
}

fn recorder_in(text: &str) -> Option<&'static str> {
    let t = text.to_lowercase();
    RECORDERS
        .iter()
        .find(|(n, _)| match *n {
            // Short names must be whole words: "obs" inside "observations" is not OBS.
            "obs" | "zoom" | "teams" | "loom" => t.split(|c: char| !c.is_alphanumeric()).any(|w| w == *n),
            _ => t.contains(n),
        })
        .map(|(_, l)| *l)
}

/// OPT-SCR-01: decide from metadata alone whether a video is a screen capture. Needs a score of 2:
/// a recorder name (2), very low bits per pixel (up to 1.5), a typical screen size (1), a frame
/// rate of 30 or less (0.5).
pub fn detect_screen(path: &Path, info: &MediaInfo) -> ScreenDetection {
    let (mut score, mut reasons, mut app) = (0f32, Vec::new(), None);
    let name = mj_core::pathutil::file_name_lossy(path);
    if let Some(l) = recorder_in(&name) {
        score += 2.0;
        app = Some(l.to_string());
        reasons.push(format!("file name looks like a {l} recording"));
    }
    if let Some(l) = info.encoder.as_deref().and_then(recorder_in) {
        score += 2.0;
        app.get_or_insert_with(|| l.to_string());
        reasons.push(format!("written by {l}"));
    }
    if let (Some(w), Some(h)) = (info.width, info.height) {
        if SCREEN_SIZES.contains(&(w, h)) {
            score += 1.0;
            reasons.push(format!("{w}x{h} is a typical screen size"));
        }
        if let (Some(kbps), Some(fps)) = (info.bitrate_kbps, info.frame_rate.filter(|f| *f > 0.0)) {
            let bpp = f64::from(kbps) * 1000.0 / (f64::from(w) * f64::from(h) * f64::from(fps));
            if bpp < 0.012 {
                score += 1.5;
                reasons.push(format!("very little detail per frame ({bpp:.3} bits per pixel)"));
            } else if bpp < 0.03 {
                score += 1.0;
                reasons.push(format!("little detail per frame ({bpp:.3} bits per pixel)"));
            }
        }
        if info.frame_rate.is_some_and(|f| f <= 30.5) {
            score += 0.5;
        }
    }
    ScreenDetection { is_screen: score >= 2.0 && info.video_tracks > 0, score, reasons, source_app: app }
}

/// Suggested cuts for the timeline (OPT-SCR-04).
#[derive(Debug, Clone, PartialEq)]
pub struct Cuts {
    /// Every dead stretch (still and, when there is sound, silent), in milliseconds.
    pub segments: Vec<(u64, u64)>,
    /// Keep the video from here (0 = nothing to trim at the start).
    pub trim_start_s: f64,
    /// Keep the video up to here (`None` = nothing to trim at the end).
    pub trim_end_s: Option<f64>,
}

/// Turn detector output into cuts: dead = still, and silent too when `silences` is given. A dead
/// stretch touching the start or the end becomes a trim that keeps one second of it.
pub fn suggest_cuts(duration_s: f64, freezes: &[(f64, f64)], silences: Option<&[(f64, f64)]>) -> Cuts {
    let dead = match silences {
        Some(s) => intersect(freezes, s),
        None => freezes.to_vec(),
    };
    let keep = 1.0;
    let lead = dead.iter().find(|(s, e)| *s < 0.5 && e - s > keep).map_or(0.0, |(_, e)| (e - keep).max(0.0));
    let trail =
        dead.iter().rev().find(|(s, e)| *e > duration_s - 0.5 && e - s > keep).map(|(s, _)| (s + keep).min(duration_s));
    Cuts {
        segments: dead.iter().map(|(s, e)| ((s * 1000.0) as u64, (e * 1000.0) as u64)).collect(),
        trim_start_s: lead,
        trim_end_s: trail.filter(|t| *t > lead + keep),
    }
}

/// ffmpeg arguments of the detection pass (decodes the whole file; run it only when asked).
pub fn detect_args(src: &Path, with_audio: bool) -> Vec<OsString> {
    let mut a = args(["-nostdin", "-hide_banner", "-loglevel", "info", "-nostats"]);
    a.extend(input_args(src));
    a.extend(args(["-map", "0:v:0", "-vf", "freezedetect=n=0.003:d=3"]));
    if with_audio {
        a.extend(args(["-map", "0:a:0", "-af", "silencedetect=noise=-45dB:d=3"]));
    }
    a.extend(args(["-f", "null", "-"]));
    a
}

/// Run the detection pass and suggest cuts for `src`.
pub fn suggest_cuts_for(ffmpeg: &Path, src: &Path, info: &MediaInfo, cancel: &CancelToken) -> Result<Cuts> {
    let dur = info.duration_s().ok_or_else(|| Error::unsupported("trim", "the length of this video is unknown"))?;
    let with_audio = info.audio_tracks > 0;
    let err = run_analysis(ffmpeg, &detect_args(src, with_audio), cancel, None)?;
    let freezes = parse_freezes(&err, Some(dur));
    let silences = with_audio.then(|| parse_silences(&err, Some(dur)));
    Ok(suggest_cuts(dur, &freezes, silences.as_deref()))
}

/// Audio-only derivative: Opus voice at 32 kbit/s mono.
pub fn audio_only_args(src: &Path, out: &Path) -> Vec<OsString> {
    let mut a = base_args();
    a.extend(input_args(src));
    a.extend(args([
        "-map",
        "0:a:0",
        "-vn",
        "-ac",
        "1",
        "-c:a",
        "libopus",
        "-b:a",
        "32k",
        "-vbr",
        "on",
        "-application",
        "voip",
        "-f",
        "opus",
    ]));
    a.push(path_arg(out));
    a
}

/// Settings of a video run from the screen step's parameters.
pub fn video_settings(p: &ParamValues) -> VideoSettings {
    let mut s = VideoSettings::from_params(p);
    s.screen = true;
    s.max_height = 0; // keep the resolution: small text must stay legible
    s.max_fps = p.text("fps", "10").parse().unwrap_or(10.0);
    s.screen_tools = p.bool("screen_tools", false) && s.codec == super::video::VCodec::Av1;
    s
}

fn settings() -> Vec<SettingSpec> {
    vec![
        spec::choice(
            "fps",
            "Frame rate",
            "10",
            &[
                ("5", "5 fps", Some("Slides and documents")),
                ("10", "10 fps", Some("Apps and code")),
                ("15", "15 fps", Some("Scrolling and demos")),
            ],
            false,
        ),
        spec::choice(
            "codec",
            "Codec",
            "hevc",
            &[
                ("hevc", "HEVC (H.265)", Some(super::video::compatibility_warning(super::video::VCodec::Hevc))),
                ("av1", "AV1", Some(super::video::compatibility_warning(super::video::VCodec::Av1))),
            ],
            false,
        ),
        spec::choice(
            "quality",
            "Quality",
            "balanced",
            &[("high", "High", None), ("balanced", "Balanced", None), ("small", "Smaller", None)],
            false,
        ),
        spec::toggle(
            "trim_dead_time",
            "Trim still, silent start and end",
            Some("Finds the stretches where nothing moves and nobody speaks"),
            false,
            false,
            true,
        ),
        spec::choice(
            "output",
            "Result",
            "video",
            &[
                ("video", "Smaller video", None),
                ("audio_only", "Audio only", Some("Keeps the voice, drops the picture")),
            ],
            false,
        ),
        spec::toggle(
            "screen_tools",
            "AV1 screen-content tools",
            Some("Only with AV1 and an encoder that supports them"),
            false,
            true,
            false,
        ),
        spec::choice(
            "speed",
            "Speed",
            "medium",
            &[("fast", "Fast", None), ("medium", "Balanced", None), ("slow", "Slow, smaller", None)],
            true,
        ),
    ]
}

fn presets() -> Vec<StepPreset> {
    use spec::{b, t};
    vec![
        spec::preset(
            "standard",
            "Screen · 10 fps",
            "HEVC at full size · text stays sharp",
            true,
            &[("codec", t("hevc")), ("fps", t("10")), ("output", t("video"))],
        ),
        spec::preset(
            "tiny",
            "Screen · 5 fps AV1",
            "Smallest · newest players only",
            true,
            &[("codec", t("av1")), ("fps", t("5")), ("screen_tools", b(true)), ("output", t("video"))],
        ),
        spec::preset("audio", "Audio only", "Opus voice · drops the picture", true, &[("output", t("audio_only"))]),
    ]
}

impl Optimiser for ScreenRecordings {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ScreenRecordings,
            title: "Screen recordings".into(),
            description: "Low-motion video at a low frame rate, text stays sharp".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: presets(),
            settings: settings(),
            platform: PlatformSupport::Both,
            tools: vec!["ffmpeg".into()],
            feature_ids: (1..=5).map(|i| format!("OPT-SCR-0{i}")).collect(),
            unit: "videos".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        crate::tools::availability(tools, &["ffmpeg"])
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && SCREENRECORDINGS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let info = info_of(&entry.path);
        let s = video_settings(params);
        let mut plan: VideoPlan = s.plan();
        plan.audio_only = params.text("output", "video") == "audio_only";
        let after = video_after(info.as_ref(), entry.size, &plan);
        Ok(Estimate {
            files: 1,
            bytes_before: entry.size,
            bytes_after: after,
            confidence: mj_core::Confidence::Estimated,
            skipped: 0,
        })
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        job.cancel.checkpoint()?;
        if job.params.text("output", "video") == "audio_only" {
            return audio_only(job);
        }
        let mut s = video_settings(job.params);
        let mut notes = vec![format!("{} fps, full resolution", s.max_fps)];
        if job.params.bool("trim_dead_time", false) {
            let exe = crate::tools::require(job.tools, "ffmpeg")?;
            if let Some(info) = info_of(job.src) {
                let cuts = suggest_cuts_for(exe, job.src, &info, job.cancel)?;
                if cuts.trim_start_s > 0.0 || cuts.trim_end_s.is_some() {
                    notes.push(format!(
                        "trimmed from {} to {}",
                        clock(cuts.trim_start_s),
                        cuts.trim_end_s.map_or("the end".to_string(), clock)
                    ));
                    s.trim = Some((cuts.trim_start_s, cuts.trim_end_s));
                } else {
                    notes.push("no still and silent start or end found".into());
                }
            }
        }
        run_video_job(job, &mut s, notes)
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        common::analyse_entries(
            ctx,
            OptimiserId::ScreenRecordings,
            MIN_SIZE,
            &|e| self.applies_to(e),
            &|e, p| {
                let info = info_of(&e.path).ok_or_else(|| Error::unsupported("screen", "unreadable metadata"))?;
                if !detect_screen(&e.path, &info).is_screen {
                    return Err(Error::unsupported("screen", "not a screen recording"));
                }
                self.estimate(e, p)
            },
            &|e, _| {
                let info = info_of(&e.path);
                let det = info.as_ref().map(|i| detect_screen(&e.path, i));
                let mut columns = std::collections::BTreeMap::new();
                if let Some(app) = det.as_ref().and_then(|d| d.source_app.clone()) {
                    columns.insert("source".to_string(), app);
                }
                if let Some(d) = info.as_ref().and_then(MediaInfo::duration_s) {
                    columns.insert("length".to_string(), clock(d));
                }
                let reason = det
                    .and_then(|d| d.reasons.into_iter().next())
                    .unwrap_or_else(|| "Looks like a screen recording".into());
                common::Described { reason, columns, sub_group: Some("screen".into()) }
            },
        )
    }
}

/// OPT-SCR-05: the voice as a small Opus file.
fn audio_only(job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
    let exe = crate::tools::require(job.tools, "ffmpeg")?;
    let before = std::fs::metadata(job.src).map_err(|e| Error::io(job.src, e))?.len();
    let info = info_of(job.src);
    if info.as_ref().is_some_and(|i| i.audio_tracks == 0) {
        return Ok(common::skipped(before, "The recording has no sound to keep"));
    }
    let out = job.temp_dir.join("mj-screen-audio.opus");
    let _ = std::fs::remove_file(&out);
    run_encode(job, exe, &audio_only_args(job.src, &out), info.as_ref().and_then(|i| i.duration_ms))?;
    let after = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    if after == 0 {
        return Err(super::ffmpeg::empty_output("ffmpeg"));
    }
    if after >= before {
        let _ = std::fs::remove_file(&out);
        return Ok(common::skipped(before, "The audio file would not be smaller"));
    }
    common::move_into_place(&out, job.dst)?;
    Ok(common::done(
        before,
        after,
        true,
        vec!["output_extension=opus".into(), "audio only: the picture is not kept".into()],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_formats::media::build::mp4::*;

    fn video(name_dims: (u16, u16), fps_frames: u32, mdat: usize, tags: &[Vec<u8>]) -> MediaInfo {
        let mut kids = vec![
            trak(b"vide", b"avc1", name_dims.0, name_dims.1, 1000, 60_000, fps_frames * 60),
            trak(b"soun", b"mp4a", 0, 0, 48_000, 2_880_000, 2800),
        ];
        kids.extend_from_slice(tags);
        let buf = file(b"isom", mdat, &[moov(60_000, &kids)]);
        let n = buf.len() as u64;
        mj_formats::media::probe_reader(std::io::Cursor::new(buf), n).unwrap()
    }

    #[test]
    fn low_detail_at_a_screen_size_is_detected() {
        let i = video((1920, 1080), 30, 2_000_000, &[]);
        let d = detect_screen(Path::new("/x/clip.mp4"), &i);
        assert!(d.is_screen && d.score >= 2.0 && d.reasons.iter().any(|r| r.contains("bits per pixel")), "{d:?}");
    }

    #[test]
    fn phone_video_is_not_a_screen_recording() {
        let i = video((1920, 1080), 30, 120_000_000, &[]);
        assert!(!detect_screen(Path::new("/x/IMG_0001.MOV"), &i).is_screen);
        let odd = video((1080, 1920), 30, 120_000_000, &[]);
        assert!(!detect_screen(Path::new("/x/IMG_0002.MOV"), &odd).is_screen);
    }

    #[test]
    fn recorder_names_decide_on_their_own() {
        let phone_like = video((1280, 960), 30, 120_000_000, &[]);
        for (name, app) in [
            ("Zoom Meeting 2026-05-01.mp4", "Zoom"),
            ("Screen Recording 2026-05-01 at 10.00.mov", "Screen Recording"),
            ("weekly sync (Teams).mp4", "Teams"),
            ("OBS 2026.mkv", "OBS"),
        ] {
            let d = detect_screen(Path::new(name), &phone_like);
            assert!(d.is_screen && d.source_app.as_deref() == Some(app), "{name}: {d:?}");
        }
        assert!(!detect_screen(Path::new("observations.mp4"), &phone_like).is_screen, "'obs' only as a whole word");
        let tagged = video((1280, 960), 30, 120_000_000, &[udta_ilst(&[ilst_text(*b"\xA9too", "OBS Studio 30.1")])]);
        let d = detect_screen(Path::new("a.mp4"), &tagged);
        assert!(d.is_screen && d.source_app.as_deref() == Some("OBS"));
    }

    #[test]
    fn cuts_trim_still_and_silent_ends_and_keep_a_second() {
        let c = suggest_cuts(
            100.0,
            &[(0.0, 10.0), (50.0, 60.0), (90.0, 100.0)],
            Some(&[(0.0, 9.0), (55.0, 58.0), (92.0, 100.0)]),
        );
        assert_eq!(c.segments, [(0, 9000), (55_000, 58_000), (92_000, 100_000)]);
        assert_eq!((c.trim_start_s, c.trim_end_s), (8.0, Some(93.0)));
        // Without sound, stillness alone counts.
        let c = suggest_cuts(100.0, &[(0.0, 10.0)], None);
        assert_eq!((c.trim_start_s, c.trim_end_s), (9.0, None));
        // Still in the middle only: nothing to trim, but the timeline shows it.
        let c = suggest_cuts(100.0, &[(40.0, 60.0)], None);
        assert_eq!((c.trim_start_s, c.trim_end_s, c.segments.len()), (0.0, None, 1));
        // Whole video still: leave it alone rather than cut to nothing.
        assert_eq!(suggest_cuts(5.0, &[(0.0, 5.0)], None).trim_end_s, None);
    }

    #[test]
    fn settings_become_a_screen_video_plan() {
        let p = ParamValues::new()
            .with("codec", spec::t("av1"))
            .with("fps", spec::t("5"))
            .with("screen_tools", spec::b(true));
        let s = video_settings(&p);
        assert!(s.screen && s.screen_tools && s.max_fps == 5.0 && s.max_height == 0);
        assert!(
            !video_settings(&ParamValues::new().with("screen_tools", spec::b(true))).screen_tools,
            "HEVC has no screen tools"
        );
        let presets = presets();
        assert_eq!(presets.len(), 3);
    }

    #[test]
    fn estimate_is_far_below_the_video_estimate() {
        let i = video((1920, 1080), 30, 100_000_000, &[]);
        let screen = video_after(Some(&i), i.file_size, &video_settings(&ParamValues::new()).plan());
        let plain = video_after(Some(&i), i.file_size, &VideoSettings::from_params(&ParamValues::new()).plan());
        assert!(screen * 2 < plain, "{screen} vs {plain}");
    }

    #[test]
    fn audio_only_arguments_and_dash_guard() {
        let a: Vec<String> = audio_only_args(Path::new("-a.mp4"), Path::new("/t/o.opus"))
            .iter()
            .map(|x| x.to_string_lossy().into_owned())
            .collect();
        assert!(
            a.contains(&"./-a.mp4".to_string())
                && a.windows(2).any(|w| w == ["-b:a", "32k"])
                && a.contains(&"-vn".to_string())
        );
        let d: Vec<String> =
            detect_args(Path::new("a.mp4"), true).iter().map(|x| x.to_string_lossy().into_owned()).collect();
        assert!(
            d.contains(&"freezedetect=n=0.003:d=3".to_string())
                && d.contains(&"silencedetect=noise=-45dB:d=3".to_string())
        );
        assert!(!detect_args(Path::new("a.mp4"), false).iter().any(|x| x == "-af"));
    }

    #[test]
    fn disabled_without_ffmpeg() {
        assert_eq!(
            ScreenRecordings.availability(&[]),
            Availability::Disabled { reason: "Requires ffmpeg on PATH".into() }
        );
        assert_eq!(ScreenRecordings.info().feature_ids.len(), 5);
    }
}
