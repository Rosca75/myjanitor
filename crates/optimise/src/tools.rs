//! External tool detection (W3-C): PATH (plus PATHEXT on Windows) and
//! settings-supplied directories, searched without starting a shell, with a
//! cached, time-limited version probe and typed availability reasons.
//!
//! A tool is "found" only when a regular, executable file with the right name
//! exists. On Windows only `.exe` and `.com` count even when `PATHEXT` lists
//! `.bat` or `.cmd`: those would be run through `cmd.exe`, which re-parses the
//! argument line, and the process runner promises argument vectors only.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use mj_core::tools::KNOWN_TOOLS;
use mj_core::{Availability, Error, ExternalTool, Result};

use crate::external::{args, run, RunOptions};

/// How long a version probe may take before the tool is reported without a version.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Features that use each tool (Settings page text).
fn used_for(id: &str) -> Vec<String> {
    let v: &[&str] = match id {
        "ffmpeg" | "ffprobe" => &["Video", "Screen recordings", "Meeting recordings"],
        "avifenc" | "cjxl" | "cwebp" => &["Photos (lossy)"],
        "7z" => &["7z archives"],
        "zstd" => &["zstd archives"],
        "oxipng" => &["PNG images"],
        "jpegtran" => &["JPEG images"],
        _ => &[],
    };
    v.iter().map(|s| s.to_string()).collect()
}

/// File names a tool may have (Linux packages ship 7-Zip as `7zz`, `7za` or `7z`).
fn base_names(id: &str) -> Vec<&str> {
    match id {
        "7z" => vec!["7z", "7zz", "7za"],
        other => vec![other],
    }
}

fn windows_extensions() -> Vec<String> {
    let from_env = std::env::var("PATHEXT").unwrap_or_default();
    let mut v: Vec<String> = from_env
        .split(';')
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| e == ".exe" || e == ".com")
        .collect();
    if v.is_empty() {
        v = vec![".exe".into(), ".com".into()];
    }
    v
}

/// True for an existing regular file that the OS would execute.
fn is_runnable(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Candidate file names for `name` in one directory.
fn file_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".com") {
            return vec![name.to_string()];
        }
        windows_extensions().iter().map(|e| format!("{name}{e}")).collect()
    } else {
        vec![name.to_string()]
    }
}

/// First runnable `name` inside `dirs`, in order. Relative and empty directory entries are skipped
/// so a tool is never picked up from the current directory.
pub fn which_in<I>(name: &str, dirs: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    let names = file_names(name);
    dirs.into_iter().filter(|d| d.is_absolute()).find_map(|d| names.iter().map(|n| d.join(n)).find(|p| is_runnable(p)))
}

/// The directories of a `PATH`-style value.
pub fn split_path(path: &OsStr) -> Vec<PathBuf> {
    std::env::split_paths(path).collect()
}

/// Find `name` on the process `PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    which_in(name, std::env::var_os("PATH").map(|p| split_path(&p)).unwrap_or_default())
}

/// Inputs of one detection run.
#[derive(Debug, Clone, Default)]
pub struct DetectOptions {
    /// Per-tool manual paths from settings (a file, or a directory to look in). They win over everything.
    pub overrides: BTreeMap<String, PathBuf>,
    /// Extra directories searched before `PATH` (settings).
    pub extra_dirs: Vec<PathBuf>,
    /// Use this instead of the process `PATH` (tests, sandboxes).
    pub path: Option<OsString>,
    /// Skip running the tools to read their versions.
    pub skip_version_probe: bool,
}

fn locate(id: &str, o: &DetectOptions) -> Option<PathBuf> {
    let names = base_names(id);
    if let Some(p) = o.overrides.get(id) {
        if is_runnable(p) {
            return Some(p.clone());
        }
        if p.is_dir() {
            if let Some(f) = names.iter().find_map(|n| which_in(n, [p.clone()])) {
                return Some(f);
            }
        }
    }
    let path_dirs = match &o.path {
        Some(p) => split_path(p),
        None => std::env::var_os("PATH").map(|p| split_path(&p)).unwrap_or_default(),
    };
    let dirs: Vec<PathBuf> = o.extra_dirs.iter().cloned().chain(path_dirs).collect();
    names.iter().find_map(|n| which_in(n, dirs.clone()))
}

type ProbeKey = (PathBuf, u64, i64);

fn probe_cache() -> &'static Mutex<HashMap<ProbeKey, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<ProbeKey, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Forget cached versions (the engine calls this from `tools_refresh`).
pub fn clear_cache() {
    mj_core::lock(probe_cache()).clear();
}

/// Arguments that make each tool print its version.
fn version_args(id: &str) -> Vec<OsString> {
    match id {
        "ffmpeg" | "ffprobe" | "cwebp" | "jpegtran" => args(["-version"]),
        "7z" => Vec::new(), // 7-Zip prints its banner when started without arguments
        _ => args(["--version"]),
    }
}

/// Pull a version number out of a banner line: the first token that starts with a digit and holds
/// a dot (`ffmpeg version n6.1.1-3 Copyright` gives `6.1.1-3`; `7-Zip (a) 23.01 (x64)` gives `23.01`).
pub fn parse_version(text: &str) -> Option<String> {
    text.lines().filter(|l| !l.trim().is_empty()).take(3).find_map(|line| {
        line.split_whitespace().find_map(|tok| {
            let t = tok.trim_matches(|c: char| matches!(c, ',' | ';' | '(' | ')' | '[' | ']' | ':'));
            let t = match t.as_bytes() {
                [b'n' | b'v' | b'V', d, ..] if d.is_ascii_digit() => &t[1..],
                _ => t,
            };
            (t.starts_with(|c: char| c.is_ascii_digit()) && t.contains('.')).then(|| t.to_string())
        })
    })
}

/// Run the tool with its version flag (time-limited, cached by path, size and modification time).
pub fn probe_version(id: &str, path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64);
    let key = (path.to_path_buf(), meta.len(), mtime);
    if let Some(v) = mj_core::lock(probe_cache()).get(&key) {
        return v.clone();
    }
    let opts = RunOptions { timeout: Some(PROBE_TIMEOUT), max_stdout: 8192, max_stderr: 8192, ..Default::default() };
    let version = run(path, &version_args(id), &opts).ok().and_then(|o| {
        // Some tools print the banner on stderr (jpegtran, 7z).
        let text = format!("{}\n{}", String::from_utf8_lossy(&o.stdout), o.stderr);
        parse_version(&text)
    });
    mj_core::lock(probe_cache()).insert(key, version.clone());
    version
}

/// Detect every known tool with full control over the search.
pub fn detect_with(o: &DetectOptions) -> Vec<ExternalTool> {
    let paths: Vec<Option<PathBuf>> = KNOWN_TOOLS.iter().map(|id| locate(id, o)).collect();
    let versions: Vec<Option<String>> = if o.skip_version_probe {
        vec![None; paths.len()]
    } else {
        std::thread::scope(|sc| {
            let hs: Vec<_> = KNOWN_TOOLS
                .iter()
                .zip(&paths)
                .map(|(id, p)| sc.spawn(move || p.as_deref().and_then(|p| probe_version(id, p))))
                .collect();
            hs.into_iter().map(|h| h.join().unwrap_or(None)).collect()
        })
    };
    KNOWN_TOOLS
        .iter()
        .zip(paths)
        .zip(versions)
        .map(|((id, path), version)| ExternalTool { id: id.to_string(), found: path.is_some(), path, version, used_for: used_for(id) })
        .collect()
}

/// Detect every known tool; `overrides` (from settings) win over PATH.
pub fn detect(overrides: &BTreeMap<String, PathBuf>) -> Vec<ExternalTool> {
    detect_with(&DetectOptions { overrides: overrides.clone(), ..Default::default() })
}

/// "Requires X on PATH" for the tools of `needed` that are missing, or `Available`.
pub fn availability(tools: &[ExternalTool], needed: &[&str]) -> Availability {
    let missing: Vec<&str> = needed.iter().copied().filter(|t| mj_core::tools::find(tools, t).is_none()).collect();
    match missing.as_slice() {
        [] => Availability::Available,
        [one] => Availability::Disabled { reason: format!("Requires {one} on PATH") },
        [init @ .., last] => Availability::Disabled { reason: format!("Requires {} and {last} on PATH", init.join(", ")) },
    }
}

/// Available when at least one of `any_of` is found; otherwise "Requires a, b or c on PATH".
pub fn availability_any(tools: &[ExternalTool], any_of: &[&str]) -> Availability {
    if any_of.iter().any(|t| mj_core::tools::find(tools, t).is_some()) {
        return Availability::Available;
    }
    match any_of {
        [] => Availability::Available,
        [one] => Availability::Disabled { reason: format!("Requires {one} on PATH") },
        [init @ .., last] => Availability::Disabled { reason: format!("Requires {} or {last} on PATH", init.join(", ")) },
    }
}

/// The executable path of a found tool, or `Error::ToolMissing`.
pub fn require<'a>(tools: &'a [ExternalTool], id: &str) -> Result<&'a Path> {
    mj_core::tools::find(tools, id).and_then(|t| t.path.as_deref()).ok_or_else(|| Error::ToolMissing(id.to_string()))
}

/// A found-tool record for tests and callers that already know the path.
pub fn tool_at(id: &str, path: impl Into<PathBuf>) -> ExternalTool {
    ExternalTool { id: id.into(), found: true, path: Some(path.into()), version: None, used_for: used_for(id) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    fn write_exe(dir: &Path, name: &str, script: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    fn fake_name(id: &str) -> String {
        if cfg!(windows) {
            format!("{id}.exe")
        } else {
            id.to_string()
        }
    }

    #[test]
    fn detects_every_known_tool() {
        let t = detect(&Default::default());
        assert_eq!(t.len(), KNOWN_TOOLS.len());
        assert!(t.iter().all(|x| x.found == x.path.is_some()));
    }

    #[test]
    fn finds_tools_in_a_fake_path_directory_and_reads_versions() {
        let dir = TempDir::new("fake-path").unwrap();
        write_exe(dir.path(), &fake_name("ffmpeg"), "#!/bin/sh\necho 'ffmpeg version n6.1.1-3ubuntu5 Copyright (c) the FFmpeg developers'\n");
        write_exe(dir.path(), &fake_name("cwebp"), "#!/bin/sh\necho 1.3.2\n");
        write_exe(dir.path(), &fake_name("jpegtran"), "#!/bin/sh\necho 'libjpeg-turbo version 3.0.1 (build 20231209)' >&2\n");
        write_exe(dir.path(), &fake_name("zstd"), "#!/bin/sh\nsleep 30\n"); // hangs: must be reported without a version
        let o = DetectOptions { path: Some(dir.path().as_os_str().to_os_string()), ..Default::default() };
        let t0 = std::time::Instant::now();
        let tools = detect_with(&o);
        let get = |id: &str| tools.iter().find(|t| t.id == id).unwrap();
        assert!(get("ffmpeg").found && get("cwebp").found && get("jpegtran").found && !get("avifenc").found);
        assert_eq!(get("avifenc").path, None);
        assert_eq!(get("ffmpeg").path.as_deref(), Some(dir.join(fake_name("ffmpeg")).as_path()));
        if cfg!(unix) {
            assert_eq!(get("ffmpeg").version.as_deref(), Some("6.1.1-3ubuntu5"));
            assert_eq!(get("cwebp").version.as_deref(), Some("1.3.2"));
            assert_eq!(get("jpegtran").version.as_deref(), Some("3.0.1"));
            assert!(get("zstd").found && get("zstd").version.is_none());
            assert!(t0.elapsed() < Duration::from_secs(15), "probe timeout not enforced");
        }
    }

    #[test]
    fn overrides_win_over_path_and_may_be_directories() {
        let path_dir = TempDir::new("path-dir").unwrap();
        let own = TempDir::new("own-dir").unwrap();
        write_exe(path_dir.path(), &fake_name("cjxl"), "#!/bin/sh\necho 'cjxl v0.8.2'\n");
        let mine = write_exe(own.path(), &fake_name("cjxl"), "#!/bin/sh\necho 'cjxl v0.9.0'\n");
        write_exe(own.path(), &fake_name("avifenc"), "#!/bin/sh\necho 'avifenc version: 1.0.4'\n");
        let mut o = DetectOptions { path: Some(path_dir.path().as_os_str().to_os_string()), skip_version_probe: true, ..Default::default() };
        o.overrides.insert("cjxl".into(), mine.clone());
        o.overrides.insert("avifenc".into(), own.path().to_path_buf()); // a directory
        o.overrides.insert("cwebp".into(), own.join("nope")); // a dead override falls back to PATH, then to missing
        let tools = detect_with(&o);
        let get = |id: &str| tools.iter().find(|t| t.id == id).unwrap();
        assert_eq!(get("cjxl").path.as_deref(), Some(mine.as_path()));
        assert!(get("avifenc").found);
        assert!(!get("cwebp").found);
        // extra_dirs come before PATH
        let o2 = DetectOptions { extra_dirs: vec![own.path().to_path_buf()], path: o.path.clone(), skip_version_probe: true, ..Default::default() };
        assert_eq!(detect_with(&o2).iter().find(|t| t.id == "cjxl").unwrap().path.as_deref(), Some(mine.as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_files_and_relative_dirs_are_not_tools() {
        let dir = TempDir::new("noexec").unwrap();
        let p = dir.join("ffmpeg");
        std::fs::write(&p, "x").unwrap(); // mode 0644
        assert_eq!(which_in("ffmpeg", [dir.path().to_path_buf()]), None);
        assert_eq!(which_in("ffmpeg", [PathBuf::from("."), PathBuf::new()]), None);
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("ffmpeg version 6.1.1-3ubuntu5 Copyright"), Some("6.1.1-3ubuntu5".into()));
        assert_eq!(parse_version("7-Zip (a) 23.01 (x64) : Copyright"), Some("23.01".into()));
        assert_eq!(parse_version("cjxl v0.8.2 [AVX2,SSE4,SSE2]"), Some("0.8.2".into()));
        assert_eq!(parse_version("avifenc version: 1.0.4 (dav1d [dec]:1.2.1)"), Some("1.0.4".into()));
        assert_eq!(parse_version("zstd command line interface 64-bits v1.5.5, by Yann Collet"), Some("1.5.5".into()));
        assert_eq!(parse_version("\n\n"), None);
        assert_eq!(parse_version("no numbers here"), None);
    }

    #[test]
    fn availability_reasons() {
        let tools = vec![tool_at("ffmpeg", "/x/ffmpeg")];
        assert_eq!(availability(&tools, &["ffmpeg"]), Availability::Available);
        assert_eq!(availability(&[], &["ffmpeg"]), Availability::Disabled { reason: "Requires ffmpeg on PATH".into() });
        assert_eq!(availability(&tools, &["ffmpeg", "ffprobe", "x"]), Availability::Disabled { reason: "Requires ffprobe and x on PATH".into() });
        assert_eq!(availability_any(&tools, &["cwebp", "ffmpeg"]), Availability::Available);
        assert_eq!(
            availability_any(&[], &["cwebp", "avifenc", "cjxl"]),
            Availability::Disabled { reason: "Requires cwebp, avifenc or cjxl on PATH".into() }
        );
        assert!(matches!(require(&[], "ffmpeg"), Err(Error::ToolMissing(_))));
        assert_eq!(require(&tools, "ffmpeg").unwrap(), Path::new("/x/ffmpeg"));
    }
}
