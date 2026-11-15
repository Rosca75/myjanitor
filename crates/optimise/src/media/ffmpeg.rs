//! Running ffmpeg and reading its text output: progress (`-progress pipe:1`), the encoder list,
//! `silencedetect` and `freezedetect` results. Parsers are pure functions over text so they are
//! tested without the tool.

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use mj_core::{CancelToken, Error, OptimiseJob, Phase, Progress, Result};

use crate::external::{args, path_arg, run, LineHook, RunOptions, RunOutput, Stream};

/// Arguments every ffmpeg run starts with (no stdin, quiet, machine-readable progress on stdout).
pub fn base_args() -> Vec<OsString> {
    args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-progress", "pipe:1", "-nostats"])
}

/// F-17: `-protocol_whitelist file [-f <demuxer>] -i <src>`. ffmpeg may open nothing but
/// local files (no network protocol, no `concat:`/`subfile:` tricks), and the demuxer is
/// forced from MyJanitor's own content sniffing whenever it is known, so a "video" that is
/// really an HLS or concat playlist is never interpreted as one.
pub fn input_args(src: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = args(["-protocol_whitelist", "file"]);
    if let Some(d) = demuxer_of(src) {
        a.extend(args(["-f", d]));
    }
    a.push("-i".into());
    a.push(path_arg(src));
    a
}

/// The ffmpeg demuxer for `src`, from its first bytes; `None` when MyJanitor does not
/// recognise the container (text playlists, scripts, unknown formats).
pub fn demuxer_of(src: &Path) -> Option<&'static str> {
    use std::io::Read;
    let mut head = [0u8; 16];
    let mut f = std::fs::File::open(src).ok()?;
    let mut n = 0;
    while n < head.len() {
        match f.read(&mut head[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    demuxer_for_head(&head[..n])
}

/// [`demuxer_of`] over bytes (pure).
pub fn demuxer_for_head(h: &[u8]) -> Option<&'static str> {
    if h.len() < 12 {
        return None;
    }
    Some(match &h[..4] {
        b"RIFF" | b"RF64" if &h[8..12] == b"WAVE" => "wav",
        b"RIFF" if &h[8..12] == b"AVI " => "avi",
        b"FORM" if matches!(&h[8..12], b"AIFF" | b"AIFC") => "aiff",
        b"fLaC" => "flac",
        b"OggS" => "ogg",
        [0x1A, 0x45, 0xDF, 0xA3] => "matroska",
        b"ID3\x02" | b"ID3\x03" | b"ID3\x04" => "mp3",
        _ if &h[4..8] == b"ftyp" => "mov",
        _ if h[0] == 0xFF && h[1] & 0xF6 == 0xF0 => "aac",
        _ if h[0] == 0xFF && h[1] & 0xE0 == 0xE0 => "mp3",
        _ => return None,
    })
}

/// F-17: refuse (skip) an input whose container MyJanitor does not recognise, before
/// ffmpeg ever reads it.
pub fn require_container(src: &Path) -> Result<()> {
    if demuxer_of(src).is_none() {
        return Err(Error::unsupported(
            "media",
            "not a recognised audio or video file (its content does not match a known container)",
        ));
    }
    Ok(())
}

/// Run an encode: progress is reported from `out_time_us=` lines against `duration_ms`.
pub fn run_encode(job: &OptimiseJob<'_>, exe: &Path, argv: &[OsString], duration_ms: Option<u64>) -> Result<RunOutput> {
    let hook = |s: Stream, line: &str| {
        if s == Stream::Stdout {
            if let Some(ms) = parse_progress_ms(line) {
                let mut p = Progress::new(Phase::Applying, ms, duration_ms);
                p.current = Some(job.src.to_path_buf());
                job.progress.report(&p);
            }
        }
    };
    let hook: LineHook<'_> = &hook;
    let opts = RunOptions {
        timeout: Some(Duration::from_secs(12 * 3600)),
        cancel: Some(job.cancel),
        cwd: Some(job.temp_dir),
        on_line: Some(hook),
        ..Default::default()
    };
    let out = run(exe, argv, &opts)?;
    out.ensure_success("ffmpeg")?;
    Ok(out)
}

/// Run an analysis pass that prints to stderr (`silencedetect`, `freezedetect`): returns stderr.
pub fn run_analysis(exe: &Path, argv: &[OsString], cancel: &CancelToken, cwd: Option<&Path>) -> Result<String> {
    let opts = RunOptions {
        timeout: Some(Duration::from_secs(3600)),
        max_stderr: 4 << 20,
        cancel: Some(cancel),
        cwd,
        ..Default::default()
    };
    let out = run(exe, argv, &opts)?;
    out.ensure_success("ffmpeg")?;
    Ok(out.stderr)
}

/// `out_time_us=12345678` (microseconds, despite the `out_time_ms` twin) to milliseconds.
pub fn parse_progress_ms(line: &str) -> Option<u64> {
    let v = line.strip_prefix("out_time_us=")?.trim().parse::<i64>().ok()?;
    (v >= 0).then_some(v as u64 / 1000)
}

/// Names of the encoders in `ffmpeg -encoders` output (video and audio).
pub fn parse_encoders(text: &str) -> Vec<String> {
    let mut seen_rule = false;
    let mut v = Vec::new();
    for l in text.lines() {
        if l.trim_start().starts_with("------") {
            seen_rule = true;
            continue;
        }
        if !seen_rule {
            continue;
        }
        let mut it = l.split_whitespace();
        if let (Some(flags), Some(name)) = (it.next(), it.next()) {
            if flags.len() == 6 && matches!(flags.as_bytes()[0], b'V' | b'A' | b'S') {
                v.push(name.to_string());
            }
        }
    }
    v
}

/// Query the encoder list.
pub fn list_encoders(exe: &Path, cancel: &CancelToken) -> Result<Vec<String>> {
    let opts = RunOptions {
        timeout: Some(Duration::from_secs(20)),
        max_stdout: 1 << 20,
        cancel: Some(cancel),
        ..Default::default()
    };
    let out = run(exe, &args(["-hide_banner", "-encoders"]), &opts)?;
    out.ensure_success("ffmpeg")?;
    Ok(parse_encoders(&String::from_utf8_lossy(&out.stdout)))
}

/// Parse `key: seconds` markers (`silence_start`, `silence_end`, `lavfi.freezedetect.freeze_start`...) into
/// `(start, end)` intervals in seconds. An unterminated last interval ends at `total_s` when known.
fn intervals(text: &str, start_key: &str, end_key: &str, total_s: Option<f64>) -> Vec<(f64, f64)> {
    let num = |l: &str, key: &str| -> Option<f64> {
        let at = l.find(key)? + key.len();
        l[at..].trim_start().split(|c: char| c.is_whitespace() || c == '|').next()?.parse::<f64>().ok()
    };
    let (mut out, mut open) = (Vec::new(), None);
    for l in text.lines() {
        if let Some(s) = num(l, start_key) {
            if let Some(prev) = open.replace(s.max(0.0)) {
                out.push((prev, s.max(prev)));
            }
        } else if let Some(e) = num(l, end_key) {
            if let Some(s) = open.take() {
                out.push((s, e.max(s)));
            }
        }
    }
    if let (Some(s), Some(t)) = (open, total_s) {
        if t > s {
            out.push((s, t));
        }
    }
    out
}

/// `silencedetect` intervals in seconds.
pub fn parse_silences(stderr: &str, total_s: Option<f64>) -> Vec<(f64, f64)> {
    intervals(stderr, "silence_start:", "silence_end:", total_s)
}

/// `freezedetect` intervals in seconds.
pub fn parse_freezes(stderr: &str, total_s: Option<f64>) -> Vec<(f64, f64)> {
    intervals(stderr, "freeze_start:", "freeze_end:", total_s)
}

/// Intersection of two sorted-or-not interval lists.
pub fn intersect(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for &(s1, e1) in a {
        for &(s2, e2) in b {
            let (s, e) = (s1.max(s2), e1.min(e2));
            if e > s {
                out.push((s, e));
            }
        }
    }
    out.sort_by(|x, y| x.0.total_cmp(&y.0));
    out
}

/// Trial-encode 0.2 s of black with `extra` encoder arguments; true when ffmpeg accepts them. Used to
/// find out whether an encoder supports an option (screen-content tools) or hardware is really present.
pub fn encoder_accepts(exe: &Path, encoder: &str, extra: &[&str], cancel: &CancelToken) -> bool {
    let mut a = args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=256x256:r=10:d=0.4",
        "-c:v",
        encoder,
    ]);
    a.extend(args(extra));
    a.extend(args(["-f", "null", "-"]));
    run(exe, &a, &RunOptions { timeout: Some(Duration::from_secs(30)), cancel: Some(cancel), ..Default::default() })
        .is_ok_and(|o| o.success())
}

/// Seconds as `h:mm:ss` or `m:ss`.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The kind of one stream, as `ffmpeg -i` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StreamKind {
    Video,
    /// A still picture attached to the file (cover art), shown as `(attached pic)`.
    CoverPicture,
    Audio,
    Subtitle,
    /// Timed data: timecode (`tmcd`), GPS or telemetry (`gpmd`), camera metadata (`mebx`)...
    Data,
    /// A file carried inside the container (fonts of a Matroska file).
    Attachment,
    Other,
}

impl StreamKind {
    /// Plural user-facing name.
    pub fn label(self) -> &'static str {
        match self {
            StreamKind::Video => "video tracks",
            StreamKind::CoverPicture => "cover pictures",
            StreamKind::Audio => "audio tracks",
            StreamKind::Subtitle => "subtitle tracks",
            StreamKind::Data => "data tracks (timecode, GPS or camera data)",
            StreamKind::Attachment => "attached files (such as subtitle fonts)",
            StreamKind::Other => "other tracks",
        }
    }
}

/// One input stream: its kind and the codec text after the kind (`tmcd`, `gpmd`, `ttf`...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputStream {
    pub kind: StreamKind,
    pub codec: String,
}

/// What `ffmpeg -i <file>` reports about a file: its length and its streams. ffmpeg reads every
/// container it can decode, so this works where MyJanitor's own media prober does not (Matroska,
/// WebM, AVI, ASF/WMV) and is the independent check of an encode's output (OPT-4, OPT-5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputReport {
    pub duration_ms: Option<u64>,
    pub streams: Vec<InputStream>,
}

impl InputReport {
    /// Number of streams of `kind`.
    pub fn count(&self, kind: StreamKind) -> usize {
        self.streams.iter().filter(|s| s.kind == kind).count()
    }
}

/// Parse the `Input #0` part of `ffmpeg -i` stderr; `None` when ffmpeg did not open the file.
pub fn parse_input_report(stderr: &str) -> Option<InputReport> {
    let mut lines = stderr.lines().skip_while(|l| !l.starts_with("Input #0"));
    lines.next()?;
    let mut r = InputReport::default();
    for line in lines {
        let t = line.trim_start();
        if line.starts_with("Input #") || line.starts_with("Output #") || t.starts_with("Stream mapping") {
            break;
        }
        if let Some(d) = t.strip_prefix("Duration: ") {
            r.duration_ms = parse_clock_ms(d.split(',').next().unwrap_or(""));
        } else if let Some(rest) = t.strip_prefix("Stream #0:") {
            let Some((_, desc)) = rest.split_once(": ") else { continue };
            let (kind, codec) = desc.split_once(": ").unwrap_or((desc, ""));
            let kind = match kind {
                "Video" if desc.contains("(attached pic)") => StreamKind::CoverPicture,
                "Video" => StreamKind::Video,
                "Audio" => StreamKind::Audio,
                "Subtitle" => StreamKind::Subtitle,
                "Data" => StreamKind::Data,
                "Attachment" => StreamKind::Attachment,
                _ => StreamKind::Other,
            };
            let codec = match codec.find(" (").filter(|_| kind == StreamKind::Data) {
                // `Data: none (tmcd / 0x64636D74)`: the tag names the data.
                Some(i) => codec[i + 2..].split([' ', ')']).next().unwrap_or("").to_string(),
                None => codec.split([',', ' ']).next().unwrap_or("").to_string(),
            };
            r.streams.push(InputStream { kind, codec });
        }
    }
    Some(r)
}

/// `HH:MM:SS.ss` to milliseconds (`N/A` and garbage give `None`).
fn parse_clock_ms(s: &str) -> Option<u64> {
    let mut parts = s.trim().split(':');
    let (h, m, sec) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let (h, m, sec) = (h.parse::<u64>().ok()?, m.parse::<u64>().ok()?, sec.parse::<f64>().ok()?);
    (sec.is_finite() && sec >= 0.0).then(|| (h * 3600 + m * 60) * 1000 + (sec * 1000.0).round() as u64)
}

/// Run `ffmpeg -i <path>` (local files only, demuxer forced as for an encode) and parse its report.
pub fn probe_input(exe: &Path, path: &Path, cancel: &CancelToken) -> Result<InputReport> {
    let mut a = args(["-nostdin", "-hide_banner"]);
    a.extend(input_args(path));
    let opts = RunOptions {
        timeout: Some(Duration::from_secs(120)),
        max_stderr: 4 << 20,
        cancel: Some(cancel),
        ..Default::default()
    };
    // Without an output ffmpeg exits with an error after printing the report: that is expected.
    let out = run(exe, &a, &opts)?;
    parse_input_report(&out.stderr).ok_or_else(|| {
        let why = out.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no report").trim().to_string();
        Error::parse("media", format!("ffmpeg could not read {}: {why}", path.display()))
    })
}

/// The length check of a run that cuts nothing (OPT-4): both lengths must be known and agree
/// within `tolerance_ms`. It fails closed: an unknown length is an error, never a pass.
pub fn check_length(d_in: Option<u64>, d_out: Option<u64>, tolerance_ms: u64, what: &str) -> Result<()> {
    match (d_in, d_out) {
        (Some(a), Some(b)) if a.abs_diff(b) <= tolerance_ms => Ok(()),
        (Some(a), Some(b)) => Err(Error::Internal(format!(
            "the new {what} is {} long but the original is {}; nothing was changed",
            clock(b as f64 / 1000.0),
            clock(a as f64 / 1000.0)
        ))),
        (None, _) => Err(Error::Internal(format!(
            "the length of the original {what} could not be read, so the result cannot be checked; nothing was changed"
        ))),
        (Some(_), None) => Err(Error::Internal(format!(
            "the length of the new {what} could not be read, so it cannot be checked; nothing was changed"
        ))),
    }
}

/// An error for a finished encode that left no usable file.
pub fn empty_output(tool: &str) -> Error {
    Error::Internal(format!("{tool} finished but wrote no output; try again or check the file plays"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENCODERS: &str = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D libx265              libx265 H.265 / HEVC (codec hevc)\n V....D hevc_nvenc           NVIDIA NVENC hevc encoder (codec hevc)\n A....D libopus              libopus Opus\n S..... mov_text             3GPP\n";

    #[test]
    fn encoder_list_skips_the_legend() {
        assert_eq!(parse_encoders(ENCODERS), ["libx265", "hevc_nvenc", "libopus", "mov_text"]);
        assert!(parse_encoders("garbage").is_empty());
    }

    #[test]
    fn progress_lines() {
        assert_eq!(parse_progress_ms("out_time_us=2500000"), Some(2500));
        assert_eq!(parse_progress_ms("out_time_us=-9223372036854775807"), None);
        assert_eq!(parse_progress_ms("progress=continue"), None);
    }

    #[test]
    fn silence_and_freeze_intervals() {
        let err = "[silencedetect @ 0x55] silence_start: 1.5\n[silencedetect @ 0x55] silence_end: 4.25 | silence_duration: 2.75\nnoise\n[silencedetect @ 0x55] silence_start: 10\n";
        assert_eq!(parse_silences(err, Some(12.0)), [(1.5, 4.25), (10.0, 12.0)]);
        assert_eq!(parse_silences(err, None), [(1.5, 4.25)]);
        let fr = "[freezedetect @ 0x1] lavfi.freezedetect.freeze_start: 0\n[freezedetect @ 0x1] lavfi.freezedetect.freeze_duration: 3\n[freezedetect @ 0x1] lavfi.freezedetect.freeze_end: 3\n";
        assert_eq!(parse_freezes(fr, None), [(0.0, 3.0)]);
        assert_eq!(intersect(&[(0.0, 3.0), (8.0, 9.0)], &[(1.0, 5.0)]), [(1.0, 3.0)]);
    }

    const REPORT: &str = "[mov @ 0x5] Using non-standard frame rate\nInput #0, mov,mp4,m4a,3gp,3g2,mj2, from 'GX010001.MP4':\n  Metadata:\n    major_brand     : mp41\n  Duration: 00:45:00.50, start: 0.000000, bitrate: 112 kb/s\n  Stream #0:0[0x1](eng): Video: h264 (High) (avc1 / 0x31637661), yuv420p, 64x48, 30 kb/s, 10 fps (default)\n    Metadata:\n      handler_name    : GoPro AVC\n  Stream #0:1[0x2](eng): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo\n  Stream #0:2[0x3](eng): Data: none (tmcd / 0x64636D74), 0 kb/s\n  Stream #0:3[0x4](eng): Data: bin_data (gpmd / 0x646D7067), 61 kb/s\n  Stream #0:4: Video: mjpeg (Baseline), yuvj420p, 300x300, 90k tbn (attached pic)\n  Stream #0:5: Attachment: ttf\n  Stream #0:6(eng): Subtitle: ass (default)\nAt least one output file must be specified\n";

    #[test]
    fn input_report_lists_length_and_every_stream() {
        let r = parse_input_report(REPORT).unwrap();
        assert_eq!(r.duration_ms, Some(2_700_500));
        let kinds: Vec<(StreamKind, &str)> = r.streams.iter().map(|s| (s.kind, s.codec.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (StreamKind::Video, "h264"),
                (StreamKind::Audio, "aac"),
                (StreamKind::Data, "tmcd"),
                (StreamKind::Data, "gpmd"),
                (StreamKind::CoverPicture, "mjpeg"),
                (StreamKind::Attachment, "ttf"),
                (StreamKind::Subtitle, "ass"),
            ]
        );
        assert_eq!(r.count(StreamKind::Data), 2);
        let na = parse_input_report("Input #0, matroska,webm, from 'a.mkv':\n  Duration: N/A, start: 0\n").unwrap();
        assert_eq!(na, InputReport::default());
        assert!(parse_input_report("a.mkv: Invalid data found when processing input\n").is_none());
        assert_eq!(parse_clock_ms("01:02:03.25"), Some(3_723_250));
        assert_eq!(parse_clock_ms("1:2"), None);
    }

    #[test]
    fn length_check_fails_closed() {
        assert!(check_length(Some(60_000), Some(59_500), 2000, "video").is_ok());
        assert!(check_length(Some(2_700_000), Some(130_000), 2000, "video").unwrap_err().to_string().contains("2:10"));
        assert!(check_length(None, Some(1000), 2000, "video").is_err());
        assert!(check_length(Some(1000), None, 2000, "video").is_err());
        assert!(check_length(None, None, 2000, "video").is_err());
    }

    #[test]
    fn clock_format() {
        assert_eq!(
            (clock(59.6), clock(125.0), clock(3725.0)),
            ("1:00".to_string(), "2:05".to_string(), "1:02:05".to_string())
        );
    }
}
