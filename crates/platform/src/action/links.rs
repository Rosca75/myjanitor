//! Hard links (`CreateHardLinkW` / `link`) and reflinks (`FICLONE` ioctl on
//! Btrfs/XFS). Owner: W2-E.
//!
//! Both functions create a **new** name and never replace an existing one: the
//! caller (`mj-actions`) creates the link under a temporary name beside the
//! duplicate and renames it over the duplicate itself after verifying content.
//! ReFS block cloning (`FSCTL_DUPLICATE_EXTENTS_TO_FILE`) is not targeted: on
//! Windows `reflink` returns `Unsupported`.

use std::path::Path;

use mj_core::{Error, Result};

/// Create `new_link` as a hard link to `existing` (both on one volume). Never follows a
/// symlink at `existing` (Linux `linkat` without `AT_SYMLINK_FOLLOW`).
pub fn hardlink(existing: &Path, new_link: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        win::create_hard_link(existing, new_link)
    }
    #[cfg(not(windows))]
    {
        std::fs::hard_link(existing, new_link).map_err(|e| map_link_error(new_link, e))
    }
}

#[cfg(not(windows))]
fn map_link_error(path: &Path, e: std::io::Error) -> Error {
    if crate::action::is_cross_device(&e) {
        Error::unsupported("Hard link", "the files are on different volumes")
    } else {
        Error::io(path, e)
    }
}

/// Create `dst` (which must not exist) as a copy-on-write clone of `src`.
/// `Unsupported` when the filesystem cannot clone (ext4, tmpfs, NTFS...).
pub fn reflink(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::reflink(src, dst)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (src, dst);
        Err(Error::unsupported("Reflink", "copy-on-write clones are only supported on Linux (Btrfs, XFS)"))
    }
}

/// `FICLONE` = `_IOW(0x94, 9, int)`.
pub const FICLONE: u64 = 0x4004_9409;

/// errno values meaning "this filesystem / pair cannot clone".
pub fn reflink_unsupported_errno(errno: i32) -> bool {
    // EXDEV, EINVAL, ENOTTY, EOPNOTSUPP, ENOSYS, EPERM (some FUSE filesystems), EBADF on special files.
    matches!(errno, 18 | 22 | 25 | 95 | 38 | 1)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::{File, OpenOptions};
    use std::os::raw::c_ulong;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    use std::path::Path;

    use mj_core::{Error, Result};

    use super::{reflink_unsupported_errno, FICLONE};
    use crate::ffi::linux::ioctl;

    const O_NOFOLLOW: i32 = 0o400000;

    pub fn reflink(src: &Path, dst: &Path) -> Result<()> {
        let s = OpenOptions::new().read(true).custom_flags(O_NOFOLLOW).open(src).map_err(|e| Error::io(src, e))?;
        if !s.metadata().map_err(|e| Error::io(src, e))?.is_file() {
            return Err(Error::InvalidInput(format!("not a regular file: {}", src.display())));
        }
        let d: File = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(O_NOFOLLOW)
            .open(dst)
            .map_err(|e| Error::io(dst, e))?;
        // SAFETY: both descriptors are open for the duration of the call; FICLONE takes the
        // source descriptor as its integer argument.
        let rc = unsafe { ioctl(d.as_raw_fd(), FICLONE as c_ulong, s.as_raw_fd()) };
        if rc == 0 {
            return d.sync_all().map_err(|e| Error::io(dst, e));
        }
        let err = std::io::Error::last_os_error();
        drop(d);
        let _ = std::fs::remove_file(dst);
        match err.raw_os_error() {
            Some(n) if reflink_unsupported_errno(n) => {
                Err(Error::unsupported("Reflink", "this filesystem does not support copy-on-write clones"))
            }
            _ => Err(Error::io(dst, err)),
        }
    }
}

#[cfg(windows)]
#[allow(non_camel_case_types, non_snake_case, clippy::upper_case_acronyms)]
mod win {
    use std::path::Path;

    use mj_core::{Error, Result};

    use crate::ffi::windows::{wide_path, BOOL, LPCWSTR, SECURITY_ATTRIBUTES};

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateHardLinkW(
            lpFileName: LPCWSTR,
            lpExistingFileName: LPCWSTR,
            lpSecurityAttributes: *mut SECURITY_ATTRIBUTES,
        ) -> BOOL;
    }

    pub fn create_hard_link(existing: &Path, new_link: &Path) -> Result<()> {
        let a = wide_path(new_link);
        let b = wide_path(existing);
        // SAFETY: both buffers are NUL-terminated and outlive the call; no security attributes.
        let ok = unsafe { CreateHardLinkW(a.as_ptr(), b.as_ptr(), std::ptr::null_mut()) };
        if ok != 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if crate::action::is_cross_device(&e) {
            Err(Error::unsupported("Hard link", "the files are on different volumes"))
        } else if e.raw_os_error() == Some(1142) {
            // ERROR_TOO_MANY_LINKS: NTFS allows 1023 links per file.
            Err(Error::unsupported("Hard link", "the file already has the maximum number of links"))
        } else {
            Err(Error::io(new_link, e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn hard_link_shares_content_and_refuses_existing() {
        let t = TempDir::new("hl").unwrap();
        let a = t.write("a", b"same").unwrap();
        let b = t.join("b");
        hardlink(&a, &b).unwrap();
        assert_eq!(std::fs::read(&b).unwrap(), b"same");
        assert!(hardlink(&a, &b).is_err(), "never replaces");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&a).unwrap().ino(), std::fs::metadata(&b).unwrap().ino());
        }
    }

    #[test]
    fn reflink_clones_or_reports_unsupported() {
        let t = TempDir::new("rl").unwrap();
        let a = t.write("a", b"clone me").unwrap();
        let b = t.join("b");
        match reflink(&a, &b) {
            Ok(()) => assert_eq!(std::fs::read(&b).unwrap(), b"clone me"),
            Err(e) => {
                assert_eq!(e.code(), "unsupported", "{e}");
                assert!(!b.exists(), "no partial clone is left behind");
            }
        }
        assert!(reflink_unsupported_errno(95));
        assert!(!reflink_unsupported_errno(13));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&a, t.join("l")).unwrap();
            assert!(reflink(&t.join("l"), &t.join("c")).is_err(), "links are not followed");
        }
    }
}
