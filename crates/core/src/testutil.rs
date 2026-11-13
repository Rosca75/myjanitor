//! Test helpers (feature `testutil`): an in-house temporary directory
//! (no `tempfile` crate) and fixture-tree builders.
//!
//! Every destructive test must operate only inside a [`TempDir`] it created.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temp dir, removed recursively on drop.
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
    keep: bool,
}

impl TempDir {
    /// Create `"<tmp>/myjanitor-test-<pid>-<n>-<label>"`.
    pub fn new(label: &str) -> io::Result<TempDir> {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let safe: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let path = std::env::temp_dir().join(format!("myjanitor-test-{}-{n}-{nanos}-{safe}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(TempDir { path, keep: false })
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Join a relative path.
    pub fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
        self.path.join(rel)
    }

    /// Write a file (creating parents) and return its path.
    pub fn write(&self, rel: impl AsRef<Path>, content: &[u8]) -> io::Result<PathBuf> {
        let p = self.path.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&p, content)?;
        Ok(p)
    }

    /// Create a directory (and parents) and return its path.
    pub fn mkdir(&self, rel: impl AsRef<Path>) -> io::Result<PathBuf> {
        let p = self.path.join(rel);
        fs::create_dir_all(&p)?;
        Ok(p)
    }

    /// Keep the directory after drop (debugging).
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Set a file's modification time (no external crate): uses `File::set_modified`.
pub fn set_mtime(path: &Path, t: std::time::SystemTime) -> io::Result<()> {
    let f = fs::OpenOptions::new().write(true).open(path)?;
    f.set_modified(t)
}

/// Snapshot of a tree for "state before == state after undo" assertions:
/// sorted `(relative path, is_dir, content)` triples. Symlinks are recorded by target.
pub fn snapshot(root: &Path) -> io::Result<Vec<(String, bool, Vec<u8>)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, bool, Vec<u8>)>) -> io::Result<()> {
        for e in fs::read_dir(dir)? {
            let e = e?;
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            let ft = e.file_type()?;
            if ft.is_symlink() {
                out.push((rel, false, fs::read_link(&p)?.to_string_lossy().into_owned().into_bytes()));
            } else if ft.is_dir() {
                out.push((rel, true, Vec::new()));
                walk(root, &p, out)?;
            } else {
                out.push((rel, false, fs::read(&p)?));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_dir_is_removed_on_drop() {
        let p;
        {
            let t = TempDir::new("drop").unwrap();
            t.write("a/b.txt", b"x").unwrap();
            t.mkdir("c").unwrap();
            p = t.path().to_path_buf();
            let snap = snapshot(t.path()).unwrap();
            assert_eq!(snap.len(), 3);
            assert_eq!(snap[1], ("a/b.txt".to_string(), false, b"x".to_vec()));
        }
        assert!(!p.exists());
    }
}
