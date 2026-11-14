//! One acting MyJanitor per data folder (F-13 of the adversarial review). Owner: W3-D.
//!
//! Two processes appending to the same journal and acting on the same files could
//! interleave records, and the second one's start-up recovery would "settle" the first
//! one's in-flight intents (deleting its temporary outputs). The engine therefore takes
//! `<data>/myjanitor.lock` when it starts with persistent state, whoever starts it: the
//! GUI, the CLI or a scheduled `run-profile`. An engine that cannot take it still reads
//! (scan, review, History), but skips crash recovery and refuses every command that
//! changes files ([`crate::Engine::require_writable`]); the GUI refuses to open a second
//! window.
//!
//! - Windows: the file is kept open without sharing for the life of the process, so a
//!   second open fails with a sharing violation; the OS releases it on exit or crash.
//! - Linux: the file is created with `create_new` and holds the owner's pid; a lock whose
//!   pid no longer runs (`/proc/<pid>` missing) is stale and taken over. The same process
//!   may take it again (an engine rebuilt in-process).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Held for the life of the engine.
#[derive(Debug)]
pub struct InstanceLock {
    /// The open lock file; `None` only while dropping.
    file: Option<File>,
    path: PathBuf,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // Close before removing: on Windows the file is open without delete sharing, so
        // removing it while the handle is open fails and would leave the file behind.
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
fn open_exclusive(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new().read(true).write(true).create(true).truncate(true).share_mode(0).open(path)
}

#[cfg(not(windows))]
fn open_exclusive(path: &Path) -> std::io::Result<File> {
    for _ in 0..4 {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(f) => return Ok(f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        match text.trim().parse::<u32>() {
            Ok(pid) if pid == std::process::id() => {
                return OpenOptions::new().write(true).truncate(true).open(path);
            }
            Ok(pid) if Path::new(&format!("/proc/{pid}")).exists() => {
                return Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, format!("held by pid {pid}")));
            }
            // A dead pid: stale. An empty or unreadable file may be another process
            // between `create_new` and writing its pid: give it a moment once.
            Ok(_) => {
                let _ = std::fs::remove_file(path);
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "the lock file is in use"))
}

/// Take the lock for `data_dir`, or say why not.
pub fn acquire(data_dir: &Path) -> Result<InstanceLock, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;
    let path = data_dir.join("myjanitor.lock");
    let mut file = open_exclusive(&path).map_err(|e| {
        format!("another MyJanitor (window, command line or scheduled run) is using {} ({e})", data_dir.display())
    })?;
    let _ = write!(file, "{}", std::process::id());
    let _ = file.sync_all();
    Ok(InstanceLock { file: Some(file), path })
}

#[cfg(test)]
mod tests {
    #[test]
    fn lock_is_released_on_drop_and_stale_locks_are_taken_over() {
        let dir = std::env::temp_dir().join(format!("myjanitor-test-lock-{}", std::process::id()));
        let l = super::acquire(&dir).unwrap();
        assert!(dir.join("myjanitor.lock").is_file());
        drop(l);
        assert!(!dir.join("myjanitor.lock").exists());
        // A stale lock (dead pid) is taken over.
        std::fs::write(dir.join("myjanitor.lock"), "4294967294").unwrap();
        #[cfg(not(windows))]
        assert!(super::acquire(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_live_other_process_holds_the_lock() {
        let dir = std::env::temp_dir().join(format!("myjanitor-test-lock2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // pid 1 always runs.
        std::fs::write(dir.join("myjanitor.lock"), "1").unwrap();
        assert!(super::acquire(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
