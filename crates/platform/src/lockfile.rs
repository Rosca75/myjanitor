//! An advisory exclusive lock on an open file, held until the file is closed (G-06).
//!
//! - Unix: `flock(LOCK_EX | LOCK_NB)` through a hand-written declaration (the symbol is in
//!   the C library std already links). The kernel drops the lock when the last descriptor
//!   of the open file is closed, so a crash or power cut never leaves a stale lock, and a
//!   reused pid means nothing. Two opens of the same file are two locks, even in one
//!   process.
//! - Windows: [`open_locked`] opens the file with share mode 0, so any other open (by any
//!   process) fails with a sharing violation until this handle is closed; the OS closes
//!   it on exit or crash.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(unix)]
mod sys {
    use std::os::raw::c_int;
    pub const LOCK_EX: c_int = 2;
    pub const LOCK_NB: c_int = 4;
    extern "C" {
        pub fn flock(fd: c_int, operation: c_int) -> c_int;
    }
}

/// Try to take an exclusive advisory lock on `file` without waiting. `Ok(true)`: taken;
/// `Ok(false)`: another open file holds it.
#[cfg(unix)]
pub fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    use std::os::unix::io::AsRawFd;
    loop {
        // SAFETY: `file` is open for the duration of the call; flock only reads the fd.
        let r = unsafe { sys::flock(file.as_raw_fd(), sys::LOCK_EX | sys::LOCK_NB) };
        if r == 0 {
            return Ok(true);
        }
        let e = io::Error::last_os_error();
        match e.kind() {
            io::ErrorKind::WouldBlock => return Ok(false),
            io::ErrorKind::Interrupted => continue,
            _ => return Err(e),
        }
    }
}

/// Windows: the share mode of [`open_locked`] is the lock.
#[cfg(not(unix))]
pub fn try_lock_exclusive(_file: &File) -> io::Result<bool> {
    Ok(true)
}

/// Open (creating it if needed, never truncating or deleting it) `path` and take the
/// exclusive lock. `Ok(None)` when another holder has it.
pub fn open_locked(path: &Path) -> io::Result<Option<File>> {
    let mut o = OpenOptions::new();
    o.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.share_mode(0);
    }
    let file = match o.open(path) {
        Ok(f) => f,
        // ERROR_SHARING_VIOLATION (32) / ERROR_LOCK_VIOLATION (33): someone holds it.
        #[cfg(windows)]
        Err(e) if matches!(e.raw_os_error(), Some(32) | Some(33)) => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(try_lock_exclusive(&file)?.then_some(file))
}

#[cfg(test)]
mod tests {
    #[test]
    fn second_open_is_refused_until_the_first_is_closed() {
        let t = mj_core::testutil::TempDir::new("lockfile").unwrap();
        let p = t.join("x.lock");
        std::fs::write(&p, "12345 (a stale pid from a crash)").unwrap();
        let a = super::open_locked(&p).unwrap().expect("a stale file never blocks");
        assert!(super::open_locked(&p).unwrap().is_none(), "held");
        drop(a);
        let b = super::open_locked(&p).unwrap();
        assert!(b.is_some(), "released on close");
        assert!(p.exists(), "the file is never deleted");
    }
}
