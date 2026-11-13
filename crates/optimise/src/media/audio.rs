//! Audio and meeting recordings (OPT-AUD-01..08) through external ffmpeg.
//! Owner: W3-C.
//!
//! | feature | how |
//! |---|---|
//! | OPT-AUD-01 | `codec`: `flac` (lossless), `opus` or `aac` (lossy) |
//! | OPT-AUD-02 | FLAC output always uses compression level 12 |
//! | OPT-AUD-03 | `strip_cover_art`, or `cover_max_px` to downsize it |
//! | OPT-AUD-04 / 07 | presets `speech` (Opus 24 kbps mono) and `transcription_safe` (Opus 48 kbps) |
//! | OPT-AUD-05 | `downmix_mono` |
//! | OPT-AUD-06 | `remove_silences` + `max_pause_s` (`silenceremove`), [`preview`] lists the removable parts |
//! | OPT-AUD-08 | disabled: no transcription tool is bundled |

use std::ffi::OsString;
use std::path::Path;

use mj_core::{
    ActionKind, Availability, CancelToken, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool,
    FileEntry, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport,
    Result, SafetyClass, SettingSpec, StepPreset,
};

use super::estimate::{audio_after, info_of, AudioPlan};
use super::ffmpeg::{base_args, clock, input_args, parse_silences, run_analysis, run_encode};
use crate::external::{args, path_arg, run, RunOptions};
use crate::images::common::{self, spec};

/// Meeting recordings: Speech preset, silences trimmed
#[derive(Debug, Default, Clone, Copy)]
pub struct Audio;

/// Extensions (lower case) this optimiser targets.
pub const AUDIO_EXTENSIONS: &[&str] = &["wav", "m4a", "mp3", "aiff", "aif", "flac", "wma"];

const MIN_SIZE: u64 = 256 * 1024;

/// Output codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Opus,
    Aac,
    Flac,
}

impl Codec {
    /// File extension of the output.
    pub fn extension(self) -> &'static str {
        match self {
            Codec::Opus => "opus",
            Codec::Aac => "m4a",
            Codec::Flac => "flac",
        }
    }
}

/// Everything the argument builder needs, read from the step parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioSettings {
    pub codec: Codec,
    pub bitrate_kbps: u32,
    pub mono: bool,
    pub remove_silences: bool,
    pub max_pause_s: f64,
    pub silence_db: f64,
    pub strip_cover: bool,
    /// Downsize kept cover art to this many pixels (0 = keep as is).
    pub cover_max_px: u32,
}

impl AudioSettings {
    /// Read the parameters (missing keys take the Speech preset's values).
    pub fn from_params(p: &ParamValues) -> AudioSettings {
        let codec = match p.text("codec", "opus") {
            "flac" => Codec::Flac,
            "aac" => Codec::Aac,
            _ => Codec::Opus,
        };
        AudioSettings {
            codec,
            bitrate_kbps: p.number("bitrate_kbps", 24.0).clamp(6.0, 320.0) as u32,
            mono: p.bool("downmix_mono", true),
            remove_silences: p.bool("remove_silences", true),
            max_pause_s: p.number("max_pause_s", 2.0).clamp(0.5, 60.0),
            silence_db: p.number("silence_db", -45.0).clamp(-90.0, -10.0),
            strip_cover: p.bool("strip_cover_art", true),
            cover_max_px: p.number("cover_max_px", 0.0).clamp(0.0, 4096.0) as u32,
        }
    }

    /// True when no sound is changed: FLAC, same channels, nothing cut.
    pub fn is_lossless(&self) -> bool {
        self.codec == Codec::Flac && !self.mono && !self.remove_silences
    }
}

/// The `silenceremove` filter: pauses longer than `max_pause_s` are cut down to `max_pause_s`.
pub fn silence_filter(s: &AudioSettings) -> String {
    format!("silenceremove=stop_periods=-1:stop_duration={d}:stop_threshold={db}dB:stop_silence={d}", d = s.max_pause_s, db = s.silence_db)
}

/// Full ffmpeg argument vector for one file.
pub fn build_args(s: &AudioSettings, src: &Path, out: &Path) -> Vec<OsString> {
    let mut a = base_args();
    a.extend(input_args(src));
    a.extend(args(["-map", "0:a:0"]));
    let keeps_cover = !s.strip_cover && s.codec != Codec::Opus; // Ogg Opus has no attached-picture stream
    if keeps_cover {
        a.extend(args(["-map", "0:v:0?"]));
        if s.cover_max_px > 0 {
            a.extend(args(["-c:v", "mjpeg", "-q:v", "4", "-vf", &format!("scale='min(iw,{n})':-2", n = s.cover_max_px)]));
        } else {
            a.extend(args(["-c:v", "copy"]));
        }
        a.extend(args(["-disposition:v:0", "attached_pic"]));
    } else {
        a.push("-vn".into());
    }
    a.extend(args(["-map_metadata", "0"]));
    if s.mono {
        a.extend(args(["-ac", "1"]));
    }
    if s.remove_silences {
        a.extend(args(["-af", &silence_filter(s)]));
    }
    match s.codec {
        Codec::Opus => {
            a.extend(args(["-c:a", "libopus", "-b:a", &format!("{}k", s.bitrate_kbps), "-vbr", "on"]));
            if s.bitrate_kbps <= 32 && s.mono {
                a.extend(args(["-application", "voip"])); // tuned for speech at low rates
            }
            a.extend(args(["-f", "opus"]));
        }
        Codec::Aac => a.extend(args(["-c:a", "aac", "-b:a", &format!("{}k", s.bitrate_kbps), "-movflags", "+faststart", "-f", "ipod"])),
        Codec::Flac => a.extend(args(["-c:a", "flac", "-compression_level", "12", "-f", "flac"])),
    }
    a.push(path_arg(out));
    a
}

/// `silencedetect` pass over the first audio stream.
pub fn detect_args(src: &Path, noise_db: f64, min_s: f64) -> Vec<OsString> {
    let mut a = args(["-nostdin", "-hide_banner", "-loglevel", "info", "-nostats"]);
    a.extend(input_args(src));
    a.extend(args(["-map", "0:a:0", "-af", &format!("silencedetect=noise={noise_db}dB:d={min_s}"), "-f", "null", "-"]));
    a
}

/// The parts of the recording a run with `s` would cut, as `(start_ms, end_ms)`.
pub fn removable_segments(silences: &[(f64, f64)], s: &AudioSettings) -> Vec<(u64, u64)> {
    silences
        .iter()
        .filter(|(a, b)| b - a > s.max_pause_s)
        .map(|(a, b)| (((a + s.max_pause_s) * 1000.0) as u64, (b * 1000.0) as u64))
        .collect()
}

/// Waveform preview (IPC `Preview`): bars, total length and the removable silences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveformPreview {
    pub peaks: Vec<u8>,
    pub duration_ms: u64,
    pub silences: Vec<(u64, u64)>,
}

/// Bucket 8-bit unsigned samples into `n` peak heights (0-255).
pub fn peaks_from_u8(samples: &[u8], n: usize) -> Vec<u8> {
    if samples.is_empty() || n == 0 {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let (a, b) = (i * samples.len() / n, ((i + 1) * samples.len() / n).max(i * samples.len() / n + 1));
            let m = samples[a..b.min(samples.len())].iter().map(|&x| (i32::from(x) - 128).unsigned_abs()).max().unwrap_or(0);
            (m * 2).min(255) as u8
        })
        .collect()
}

/// Decode a low-rate mono copy for the bars and run `silencedetect` for the removable parts.
pub fn preview(ffmpeg: &Path, src: &Path, params: &ParamValues, cancel: &CancelToken) -> Result<WaveformPreview> {
    let s = AudioSettings::from_params(params);
    let mut a = args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    a.extend(input_args(src));
    a.extend(args(["-map", "0:a:0", "-ac", "1", "-ar", "1000", "-f", "u8", "-"]));
    let out = run(ffmpeg, &a, &RunOptions { cancel: Some(cancel), max_stdout: 16 << 20, ..Default::default() })?;
    out.ensure_success("ffmpeg")?;
    let duration_ms = info_of(src).and_then(|i| i.duration_ms).unwrap_or(out.stdout.len() as u64);
    let err = run_analysis(ffmpeg, &detect_args(src, s.silence_db, s.max_pause_s), cancel, None)?;
    let silences = if s.remove_silences { removable_segments(&parse_silences(&err, Some(duration_ms as f64 / 1000.0)), &s) } else { Vec::new() };
    Ok(WaveformPreview { peaks: peaks_from_u8(&out.stdout, 200), duration_ms, silences })
}

fn settings() -> Vec<SettingSpec> {
    let mut transcribe = spec::toggle("transcribe", "Transcribe on this PC, then archive audio", Some("Keeps a searchable text file next to each meeting"), false, true, false);
    transcribe.disabled_reason = Some("Requires a local transcription tool (none is bundled)".into());
    vec![
        spec::choice(
            "codec",
            "Format",
            "opus",
            &[("opus", "Opus", Some("Smallest for speech")), ("aac", "AAC (.m4a)", Some("Plays everywhere")), ("flac", "FLAC", Some("Lossless"))],
            true,
        ),
        spec::number("bitrate_kbps", "Bitrate", 24.0, 6.0, 320.0, 2.0, Some("kbps"), true),
        spec::toggle("downmix_mono", "Downmix to mono", None, true, false, true),
        spec::toggle("remove_silences", "Remove long silences", None, true, false, true),
        spec::number("max_pause_s", "Max pause", 2.0, 1.0, 30.0, 1.0, Some("s max"), false),
        spec::toggle("strip_cover_art", "Strip cover art", None, true, false, false),
        spec::number("cover_max_px", "Shrink kept cover art to", 0.0, 0.0, 4096.0, 100.0, Some("px (0 = keep)"), true),
        spec::number("silence_db", "Silence threshold", -45.0, -90.0, -10.0, 1.0, Some("dB"), true),
        transcribe,
    ]
}

fn presets() -> Vec<StepPreset> {
    use spec::{b, n, t};
    vec![
        spec::preset("speech", "Speech", "Opus 24 kbps mono · smallest", true, &[("codec", t("opus")), ("bitrate_kbps", n(24.0)), ("downmix_mono", b(true)), ("remove_silences", b(true))]),
        spec::preset("transcription_safe", "Transcription-safe", "Opus 48 kbps · keeps detail for speech-to-text", true, &[("codec", t("opus")), ("bitrate_kbps", n(48.0)), ("downmix_mono", b(false)), ("remove_silences", b(false))]),
        spec::preset("lossless", "Lossless", "FLAC · no quality loss, ~45 % smaller", false, &[("codec", t("flac")), ("downmix_mono", b(false)), ("remove_silences", b(false))]),
    ]
}

impl Optimiser for Audio {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::Audio,
            title: "Meeting recordings".into(),
            description: "Speech preset, silences trimmed".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: presets(),
            settings: settings(),
            platform: PlatformSupport::Both,
            tools: vec!["ffmpeg".into()],
            feature_ids: (1..=8).map(|i| format!("OPT-AUD-0{i}")).collect(),
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        crate::tools::availability(tools, &["ffmpeg"])
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && AUDIO_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let s = AudioSettings::from_params(params);
        let plan = AudioPlan { lossy_kbps: (s.codec != Codec::Flac).then_some(s.bitrate_kbps), mono: s.mono, strip_cover: s.strip_cover };
        let after = audio_after(info_of(&entry.path).as_ref(), entry.size, &plan);
        Ok(Estimate { files: 1, bytes_before: entry.size, bytes_after: after.min(entry.size), confidence: mj_core::Confidence::Estimated, skipped: 0 })
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        job.cancel.checkpoint()?;
        let exe = crate::tools::require(job.tools, "ffmpeg")?;
        let before = std::fs::metadata(job.src).map_err(|e| Error::io(job.src, e))?.len();
        let s = AudioSettings::from_params(job.params);
        let info = info_of(job.src);
        let mut notes = vec![format!("output_extension={}", s.codec.extension())];
        let mut lossy = !s.is_lossless();
        if s.codec == Codec::Flac && info.as_ref().and_then(|i| i.audio_codec.as_deref()) == Some("pcm_float") {
            lossy = true; // FLAC stores integers: floating-point samples are rounded
            notes.push("floating-point samples were rounded to 24 bits".into());
        }
        let out = job.temp_dir.join(format!("mj-audio-out.{}", s.codec.extension()));
        let _ = std::fs::remove_file(&out);
        let dur = info.as_ref().and_then(|i| i.duration_ms);
        run_encode(job, exe, &build_args(&s, job.src, &out), dur)?;
        let after = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        if after == 0 {
            return Err(super::ffmpeg::empty_output("ffmpeg"));
        }
        // A run that cuts nothing must keep the length (within a second or 2 %).
        if let (false, Some(d_in), Some(d_out)) = (s.remove_silences, dur, info_of(&out).and_then(|i| i.duration_ms)) {
            if d_in.abs_diff(d_out) > (d_in / 50).max(1000) {
                let _ = std::fs::remove_file(&out);
                return Err(Error::Internal(format!("the converted file is {} long but the original is {}; nothing was changed", clock(d_out as f64 / 1000.0), clock(d_in as f64 / 1000.0))));
            }
        }
        if after >= before {
            let _ = std::fs::remove_file(&out);
            return Ok(common::skipped(before, "The converted recording would not be smaller"));
        }
        common::move_into_place(&out, job.dst)?;
        Ok(common::done(before, after, lossy, notes))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        common::analyse_entries(
            ctx,
            OptimiserId::Audio,
            MIN_SIZE,
            &|e| self.applies_to(e),
            &|e, p| self.estimate(e, p),
            &|e, _| {
                let info = info_of(&e.path);
                let mut columns = std::collections::BTreeMap::new();
                if let Some(d) = info.as_ref().and_then(|i| i.duration_s()) {
                    columns.insert("length".to_string(), clock(d));
                }
                common::Described { reason: "Recording can be stored in much less space".into(), columns, sub_group: None }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[OsString]) -> Vec<String> {
        v.iter().map(|x| x.to_string_lossy().into_owned()).collect()
    }

    fn settings_of(preset: &str) -> AudioSettings {
        let p = presets().into_iter().find(|p| p.id == preset).unwrap();
        AudioSettings::from_params(&p.values)
    }

    #[test]
    fn speech_preset_arguments() {
        let a = s(&build_args(&settings_of("speech"), Path::new("/in/a.wav"), Path::new("/t/o.opus")));
        let tail: Vec<&str> = a.iter().skip(8).map(String::as_str).collect();
        assert_eq!(
            tail,
            [
                "-i", "/in/a.wav", "-map", "0:a:0", "-vn", "-map_metadata", "0", "-ac", "1", "-af",
                "silenceremove=stop_periods=-1:stop_duration=2:stop_threshold=-45dB:stop_silence=2",
                "-c:a", "libopus", "-b:a", "24k", "-vbr", "on", "-application", "voip", "-f", "opus", "/t/o.opus"
            ]
        );
        assert_eq!(&a[..2], ["-nostdin", "-hide_banner"]);
    }

    #[test]
    fn transcription_safe_keeps_stereo_and_pauses() {
        let a = s(&build_args(&settings_of("transcription_safe"), Path::new("/in/a.wav"), Path::new("/t/o.opus")));
        assert!(a.contains(&"48k".to_string()) && !a.contains(&"-ac".to_string()) && !a.contains(&"-af".to_string()) && !a.contains(&"voip".to_string()));
    }

    #[test]
    fn lossless_preset_is_flac_level_12_and_lossless() {
        let st = settings_of("lossless");
        assert!(st.is_lossless());
        let a = s(&build_args(&st, Path::new("/in/a.wav"), Path::new("/t/o.flac")));
        assert!(a.windows(2).any(|w| w == ["-compression_level", "12"]) && a.contains(&"flac".to_string()));
        assert!(!settings_of("speech").is_lossless() && !settings_of("transcription_safe").is_lossless());
    }

    #[test]
    fn cover_art_options() {
        let mut st = settings_of("lossless");
        st.strip_cover = false;
        let keep = s(&build_args(&st, Path::new("a.flac"), Path::new("o.flac")));
        assert!(keep.windows(2).any(|w| w == ["-c:v", "copy"]) && keep.contains(&"attached_pic".to_string()));
        st.cover_max_px = 300;
        let small = s(&build_args(&st, Path::new("a.flac"), Path::new("o.flac")));
        assert!(small.iter().any(|x| x == "scale='min(iw,300)':-2") && small.contains(&"mjpeg".to_string()));
        st.codec = Codec::Aac;
        let aac = s(&build_args(&st, Path::new("a.m4a"), Path::new("o.m4a")));
        assert!(aac.windows(2).any(|w| w == ["-c:a", "aac"]) && aac.contains(&"ipod".to_string()));
        st.codec = Codec::Opus;
        assert!(s(&build_args(&st, Path::new("a"), Path::new("o"))).contains(&"-vn".to_string()), "Opus never carries cover art");
    }

    #[test]
    fn option_paths_with_dashes_are_guarded() {
        let a = s(&build_args(&settings_of("speech"), Path::new("-evil.wav"), Path::new("-o.opus")));
        assert!(a.contains(&"./-evil.wav".to_string()) && a.contains(&"./-o.opus".to_string()));
    }

    #[test]
    fn parameters_are_clamped() {
        let p = ParamValues::new().with("bitrate_kbps", spec::n(9999.0)).with("max_pause_s", spec::n(0.0)).with("codec", spec::t("weird"));
        let st = AudioSettings::from_params(&p);
        assert_eq!((st.bitrate_kbps, st.max_pause_s, st.codec), (320, 0.5, Codec::Opus));
    }

    #[test]
    fn removable_segments_keep_the_allowed_pause() {
        let st = settings_of("speech");
        assert_eq!(removable_segments(&[(1.0, 2.5), (10.0, 16.0)], &st), [(12_000, 16_000)]);
    }

    #[test]
    fn peaks_bucket_samples() {
        let mut v = vec![128u8; 100];
        v[10] = 228;
        v[90] = 0;
        let p = peaks_from_u8(&v, 10);
        assert_eq!(p.len(), 10);
        assert_eq!((p[1], p[9], p[5]), (200, 255, 0));
        assert!(peaks_from_u8(&[], 10).is_empty());
    }

    #[test]
    fn disabled_without_ffmpeg_and_transcription_is_disabled_with_reason() {
        assert_eq!(Audio.availability(&[]), Availability::Disabled { reason: "Requires ffmpeg on PATH".into() });
        let t = Audio.info().settings.into_iter().find(|x| x.key == "transcribe").unwrap();
        assert!(t.disabled_reason.unwrap().starts_with("Requires"));
        assert!(Audio.info().feature_ids.contains(&"OPT-AUD-08".to_string()));
    }
}
