//! Diagnostics (XC-3): a small rotating log file in the app data folder, so a user can
//! report a problem that left nothing else behind (the GUI has no console on Windows).
//!
//! What goes in: panic payloads with thread and source location, and one line per failure
//! the engine otherwise turns into a short message (a detector that panicked, a job that
//! stopped unexpectedly). What stays out: file contents, and any path detail beyond what the
//! journal already stores (messages carry the paths of the operation that failed, no more).
//! Std only: a size-capped file plus one rotated copy (`myjanitor.log`, `myjanitor.log.1`).

use std::any::Any;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

/// File name inside the data folder.
pub const LOG_NAME: &str = "myjanitor.log";
/// A log file is rotated before it would grow past this size.
pub const MAX_LOG_BYTES: u64 = 256 * 1024;
/// A message longer than this is cut (panic text can embed large values).
const MAX_MESSAGE_CHARS: usize = 800;

/// A size-capped log file with one rotation.
#[derive(Debug, Clone)]
pub struct LogFile {
    path: PathBuf,
    max_bytes: u64,
}

impl LogFile {
    /// Log at `path`, rotating once the file would exceed `max_bytes`.
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        LogFile { path, max_bytes }
    }

    /// The standard log of a data folder.
    pub fn in_dir(dir: &Path) -> Self {
        LogFile::new(dir.join(LOG_NAME), MAX_LOG_BYTES)
    }

    /// Append `<time> <level> <message>`.
    pub fn log(&self, level: &str, message: &str) {
        self.append(&format!("{} {level} {message}", stamp()));
    }

    /// The rotated copy's path.
    pub fn rotated_path(&self) -> PathBuf {
        let mut s = self.path.clone().into_os_string();
        s.push(".1");
        PathBuf::from(s)
    }

    /// Append one line. Never fails the caller: a log that cannot be written is dropped.
    pub fn append(&self, line: &str) {
        let line = one_line(line);
        let size = fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if size > 0 && size + line.len() as u64 + 1 > self.max_bytes {
            let _ = fs::remove_file(self.rotated_path());
            let _ = fs::rename(&self.path, self.rotated_path());
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Collapse line breaks and cut to the message cap.
fn one_line(s: &str) -> String {
    let flat: String = s.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    if flat.chars().count() > MAX_MESSAGE_CHARS {
        let mut cut: String = flat.chars().take(MAX_MESSAGE_CHARS).collect();
        cut.push_str("...");
        cut
    } else {
        flat
    }
}

/// The text of a caught panic payload.
pub fn panic_text(p: &(dyn Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic without a message".into())
}

static TARGET: Mutex<Option<LogFile>> = Mutex::new(None);
static HOOK: Once = Once::new();

fn target() -> Option<LogFile> {
    TARGET.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn stamp() -> String {
    crate::time::Timestamp::now().to_rfc3339()
}

/// Write to `<data dir>/myjanitor.log` from now on and install the panic hook (once per
/// process; the previous hook still runs). The newest folder wins.
pub fn init(data_dir: &Path) {
    *TARGET.lock().unwrap_or_else(|e| e.into_inner()) = Some(LogFile::in_dir(data_dir));
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_else(|| "?".into());
            log(
                "PANIC",
                &format!("thread '{}' at {place}: {}", thread.name().unwrap_or("<unnamed>"), panic_text(info.payload())),
            );
            previous(info);
        }));
    });
}

/// The current log file, if [`init`] ran.
pub fn log_path() -> Option<PathBuf> {
    target().map(|t| t.path)
}

/// Append `<time> <level> <message>` to the log (a no-op before [`init`]).
pub fn log(level: &str, message: &str) {
    if let Some(t) = target() {
        t.log(level, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn lines_are_flat_and_capped() {
        let d = TempDir::new("diag-lines").unwrap();
        let l = LogFile::new(d.join("x.log"), 10_000);
        l.append("a\nb\r\nc");
        l.append(&"z".repeat(5000));
        let text = fs::read_to_string(d.join("x.log")).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "a b  c");
        assert_eq!(lines.len(), 2);
        assert!(lines[1].len() < 900 && lines[1].ends_with("..."));
    }

    #[test]
    fn the_file_rotates_once_and_stays_bounded() {
        let d = TempDir::new("diag-rotate").unwrap();
        let l = LogFile::new(d.join("x.log"), 200);
        for i in 0..100 {
            l.append(&format!("line {i:03} {}", "p".repeat(40)));
        }
        let cur = fs::metadata(d.join("x.log")).unwrap().len();
        let old = fs::metadata(l.rotated_path()).unwrap().len();
        assert!(cur <= 200 && old <= 200, "{cur} {old}");
        let all = fs::read_to_string(d.join("x.log")).unwrap();
        assert!(all.contains("line 099"), "the newest line is in the live file");
        assert!(!l.rotated_path().with_extension("2").exists());
    }

    #[test]
    fn payload_text_of_both_panic_kinds() {
        let a = std::panic::catch_unwind(|| panic!("static text")).unwrap_err();
        let b = std::panic::catch_unwind(|| panic!("formatted {}", 7)).unwrap_err();
        assert_eq!(panic_text(&*a), "static text");
        assert_eq!(panic_text(&*b), "formatted 7");
    }
}
