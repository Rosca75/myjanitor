//! Private scratch folders (F-03 of the adversarial review).
//!
//! Every intermediate output (an encoder's output, a dry-run result, a recompression
//! stage) is written inside a folder that exactly one job created: the folder name is
//! unique (process id, a per-process counter and the clock) and it is created with
//! `create_dir`, never `create_dir_all`, so creation fails rather than reuses a folder
//! that already exists. Two jobs, two dry runs or two processes sharing the same parent
//! (`<cache>/tmp`) can therefore never write to, or delete, each other's files. Fixed
//! file names inside such a folder are safe for the same reason.
//!
//! The folder and everything in it is removed when the [`ScratchDir`] is dropped
//! (`remove_dir_all` does not follow links, and the folder only ever holds files the job
//! wrote).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A folder created exclusively for one job, removed on drop.
#[derive(Debug)]
pub struct ScratchDir {
    dir: PathBuf,
}

impl ScratchDir {
    /// Create `<parent>/mj-<label>-<pid>-<n>-<nanos>` (the parent is created if missing).
    /// `label` is reduced to ASCII letters, digits and `-`.
    pub fn new(parent: &Path, label: &str) -> Result<ScratchDir> {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        let label: String =
            label.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(32).collect::<String>();
        for _ in 0..64 {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or_default();
            let dir = parent.join(format!("mj-{label}-{}-{n}-{nanos}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(ScratchDir { dir }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::io(&dir, e)),
            }
        }
        Err(Error::Conflict(format!("no free scratch folder name under {}", parent.display())))
    }

    /// The folder.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// A path inside the folder.
    pub fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_unique_exclusive_and_removed() {
        let base = std::env::temp_dir().join(format!("mj-scratch-test-{}", std::process::id()));
        let a = ScratchDir::new(&base, "audio out").unwrap();
        let b = ScratchDir::new(&base, "audio out").unwrap();
        assert_ne!(a.path(), b.path());
        assert!(a.path().file_name().unwrap().to_string_lossy().starts_with("mj-audioout-"));
        std::fs::write(a.join("x"), b"1").unwrap();
        let pa = a.path().to_path_buf();
        drop(a);
        assert!(!pa.exists());
        assert!(b.path().is_dir());
        drop(b);
        let _ = std::fs::remove_dir(&base);
    }
}
