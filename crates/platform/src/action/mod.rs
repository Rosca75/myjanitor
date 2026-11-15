//! Write-side platform code. **Owner: W2-E** (except `fs_compress`, owned by W3-B).
//! Only `mj-actions` calls into this module, after its safety checks.
//!
//! Besides the `ActionPlatform` back-ends this module exports the two filesystem
//! primitives every reversible action is built on:
//!
//! - [`rename_no_replace`]: an atomic rename that **fails** instead of replacing an
//!   existing destination (Linux `renameat2(RENAME_NOREPLACE)`, Windows
//!   `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`). `std::fs::rename` replaces
//!   silently on both platforms, which is exactly what "never overwrite" forbids.
//! - [`sync_dir`]: make a rename or a new directory entry durable.

pub mod cloud;
pub mod elevation;
pub mod fs_compress;
pub mod links;
pub mod trash;

use std::io;
use std::path::Path;

use mj_core::{Error, Result};

/// True when `err` means "source and destination are on different volumes".
pub fn is_cross_device(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::CrossesDevices {
        return true;
    }
    // EXDEV (18) on Linux, ERROR_NOT_SAME_DEVICE (17) on Windows.
    matches!(err.raw_os_error(), Some(18) if cfg!(unix)) || matches!(err.raw_os_error(), Some(17) if cfg!(windows))
}

/// Rename `from` to `to`, failing with `AlreadyExists` when `to` exists (file, folder or
/// link). Never crosses volumes: a cross-device move fails with an error for which
/// [`is_cross_device`] is true, and the caller copies instead.
pub fn rename_no_replace_io(from: &Path, to: &Path) -> io::Result<()> {
    imp::rename_no_replace(from, to)
}

/// [`rename_no_replace_io`] with the error mapped onto `mj_core::Error` (path = `to` for
/// "already exists", `from` otherwise).
pub fn rename_no_replace(from: &Path, to: &Path) -> Result<()> {
    rename_no_replace_io(from, to).map_err(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            Error::Conflict(format!("{} already exists; nothing was overwritten", to.display()))
        } else {
            Error::io(from, e)
        }
    })
}

/// Flush a directory's entries to disk (Linux: `fsync` on the directory; Windows: a no-op,
/// NTFS metadata is journaled and directory handles cannot be flushed this way).
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::os::raw::{c_char, c_int, c_uint};
    use std::path::Path;

    use crate::ffi::linux::cpath;

    const AT_FDCWD: c_int = -100;
    const RENAME_NOREPLACE: c_uint = 1;
    const EINVAL: i32 = 22;
    const ENOSYS: i32 = 38;

    extern "C" {
        // glibc >= 2.28.
        fn renameat2(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }

    pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        let a = cpath(from)?;
        let b = cpath(to)?;
        // SAFETY: both pointers are valid NUL-terminated strings living until after the call;
        // AT_FDCWD makes the paths relative to the working directory (they are absolute).
        let rc = unsafe { renameat2(AT_FDCWD, a.as_ptr(), AT_FDCWD, b.as_ptr(), RENAME_NOREPLACE) };
        if rc == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            // Filesystem without RENAME_NOREPLACE support (some network filesystems).
            Some(EINVAL) | Some(ENOSYS) => fallback(from, to),
            _ => Err(err),
        }
    }

    /// Without kernel support: files use link + unlink (link never replaces), folders
    /// check-then-rename (a narrow race, documented in docs/decisions/W2-E.md).
    fn fallback(from: &Path, to: &Path) -> io::Result<()> {
        let md = std::fs::symlink_metadata(from)?;
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        if md.is_file() {
            std::fs::hard_link(from, to)?;
            std::fs::remove_file(from)
        } else {
            std::fs::rename(from, to)
        }
    }
}

#[cfg(windows)]
#[allow(non_camel_case_types, non_snake_case, clippy::upper_case_acronyms)]
mod imp {
    use std::io;
    use std::path::Path;

    use crate::ffi::windows::{wide_path, BOOL, DWORD, LPCWSTR};

    /// Flush the move to disk before returning.
    const MOVEFILE_WRITE_THROUGH: DWORD = 0x8;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(lpExistingFileName: LPCWSTR, lpNewFileName: LPCWSTR, dwFlags: DWORD) -> BOOL;
    }

    pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        let a = wide_path(from);
        let b = wide_path(to);
        // SAFETY: both buffers are NUL-terminated UTF-16 strings that outlive the call.
        // Without MOVEFILE_REPLACE_EXISTING the call fails with ERROR_ALREADY_EXISTS /
        // ERROR_FILE_EXISTS when `to` exists; without MOVEFILE_COPY_ALLOWED it fails with
        // ERROR_NOT_SAME_DEVICE across volumes.
        let ok = unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if ok != 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(80) | Some(183) => Err(io::Error::new(io::ErrorKind::AlreadyExists, err)),
            _ => Err(err),
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use std::io;
    use std::path::Path;

    pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        std::fs::rename(from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn rename_never_replaces() {
        let t = TempDir::new("rnr").unwrap();
        let a = t.write("a", b"A").unwrap();
        let b = t.write("b", b"B").unwrap();
        assert_eq!(rename_no_replace(&a, &b).unwrap_err().code(), "conflict");
        assert_eq!(std::fs::read(&b).unwrap(), b"B");
        assert_eq!(std::fs::read(&a).unwrap(), b"A");
        let d1 = t.mkdir("d1").unwrap();
        let d2 = t.mkdir("d2").unwrap();
        assert!(rename_no_replace(&d1, &d2).is_err(), "an empty folder is not replaced either");
        rename_no_replace(&a, &t.join("c")).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read(t.join("c")).unwrap(), b"A");
        sync_dir(t.path()).unwrap();
    }
}
