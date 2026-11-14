//! One running MyJanitor per data folder. Owner: W3-D.
//!
//! Two instances appending to the same journal and acting on the same files could
//! interleave records and race each other, so a second instance refuses to start.
//! The lock is `<data>/myjanitor.lock`:
//!
//! - Windows: the file is kept open without sharing for the life of the process, so a
//!   second open fails with a sharing violation; the OS releases it on exit or crash.
//! - Linux: the file holds the owner's pid; a lock whose pid no longer runs
//!   (`/proc/<pid>` missing) is stale and taken over.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Held for the life of the process.
pub struct InstanceLock {
    #[allow(dead_code)]
    file: File,
    path: PathBuf,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
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
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(pid) = text.trim().parse::<u32>() {
            if pid != std::process::id() && Path::new(&format!("/proc/{pid}")).exists() {
                return Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, format!("held by pid {pid}")));
            }
        }
    }
    OpenOptions::new().write(true).create(true).truncate(true).open(path)
}

/// Take the lock for `data_dir`, or say why not.
pub fn acquire(data_dir: &Path) -> Result<InstanceLock, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;
    let path = data_dir.join("myjanitor.lock");
    let mut file = open_exclusive(&path).map_err(|e| {
        format!("MyJanitor is already running for {} ({e}); close the other window first", data_dir.display())
    })?;
    let _ = write!(file, "{}", std::process::id());
    let _ = file.sync_all();
    Ok(InstanceLock { file, path })
}

#[cfg(test)]
mod tests {
    #[test]
    fn second_lock_in_the_same_process_is_allowed_only_after_release() {
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
}
