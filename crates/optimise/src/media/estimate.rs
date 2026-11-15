//! Size and time estimates that work without any tool: bitrate arithmetic over the in-house
//! metadata (target bitrate x duration). Every number here is an *estimate* and callers mark it
//! `Confidence::Estimated`.

use std::path::Path;

use mj_formats::media::MediaInfo;

/// Probe a file's metadata; `None` when the format is not one the in-house parsers read.
pub fn info_of(path: &Path) -> Option<MediaInfo> {
    mj_formats::media::probe(path).ok()
}

/// What a video run will do, as far as size is concerned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoPlan {
    pub av1: bool,
    pub crf: f64,
    /// Output height cap in pixels (0 = keep).
    pub max_height: u32,
    /// Frame-rate cap (0 = keep).
    pub max_fps: f64,
    /// Screen content (flat colours, little motion): much cheaper per pixel.
    pub screen: bool,
    pub remux: bool,
    pub audio_only: bool,
}

/// Bits per pixel per frame for the codec at `crf`; six CRF steps roughly double or halve the rate.
pub fn video_bpp(av1: bool, crf: f64, screen: bool) -> f64 {
    let (base, ref_crf) = match (av1, screen) {
        (false, false) => (0.070, 28.0),
        (true, false) => (0.055, 32.0),
        (false, true) => (0.030, 30.0),
        (true, true) => (0.022, 34.0),
    };
    base * 2f64.powf((ref_crf - crf) / 6.0)
}

/// Estimated size of the output of a video run.
pub fn video_after(info: Option<&MediaInfo>, size: u64, p: &VideoPlan) -> u64 {
    let before = size as f64;
    if p.remux {
        return (before * 0.995) as u64;
    }
    let dur = info.and_then(MediaInfo::duration_s).filter(|d| *d > 0.0);
    if p.audio_only {
        return dur.map_or(before * 0.03, |d| d * 32_000.0 / 8.0) as u64;
    }
    let after = match (info, dur) {
        (Some(i), Some(d)) => match (i.width, i.height, i.frame_rate) {
            (Some(w), Some(h), fps) => {
                let (mut w, mut h) = (f64::from(w), f64::from(h));
                if p.max_height > 0 && h > f64::from(p.max_height) {
                    w *= f64::from(p.max_height) / h;
                    h = f64::from(p.max_height);
                }
                let fps_in = f64::from(fps.unwrap_or(30.0));
                let fps_out = if p.max_fps > 0.0 && fps_in > p.max_fps { p.max_fps } else { fps_in };
                let video_bps = video_bpp(p.av1, p.crf, p.screen) * w * h * fps_out;
                let audio_bps = if i.audio_tracks > 0 { 96_000.0 } else { 0.0 };
                (video_bps + audio_bps) * d / 8.0 * 1.01
            }
            _ => before * fallback_ratio(p),
        },
        _ => before * fallback_ratio(p),
    };
    after.min(before) as u64
}

fn fallback_ratio(p: &VideoPlan) -> f64 {
    match (p.screen, p.av1) {
        (true, _) => 0.2,
        (false, true) => 0.40,
        (false, false) => 0.50,
    }
}

/// Rough seconds to encode: realtime speed of the encoder at 1080p30, scaled by pixel rate.
pub fn encode_seconds(encoder: &str, speed_mult: f64, dur_s: f64, w: u32, h: u32, fps: f64) -> f64 {
    let factor = match encoder {
        "libx265" => 0.9,
        "libsvtav1" => 1.2,
        "libaom-av1" => 0.12,
        "librav1e" => 0.15,
        e if e.ends_with("_nvenc") => 8.0,
        e if e.ends_with("_qsv") => 6.0,
        e if e.ends_with("_amf") => 5.0,
        _ => 0.5,
    };
    let pixel_rate = (f64::from(w.max(1)) * f64::from(h.max(1)) * fps.max(1.0)).max(1.0);
    dur_s * (pixel_rate / (1920.0 * 1080.0 * 30.0)) / (factor * speed_mult.max(0.05))
}

/// What an audio run will do, as far as size is concerned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioPlan {
    /// `Some(kbps)` for Opus/AAC; `None` for FLAC.
    pub lossy_kbps: Option<u32>,
    pub mono: bool,
    pub strip_cover: bool,
}

/// Estimated size of the output of an audio run.
pub fn audio_after(info: Option<&MediaInfo>, size: u64, p: &AudioPlan) -> u64 {
    let before = size as f64;
    let cover = if p.strip_cover { info.and_then(|i| i.cover_art_bytes).unwrap_or(0) as f64 } else { 0.0 };
    let dur = info.and_then(MediaInfo::duration_s).filter(|d| *d > 0.0);
    let after = match (p.lossy_kbps, dur) {
        (Some(k), Some(d)) => d * f64::from(k) * 125.0 * 1.02,
        (Some(k), None) => before * f64::from(k) / 384.0,
        (None, _) => {
            let codec = info.and_then(|i| i.audio_codec.as_deref()).unwrap_or("");
            let stereo = info.and_then(|i| i.channels).unwrap_or(2) > 1;
            let mut r = match codec {
                "flac" => 0.97,
                "pcm" | "pcm_float" => 0.55,
                _ => 3.0, // a lossy source grows when stored as FLAC
            };
            if p.mono && stereo {
                r *= if codec == "flac" {
                    0.55
                } else if codec.starts_with("pcm") {
                    0.5
                } else {
                    1.0
                };
            }
            (before - cover).max(0.0) * r
        }
    };
    after.clamp(0.0, before * 4.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_formats::media::build;

    fn info(buf: Vec<u8>) -> MediaInfo {
        let n = buf.len() as u64;
        mj_formats::media::probe_reader(std::io::Cursor::new(buf), n).unwrap()
    }

    fn video_info() -> MediaInfo {
        use build::mp4::*;
        let tail = moov(
            600_000,
            &[
                trak(b"vide", b"avc1", 1920, 1080, 30_000, 18_000_000, 18_000),
                trak(b"soun", b"mp4a", 0, 0, 48_000, 28_800_000, 28_000),
            ],
        );
        info(file(b"isom", 600_000_000, &[tail]))
    }

    #[test]
    fn video_estimate_follows_codec_crf_resolution_and_fps() {
        let i = video_info();
        let size = i.file_size;
        let mk = |av1, crf, h, fps| VideoPlan {
            av1,
            crf,
            max_height: h,
            max_fps: fps,
            screen: false,
            remux: false,
            audio_only: false,
        };
        let hevc = video_after(Some(&i), size, &mk(false, 28.0, 0, 0.0));
        let av1 = video_after(Some(&i), size, &mk(true, 32.0, 0, 0.0));
        assert!(hevc < size * 6 / 10 && av1 < hevc, "{hevc} {av1} {size}");
        assert!(video_after(Some(&i), size, &mk(false, 34.0, 0, 0.0)) < hevc, "higher CRF is smaller");
        assert!(video_after(Some(&i), size, &mk(false, 28.0, 720, 0.0)) < hevc / 2, "720p quarter-ish");
        assert!(video_after(Some(&i), size, &mk(false, 28.0, 0, 15.0)) < hevc * 6 / 10, "half the frames");
        let screen = VideoPlan { screen: true, ..mk(false, 30.0, 0, 10.0) };
        assert!(video_after(Some(&i), size, &screen) < hevc / 3);
        // Never larger than the source; no metadata falls back to a ratio.
        assert_eq!(video_after(Some(&i), 1000, &mk(false, 10.0, 0, 0.0)), 1000);
        assert_eq!(video_after(None, 1000, &mk(false, 28.0, 0, 0.0)), 500);
        let remux = VideoPlan { remux: true, ..mk(false, 28.0, 0, 0.0) };
        assert!(video_after(Some(&i), size, &remux) > size / 2);
    }

    #[test]
    fn encode_time_ranks_encoders_and_scales_with_pixels() {
        let (sw, hw) = (
            encode_seconds("libx265", 1.0, 600.0, 1920, 1080, 30.0),
            encode_seconds("hevc_nvenc", 1.0, 600.0, 1920, 1080, 30.0),
        );
        assert!(hw * 5.0 < sw && (sw - 667.0).abs() < 5.0, "{sw} {hw}");
        assert!(encode_seconds("libx265", 1.0, 600.0, 3840, 2160, 30.0) > sw * 3.9);
        assert!(encode_seconds("libx265", 2.0, 600.0, 1920, 1080, 30.0) < sw);
        assert!(encode_seconds("libaom-av1", 1.0, 60.0, 1280, 720, 30.0).is_finite());
    }

    #[test]
    fn audio_estimates() {
        let wav = info(build::wav(16_000, 1, 600, None)); // 19.2 MB of PCM, 600 s
        let size = wav.file_size;
        let opus = audio_after(Some(&wav), size, &AudioPlan { lossy_kbps: Some(24), mono: true, strip_cover: true });
        assert!((1_800_000..1_900_000).contains(&opus), "{opus}");
        let flac = audio_after(Some(&wav), size, &AudioPlan { lossy_kbps: None, mono: false, strip_cover: true });
        assert!(flac < size * 6 / 10 && flac > size / 2);
        let flac_stereo_src = info(build::build_flac(44_100, 2, 16, 44_100 * 60, None, 5000));
        let again = audio_after(
            Some(&flac_stereo_src),
            flac_stereo_src.file_size,
            &AudioPlan { lossy_kbps: None, mono: false, strip_cover: true },
        );
        assert!(again <= flac_stereo_src.file_size);
        assert!(
            audio_after(None, 1_000_000, &AudioPlan { lossy_kbps: Some(48), mono: false, strip_cover: false })
                < 200_000
        );
    }
}
