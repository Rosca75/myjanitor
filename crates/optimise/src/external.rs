//! Running external tools as separate processes (W3-C): argument vectors
//! only (never a shell), stdin closed, stdout and stderr captured with size
//! caps, a timeout, cancellation through `CancelToken` (the child is killed),
//! a per-line hook for progress parsing and a working directory of the
//! caller's choosing (a temp dir).
//!
//! Nothing here starts a shell: `program` is an absolute path found by
//! [`crate::tools`] and every argument is a separate `OsString`, so a file
//! name such as `a; rm -rf ~` is just a name. Names that start with `-` are
//! turned into `./-name` by [`path_arg`] so no tool reads them as an option.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mj_core::{CancelToken, Error, Result};

/// Which pipe a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Per-line observer (progress parsing). Called from a reader thread.
pub type LineHook<'a> = &'a (dyn Fn(Stream, &str) + Sync);

/// How to run one process.
pub struct RunOptions<'a> {
    /// Kill the process after this long (`None` = no limit).
    pub timeout: Option<Duration>,
    /// Bytes of stdout kept (the head); more is read and discarded so the child never blocks.
    pub max_stdout: usize,
    /// Bytes of stderr kept (the tail, where error messages are).
    pub max_stderr: usize,
    pub cancel: Option<&'a CancelToken>,
    /// Working directory (a temp dir; never a user folder).
    pub cwd: Option<&'a Path>,
    pub on_line: Option<LineHook<'a>>,
}

impl Default for RunOptions<'_> {
    fn default() -> Self {
        RunOptions {
            timeout: Some(Duration::from_secs(300)),
            max_stdout: 1 << 20,
            max_stderr: 64 << 10,
            cancel: None,
            cwd: None,
            on_line: None,
        }
    }
}

/// What a finished process left behind.
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    /// Exit code (`None` when killed by a signal or by us).
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub timed_out: bool,
    pub elapsed: Duration,
}

impl RunOutput {
    /// True for exit code 0 and no timeout.
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }

    /// The last `n` characters of stderr, trimmed (for error messages).
    pub fn stderr_tail(&self, n: usize) -> String {
        let t = self.stderr.trim_end();
        let start = t.char_indices().rev().nth(n.saturating_sub(1)).map_or(0, |(i, _)| i);
        t[start..].trim_start().to_string()
    }

    /// Turn a failed run into an error that says what failed and why.
    pub fn ensure_success(&self, tool: &str) -> Result<()> {
        if self.timed_out {
            return Err(Error::Io {
                path: None,
                kind: "timed_out",
                message: format!("{tool} did not finish in time and was stopped"),
            });
        }
        if self.code != Some(0) {
            let code = self.code.map_or("a signal".to_string(), |c| format!("code {c}"));
            return Err(Error::Io {
                path: None,
                kind: "tool_failed",
                message: format!("{tool} failed (exit {code}): {}", self.stderr_tail(400)),
            });
        }
        Ok(())
    }
}

/// A path as a command-line argument: names that would read as an option get a `./` prefix.
pub fn path_arg(p: &Path) -> OsString {
    let s = p.as_os_str();
    if s.to_string_lossy().starts_with('-') {
        let mut o = OsString::from("./");
        o.push(s);
        o
    } else {
        s.to_os_string()
    }
}

/// Build an argument vector from string-like parts (a small readability helper for builders).
pub fn args<I, S>(parts: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    parts.into_iter().map(|s| s.as_ref().to_os_string()).collect()
}

/// Read a pipe to the end, keeping at most `cap` bytes (head or tail) and feeding lines to `hook`.
fn pump(mut r: impl Read, cap: usize, keep_tail: bool, stream: Stream, hook: Option<LineHook<'_>>) -> (Vec<u8>, bool) {
    let (mut kept, mut truncated) = (Vec::new(), false);
    let (mut line, mut buf) = (Vec::new(), [0u8; 8192]);
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let chunk = &buf[..n];
        if keep_tail {
            kept.extend_from_slice(chunk);
            if kept.len() > cap * 2 {
                kept.drain(..kept.len() - cap);
                truncated = true;
            }
        } else if kept.len() < cap {
            let take = (cap - kept.len()).min(n);
            kept.extend_from_slice(&chunk[..take]);
            truncated |= take < n;
        } else {
            truncated = true;
        }
        if let Some(h) = hook {
            for &b in chunk {
                if b == b'\n' || b == b'\r' {
                    if !line.is_empty() {
                        h(stream, &String::from_utf8_lossy(&line));
                        line.clear();
                    }
                } else if line.len() < 8192 {
                    line.push(b);
                }
            }
        }
    }
    if let (Some(h), false) = (hook, line.is_empty()) {
        h(stream, &String::from_utf8_lossy(&line));
    }
    if keep_tail && kept.len() > cap {
        kept.drain(..kept.len() - cap);
        truncated = true;
    }
    (kept, truncated)
}

/// Kill the child and, best effort, anything it started (a stuck helper would keep the pipes open).
fn kill_tree(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    #[cfg(unix)]
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid])
            .creation_flags(0x0800_0000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// Run `program` with `args`. Returns `Err(Cancelled)` when the token fires (the child is killed),
/// `Err(ToolMissing)` when the program cannot be started, and `Ok` otherwise, including non-zero exits
/// and timeouts (see [`RunOutput::ensure_success`]).
pub fn run(program: &Path, args: &[OsString], opts: &RunOptions<'_>) -> Result<RunOutput> {
    if opts.cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(Error::Cancelled);
    }
    let name = program.file_name().map_or_else(|| program.display().to_string(), |n| n.to_string_lossy().into_owned());
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(d) = opts.cwd {
        cmd.current_dir(d);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash from a GUI app
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // own process group, so a kill can reach helpers the tool started
    }
    let started = Instant::now();
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::ToolMissing(name.clone()),
        _ => Error::Io {
            path: Some(program.to_path_buf()),
            kind: "spawn",
            message: format!("could not start {name}: {e}"),
        },
    })?;
    let (out, err) = (child.stdout.take(), child.stderr.take());
    let (stop, hook) = (AtomicBool::new(false), opts.on_line);
    let mut result = RunOutput::default();
    let mut cancelled = false;
    std::thread::scope(|sc| {
        let h_out = sc.spawn(|| out.map(|r| pump(r, opts.max_stdout, false, Stream::Stdout, hook)).unwrap_or_default());
        let h_err = sc.spawn(|| err.map(|r| pump(r, opts.max_stderr, true, Stream::Stderr, hook)).unwrap_or_default());
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) => {}
                Err(_) => break None,
            }
            let over = opts.timeout.is_some_and(|t| started.elapsed() >= t);
            cancelled = opts.cancel.is_some_and(CancelToken::is_cancelled);
            if over || cancelled {
                stop.store(true, Ordering::SeqCst);
                result.timed_out = over && !cancelled;
                kill_tree(&mut child);
                break child.wait().ok();
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        result.code = status.and_then(|s| s.code()).filter(|_| !stop.load(Ordering::SeqCst));
        let (o, ot) = h_out.join().unwrap_or_default();
        let (e, et) = h_err.join().unwrap_or_default();
        result.stdout = o;
        result.stdout_truncated = ot;
        result.stderr = String::from_utf8_lossy(&e).into_owned();
        result.stderr_truncated = et;
    });
    result.elapsed = started.elapsed();
    if cancelled {
        return Err(Error::Cancelled);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn path_arg_guards_leading_dash() {
        assert_eq!(path_arg(Path::new("-rf")), OsString::from("./-rf"));
        assert_eq!(path_arg(Path::new("/tmp/-x")), OsString::from("/tmp/-x"));
        assert_eq!(path_arg(Path::new("a b.mp4")), OsString::from("a b.mp4"));
    }

    #[test]
    fn missing_program_is_tool_missing() {
        let r = run(Path::new("/definitely/not/here/tool"), &[], &RunOptions::default());
        assert!(matches!(r, Err(Error::ToolMissing(_))), "{r:?}");
    }

    #[test]
    fn already_cancelled_token_never_starts_the_process() {
        let c = CancelToken::new();
        c.cancel();
        let opts = RunOptions { cancel: Some(&c), ..Default::default() };
        assert_eq!(run(Path::new("/bin/sh"), &[], &opts).unwrap_err(), Error::Cancelled);
    }

    #[cfg(unix)]
    mod unix {
        use super::*;

        fn sh(script: &str, extra: &[&str]) -> (std::path::PathBuf, Vec<OsString>) {
            let mut a = vec![OsString::from("-c"), OsString::from(script), OsString::from("sh")];
            a.extend(extra.iter().map(OsString::from));
            (Path::new("/bin/sh").to_path_buf(), a)
        }

        #[test]
        fn captures_output_and_exit_code() {
            let (p, a) = sh("echo out; echo err 1>&2; exit 3", &[]);
            let o = run(&p, &a, &RunOptions::default()).unwrap();
            assert_eq!((o.code, o.stdout.as_slice(), o.stderr.trim()), (Some(3), &b"out\n"[..], "err"));
            assert!(!o.success());
            let e = o.ensure_success("sh").unwrap_err();
            assert!(e.to_string().contains("exit code 3") && e.to_string().contains("err"), "{e}");
        }

        #[test]
        fn arguments_are_never_interpreted_by_a_shell() {
            let (p, a) = sh("printf %s \"$1\"", &["; echo pwned; $(id)"]);
            let o = run(&p, &a, &RunOptions::default()).unwrap();
            assert_eq!(String::from_utf8_lossy(&o.stdout), "; echo pwned; $(id)");
        }

        #[test]
        fn stdin_is_closed() {
            let (p, a) = sh("cat; echo done", &[]);
            let o = run(&p, &a, &RunOptions { timeout: Some(Duration::from_secs(5)), ..Default::default() }).unwrap();
            assert!(o.success() && o.stdout == b"done\n", "{o:?}");
        }

        #[test]
        fn output_caps_keep_head_of_stdout_and_tail_of_stderr() {
            let (p, a) = sh("i=0; while [ $i -lt 2000 ]; do echo line$i; echo e$i 1>&2; i=$((i+1)); done", &[]);
            let o = run(&p, &a, &RunOptions { max_stdout: 100, max_stderr: 100, ..Default::default() }).unwrap();
            assert!(o.stdout_truncated && o.stdout.len() == 100 && o.stdout.starts_with(b"line0\n"));
            assert!(o.stderr_truncated && o.stderr.len() <= 100 && o.stderr.trim_end().ends_with("e1999"));
        }

        #[test]
        fn timeout_kills_the_child() {
            let (p, a) = sh("sleep 20", &[]);
            let t = Instant::now();
            let o =
                run(&p, &a, &RunOptions { timeout: Some(Duration::from_millis(200)), ..Default::default() }).unwrap();
            assert!(o.timed_out && !o.success() && o.code.is_none());
            assert!(t.elapsed() < Duration::from_secs(5));
            assert!(o.ensure_success("sleep").unwrap_err().to_string().contains("in time"));
        }

        #[test]
        fn cancel_kills_the_child_promptly() {
            let (p, a) = sh("sleep 20", &[]);
            let c = CancelToken::new();
            let c2 = c.clone();
            let h = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                c2.cancel();
            });
            let t = Instant::now();
            let r = run(&p, &a, &RunOptions { cancel: Some(&c), timeout: None, ..Default::default() });
            h.join().unwrap();
            assert_eq!(r.unwrap_err(), Error::Cancelled);
            assert!(t.elapsed() < Duration::from_secs(5));
        }

        #[test]
        fn working_directory_and_line_hook() {
            let dir = mj_core::testutil::TempDir::new("run-cwd").unwrap();
            let (p, a) = sh("pwd; printf 'a\\rb\\nc'", &[]);
            let seen = Mutex::new(Vec::new());
            let hook = |s: Stream, l: &str| seen.lock().unwrap().push((s, l.to_string()));
            let o =
                run(&p, &a, &RunOptions { cwd: Some(dir.path()), on_line: Some(&hook), ..Default::default() }).unwrap();
            let want = std::fs::canonicalize(dir.path()).unwrap();
            assert_eq!(
                std::fs::canonicalize(String::from_utf8_lossy(&o.stdout).lines().next().unwrap()).unwrap(),
                want
            );
            let lines: Vec<String> = seen.lock().unwrap().iter().map(|(_, l)| l.clone()).collect();
            assert_eq!(&lines[1..], ["a", "b", "c"]);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_cmd_runs_without_a_shell_string() {
        let cmd = Path::new("cmd.exe");
        let o = run(cmd, &args(["/C", "echo", "hi"]), &RunOptions::default()).unwrap();
        assert!(o.success());
    }
}
