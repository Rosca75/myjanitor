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

/// `-i <src>`.
pub fn input_args(src: &Path) -> Vec<OsString> {
    vec!["-i".into(), path_arg(src)]
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
    let opts = RunOptions { timeout: Some(Duration::from_secs(12 * 3600)), cancel: Some(job.cancel), cwd: Some(job.temp_dir), on_line: Some(hook), ..Default::default() };
    let out = run(exe, argv, &opts)?;
    out.ensure_success("ffmpeg")?;
    Ok(out)
}

/// Run an analysis pass that prints to stderr (`silencedetect`, `freezedetect`): returns stderr.
pub fn run_analysis(exe: &Path, argv: &[OsString], cancel: &CancelToken, cwd: Option<&Path>) -> Result<String> {
    let opts = RunOptions { timeout: Some(Duration::from_secs(3600)), max_stderr: 4 << 20, cancel: Some(cancel), cwd, ..Default::default() };
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
    let opts = RunOptions { timeout: Some(Duration::from_secs(20)), max_stdout: 1 << 20, cancel: Some(cancel), ..Default::default() };
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
    let mut a = args(["-nostdin", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=256x256:r=10:d=0.4", "-c:v", encoder]);
    a.extend(args(extra));
    a.extend(args(["-f", "null", "-"]));
    run(exe, &a, &RunOptions { timeout: Some(Duration::from_secs(30)), cancel: Some(cancel), ..Default::default() }).is_ok_and(|o| o.success())
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

    #[test]
    fn clock_format() {
        assert_eq!((clock(59.6), clock(125.0), clock(3725.0)), ("1:00".to_string(), "2:05".to_string(), "1:02:05".to_string()));
    }
}
