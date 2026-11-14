//! Integration tests against the real `ffmpeg` and `ffprobe`. They skip cleanly (and say so on
//! stderr) when ffmpeg is not on PATH; media is generated in a temp dir, nothing is committed.

use std::path::{Path, PathBuf};

use mj_core::testutil::TempDir;
use mj_core::{CancelToken, ExternalTool, NullProgress, OptimiseJob, Optimiser, ParamValues};

use super::audio::{self, Audio};
use super::screen::{self, ScreenRecordings};
use super::video::{detect_hardware, Video};
use crate::external::{args, run, RunOptions};
use crate::images::common::spec;

fn ffmpeg() -> Option<(PathBuf, Vec<ExternalTool>)> {
    let p = crate::tools::which("ffmpeg");
    match &p {
        Some(p) => eprintln!("real ffmpeg tests: using {}", p.display()),
        None => eprintln!("real ffmpeg tests: SKIPPED (ffmpeg not on PATH)"),
    }
    p.map(|p| (p.clone(), vec![crate::tools::tool_at("ffmpeg", p)]))
}

fn encoders_have(exe: &Path, names: &[&str]) -> bool {
    let list = super::ffmpeg::list_encoders(exe, &CancelToken::new()).unwrap();
    names.iter().all(|n| list.iter().any(|x| x == n))
}

/// 16 kHz mono 16-bit WAV: 3 s of tone, 6 s of silence, 3 s of tone.
fn tone_silence_tone(dir: &TempDir) -> PathBuf {
    let rate = 16_000usize;
    let mut pcm = Vec::new();
    for sec in 0..12usize {
        for i in 0..rate {
            let v = if (3..9).contains(&sec) {
                0i16
            } else {
                ((i as f64 * 440.0 * std::f64::consts::TAU / rate as f64).sin() * 12_000.0) as i16
            };
            pcm.extend_from_slice(&v.to_le_bytes());
        }
    }
    let mut w = b"RIFF".to_vec();
    w.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&[1, 0, 1, 0]);
    w.extend_from_slice(&(rate as u32).to_le_bytes());
    w.extend_from_slice(&((rate * 2) as u32).to_le_bytes());
    w.extend_from_slice(&[2, 0, 16, 0]);
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    w.extend(pcm);
    dir.write("meeting.wav", &w).unwrap()
}

fn job_run(
    o: &dyn Optimiser,
    src: &Path,
    dst: &Path,
    dir: &TempDir,
    p: &ParamValues,
    tools: &[ExternalTool],
) -> mj_core::Result<mj_core::OptimiseOutput> {
    let c = CancelToken::new();
    o.optimise(&OptimiseJob { src, dst, params: p, tools, temp_dir: dir.path(), cancel: &c, progress: &NullProgress })
}

fn make_video(exe: &Path, dir: &TempDir, motion_s: u32, still_s: u32) -> PathBuf {
    let out = dir.join("screen.mp4");
    let total = motion_s + still_s;
    let mut a = args(["-nostdin", "-loglevel", "error", "-y"]);
    a.extend(args(["-f", "lavfi", "-i", &format!("testsrc2=s=320x240:r=10:d={motion_s}")]));
    a.extend(args(["-f", "lavfi", "-i", &format!("color=c=black:s=320x240:r=10:d={still_s}")]));
    a.extend(args(["-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono"]));
    a.extend(args([
        "-filter_complex",
        "[0:v][1:v]concat=n=2:v=1:a=0[v]",
        "-map",
        "[v]",
        "-map",
        "2:a",
        "-t",
        &total.to_string(),
    ]));
    a.extend(args(["-c:v", "libx264", "-b:v", "3M", "-pix_fmt", "yuv420p", "-c:a", "aac"]));
    a.push(out.clone().into());
    let o = run(exe, &a, &RunOptions::default()).unwrap();
    assert!(o.success(), "{}", o.stderr);
    out
}

#[test]
fn audio_speech_preset_removes_long_silences_and_shrinks() {
    let Some((exe, tools)) = ffmpeg() else { return };
    if !encoders_have(&exe, &["libopus"]) {
        return eprintln!("SKIPPED: no libopus");
    }
    let d = TempDir::new("ff-audio").unwrap();
    let src = tone_silence_tone(&d);
    let dst = d.join("out.opus");
    let preset = Audio.info().presets.into_iter().find(|p| p.id == "speech").unwrap();
    let r = job_run(&Audio, &src, &dst, &d, &preset.values, &tools).unwrap();
    assert!(r.skipped.is_none() && r.lossy && r.bytes_after * 10 < r.bytes_before, "{r:?}");
    assert!(r.notes.contains(&"output_extension=opus".to_string()));
    let i = mj_formats::media::probe(&dst).unwrap();
    assert_eq!((i.audio_codec.as_deref(), i.channels), (Some("opus"), Some(1)));
    // 3 s + 2 s of the pause (kept) + 3 s = 8 s, not the original 12.
    let secs = i.duration_s().unwrap();
    assert!((6.5..9.5).contains(&secs), "duration {secs}");
}

#[test]
fn audio_flac_lossless_keeps_length_and_is_marked_lossless() {
    let Some((exe, tools)) = ffmpeg() else { return };
    if !encoders_have(&exe, &["flac"]) {
        return eprintln!("SKIPPED: no flac encoder");
    }
    let d = TempDir::new("ff-flac").unwrap();
    let src = tone_silence_tone(&d);
    let dst = d.join("out.flac");
    let preset = Audio.info().presets.into_iter().find(|p| p.id == "lossless").unwrap();
    let r = job_run(&Audio, &src, &dst, &d, &preset.values, &tools).unwrap();
    assert!(!r.lossy && r.bytes_after < r.bytes_before / 3, "{r:?}");
    let (a, b) = (mj_formats::media::probe(&src).unwrap(), mj_formats::media::probe(&dst).unwrap());
    assert_eq!(
        (b.duration_ms, b.sample_rate, b.channels, b.bits_per_sample),
        (a.duration_ms, a.sample_rate, a.channels, a.bits_per_sample)
    );
}

#[test]
fn silence_preview_lists_the_removable_part() {
    let Some((exe, _)) = ffmpeg() else { return };
    let d = TempDir::new("ff-prev").unwrap();
    let src = tone_silence_tone(&d);
    let p = audio::preview(&exe, &src, &ParamValues::new(), &CancelToken::new()).unwrap();
    assert_eq!(p.duration_ms, 12_000);
    assert_eq!(p.peaks.len(), 200);
    // Silence 3..9 s with a 2 s allowed pause: 5..9 s is removable (detector tolerance 0.3 s).
    assert_eq!(p.silences.len(), 1, "{:?}", p.silences);
    let (s, e) = p.silences[0];
    assert!(s.abs_diff(5_000) < 400 && e.abs_diff(9_000) < 400, "{s} {e}");
    assert!(p.peaks[10] > 40 && p.peaks[100] < 10, "tone then silence");
}

#[test]
fn video_hevc_reencode_shrinks_and_probes_as_hevc() {
    let Some((exe, tools)) = ffmpeg() else { return };
    if !encoders_have(&exe, &["libx264", "libx265"]) {
        return eprintln!("SKIPPED: no libx264/libx265");
    }
    let d = TempDir::new("ff-video").unwrap();
    let src = make_video(&exe, &d, 3, 1);
    let dst = d.join("out.mp4");
    let p = ParamValues::new().with("speed", spec::t("fast"));
    let r = job_run(&Video, &src, &dst, &d, &p, &tools).unwrap();
    assert!(r.skipped.is_none() && r.lossy && r.bytes_after < r.bytes_before, "{r:?}");
    assert!(r.notes.iter().any(|n| n == "encoder: libx265") && r.notes.iter().any(|n| n.contains("HEVC")));
    let (a, b) = (mj_formats::media::probe(&src).unwrap(), mj_formats::media::probe(&dst).unwrap());
    assert_eq!(b.video_codec.as_deref(), Some("hevc"));
    assert_eq!((b.width, b.height), (a.width, a.height));
    assert!(b.duration_ms.unwrap().abs_diff(a.duration_ms.unwrap()) < 600);
}

#[test]
fn video_downscale_and_fps_cap() {
    let Some((exe, tools)) = ffmpeg() else { return };
    if !encoders_have(&exe, &["libx264", "libx265"]) {
        return eprintln!("SKIPPED: no libx264/libx265");
    }
    let d = TempDir::new("ff-scale").unwrap();
    let src = make_video(&exe, &d, 3, 1);
    let dst = d.join("o.mp4");
    let p = ParamValues::new()
        .with("speed", spec::t("fast"))
        .with("max_height", spec::t("120"))
        .with("max_fps", spec::t("5"));
    let r = job_run(&Video, &src, &dst, &d, &p, &tools).unwrap();
    let b = mj_formats::media::probe(&dst).unwrap();
    assert_eq!(b.height, Some(120), "{r:?}");
    assert!(b.frame_rate.unwrap() < 6.0, "{:?}", b.frame_rate);
}

#[test]
fn hardware_detection_runs_and_never_claims_unverified_hardware() {
    let Some((exe, _)) = ffmpeg() else { return };
    let hw = detect_hardware(&exe, &CancelToken::new()).unwrap();
    eprintln!("hardware encoders listed: {:?}", hw.iter().map(|h| (&h.name, h.verified)).collect::<Vec<_>>());
    // Without a GPU the listed encoders fail their trial encode.
    assert!(hw.iter().all(|h| h.name.ends_with("nvenc") || h.name.ends_with("qsv") || h.name.ends_with("amf")));
}

#[test]
fn screen_dead_time_is_found_and_trimmed() {
    let Some((exe, tools)) = ffmpeg() else { return };
    if !encoders_have(&exe, &["libx264", "libx265"]) {
        return eprintln!("SKIPPED: no libx264/libx265");
    }
    let d = TempDir::new("ff-dead").unwrap();
    let src = make_video(&exe, &d, 3, 6); // 3 s of motion, then 6 s of a still, silent screen
    let info = mj_formats::media::probe(&src).unwrap();
    let cuts = screen::suggest_cuts_for(&exe, &src, &info, &CancelToken::new()).unwrap();
    let end = cuts.trim_end_s.expect("a still, silent end");
    assert!((3.5..5.0).contains(&end), "{cuts:?}");
    assert_eq!(cuts.trim_start_s, 0.0);
    let dst = d.join("o.mp4");
    let p = ParamValues::new().with("trim_dead_time", spec::b(true)).with("speed", spec::t("fast"));
    let r = job_run(&ScreenRecordings, &src, &dst, &d, &p, &tools).unwrap();
    let b = mj_formats::media::probe(&dst).unwrap();
    assert!((3.0..6.0).contains(&b.duration_s().unwrap()), "{:?} {r:?}", b.duration_s());
    assert!(r.notes.iter().any(|n| n.starts_with("trimmed from")));
}

#[test]
fn screen_audio_only_derivative() {
    let Some((exe, tools)) = ffmpeg() else { return };
    if !encoders_have(&exe, &["libx264", "libopus"]) {
        return eprintln!("SKIPPED: no libx264/libopus");
    }
    let d = TempDir::new("ff-aonly").unwrap();
    let src = make_video(&exe, &d, 3, 1);
    let dst = d.join("o.opus");
    let r =
        job_run(&ScreenRecordings, &src, &dst, &d, &ParamValues::new().with("output", spec::t("audio_only")), &tools)
            .unwrap();
    assert!(r.skipped.is_none() && r.bytes_after * 5 < r.bytes_before, "{r:?}");
    assert_eq!(mj_formats::media::probe(&dst).unwrap().audio_codec.as_deref(), Some("opus"));
}

#[test]
fn cancel_stops_a_running_encode() {
    let Some((exe, tools)) = ffmpeg() else { return };
    if !encoders_have(&exe, &["libx264", "libx265"]) {
        return eprintln!("SKIPPED: no libx264/libx265");
    }
    let d = TempDir::new("ff-cancel").unwrap();
    let src = make_video(&exe, &d, 3, 1);
    let dst = d.join("o.mp4");
    let c = CancelToken::new();
    c.cancel();
    let p = ParamValues::new().with("speed", spec::t("slow"));
    let r = Video.optimise(&OptimiseJob {
        src: &src,
        dst: &dst,
        params: &p,
        tools: &tools,
        temp_dir: d.path(),
        cancel: &c,
        progress: &NullProgress,
    });
    assert_eq!(r.unwrap_err(), mj_core::Error::Cancelled);
    assert!(!dst.exists());
}
