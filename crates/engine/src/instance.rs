//! One acting MyJanitor per data folder (F-13, G-06 of the adversarial reviews). Owner: W3-D.
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
//! The lock is an OS lock held on the open file for the life of the process
//! ([`mj_platform::lockfile`]: `flock` on Linux, an unshared handle on Windows), never a
//! pid written in a file: the kernel releases it on exit or crash, so a stale file after a
//! crash never blocks start-up and a reused pid is never trusted. The file itself is never
//! deleted (deleting it would let a second process lock a new file while the first still
//! holds the old one). The pid written into it is for people reading it only. Engines of
//! one process share one lock (an engine rebuilt in-process, tests): it is released when
//! the last of them is dropped.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Locks this process holds, by data folder key, with the count of engines sharing each.
type Held = HashMap<String, (Arc<File>, usize)>;
static HELD: Mutex<Option<Held>> = Mutex::new(None);

/// Held for the life of the engine.
#[derive(Debug)]
pub struct InstanceLock {
    key: String,
    _file: Arc<File>,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let mut held = mj_core::lock(&HELD);
        if let Some(map) = held.as_mut() {
            if let Some((_, n)) = map.get_mut(&self.key) {
                *n -= 1;
                if *n == 0 {
                    // Closing the last handle releases the OS lock.
                    map.remove(&self.key);
                }
            }
        }
    }
}

fn key_of(data_dir: &Path) -> String {
    let p = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    mj_core::pathutil::path_key(&p)
}

/// Take the lock for `data_dir`, or say why not.
pub fn acquire(data_dir: &Path) -> Result<InstanceLock, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;
    let key = key_of(data_dir);
    let mut held = mj_core::lock(&HELD);
    let map = held.get_or_insert_with(HashMap::new);
    if let Some((file, n)) = map.get_mut(&key) {
        *n += 1;
        return Ok(InstanceLock { key, _file: file.clone() });
    }
    let path: PathBuf = data_dir.join("myjanitor.lock");
    let busy = || format!("another MyJanitor (window, command line or scheduled run) is using {}", data_dir.display());
    let mut file = match mj_platform::lockfile::open_locked(&path) {
        Ok(Some(f)) => f,
        Ok(None) => return Err(busy()),
        Err(e) => return Err(format!("{} ({e})", busy())),
    };
    // Informational only: who holds it (never read back to decide anything).
    let _ = file.set_len(0);
    let _ = file.rewind();
    let _ = write!(file, "{}", std::process::id());
    let _ = file.sync_all();
    let file = Arc::new(file);
    map.insert(key.clone(), (file.clone(), 1));
    Ok(InstanceLock { key, _file: file })
}

#[cfg(test)]
mod tests {
    use mj_core::testutil::TempDir;

    #[test]
    fn g06_a_stale_lock_file_never_blocks_and_the_lock_is_shared_in_process() {
        let t = TempDir::new("lock").unwrap();
        let dir = t.join("data");
        std::fs::create_dir_all(&dir).unwrap();
        // A file left by a crash, naming a pid that runs (pid 1 always does).
        std::fs::write(dir.join("myjanitor.lock"), "1").unwrap();
        let a = super::acquire(&dir).expect("a stale file is not a lock");
        let b = super::acquire(&dir).expect("engines of one process share the lock");
        // Another holder (another open of the file, as another process would be) is refused.
        assert!(mj_platform::lockfile::open_locked(&dir.join("myjanitor.lock")).unwrap().is_none());
        drop(a);
        assert!(mj_platform::lockfile::open_locked(&dir.join("myjanitor.lock")).unwrap().is_none(), "b holds it");
        drop(b);
        let other = mj_platform::lockfile::open_locked(&dir.join("myjanitor.lock")).unwrap();
        assert!(other.is_some(), "released with the last engine");
        assert!(super::acquire(&dir).is_err(), "now the other holder has it");
        drop(other);
        assert!(super::acquire(&dir).is_ok());
        assert!(dir.join("myjanitor.lock").exists(), "never deleted");
    }
}
