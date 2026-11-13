//! Btrfs transparent compression. Owner: W3-B.
//!
//! Compress = set the `btrfs.compression` property to `zstd` (the xattr
//! behind `btrfs property set <file> compression zstd`), then rewrite the
//! existing extents with `BTRFS_IOC_DEFRAG_RANGE` and the compress flag (what
//! `btrfs filesystem defragment -czstd` does). Revert = remove the property
//! and rewrite again without the compress flag. Content never changes; the
//! rewrite unshares extents shared with snapshots or reflinks (more space
//! used, never data lost).
//!
//! Detection is `statfs` `f_type == BTRFS_SUPER_MAGIC`. Hand-written FFI
//! against the glibc symbols std already links; structure layouts and ioctl
//! numbers are from `linux/btrfs.h`, `linux/fs.h` and `linux/magic.h`.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use mj_core::{Error, FsCompressAlgo, Result};

use crate::ffi::linux::{cpath, ioctl, size_t, ssize_t};

/// `BTRFS_SUPER_MAGIC` (`linux/magic.h`).
pub const BTRFS_SUPER_MAGIC: i64 = 0x9123_683E;
/// `FS_IOC_GETFLAGS` = `_IOR('f', 1, long)` on 64-bit (`linux/fs.h`).
pub const FS_IOC_GETFLAGS: u64 = 0x8008_6601;
/// `FS_COMPR_FL` (`linux/fs.h`): compression requested for the inode.
pub const FS_COMPR_FL: u32 = 0x0000_0004;
/// `BTRFS_DEFRAG_RANGE_COMPRESS` (`linux/btrfs.h`).
pub const BTRFS_DEFRAG_RANGE_COMPRESS: u64 = 1;
/// `BTRFS_DEFRAG_RANGE_START_IO` (`linux/btrfs.h`): write back immediately.
pub const BTRFS_DEFRAG_RANGE_START_IO: u64 = 2;
/// `BTRFS_COMPRESS_ZSTD` (`compression.h`, value of `compress_type`).
pub const BTRFS_COMPRESS_ZSTD: u32 = 3;
/// The property xattr.
pub const PROP_COMPRESSION: &str = "btrfs.compression";

/// `struct btrfs_ioctl_defrag_range_args` (`linux/btrfs.h`), 48 bytes.
#[repr(C)]
#[derive(Debug, Default)]
pub struct DefragRangeArgs {
    pub start: u64,
    pub len: u64,
    pub flags: u64,
    pub extent_thresh: u32,
    pub compress_type: u32,
    pub unused: [u32; 4],
}

/// `_IOW(type, nr, size)` from `asm-generic/ioctl.h`.
pub const fn iow(ty: u64, nr: u64, size: u64) -> u64 {
    (1 << 30) | (size << 16) | (ty << 8) | nr
}

/// `BTRFS_IOC_DEFRAG_RANGE` = `_IOW(BTRFS_IOCTL_MAGIC 0x94, 16, struct btrfs_ioctl_defrag_range_args)` = 0x40309410.
pub const BTRFS_IOC_DEFRAG_RANGE: u64 = iow(0x94, 16, std::mem::size_of::<DefragRangeArgs>() as u64);

/// `O_NOFOLLOW` (`asm-generic/fcntl.h`; x86 differs).
#[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
const O_NOFOLLOW: i32 = 0o400000;
#[cfg(not(any(target_arch = "x86_64", target_arch = "x86")))]
const O_NOFOLLOW: i32 = 0o100000;
/// `ENODATA` (`asm-generic/errno.h`): no such attribute.
const ENODATA: i32 = 61;

extern "C" {
    fn statfs(path: *const c_char, buf: *mut c_void) -> c_int;
    fn fgetxattr(fd: c_int, name: *const c_char, value: *mut c_void, size: size_t) -> ssize_t;
    fn fsetxattr(fd: c_int, name: *const c_char, value: *const c_void, size: size_t, flags: c_int) -> c_int;
    fn fremovexattr(fd: c_int, name: *const c_char) -> c_int;
}

/// The filesystem magic of the volume holding `path` (`statfs.f_type`).
pub fn fs_type(path: &Path) -> io::Result<i64> {
    let c = cpath(path)?;
    // `struct statfs` is 120 bytes on x86-64; 16 aligned words leave room on every ABI.
    // `f_type` is its first field (a `long`).
    let mut buf = [0i64; 16];
    // SAFETY: `c` is NUL-terminated; `buf` is writable, aligned and larger than `struct statfs`.
    let r = unsafe { statfs(c.as_ptr(), buf.as_mut_ptr() as *mut c_void) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buf[0])
}

/// True on Btrfs. The magic is compared on its low 32 bits (`f_type` may be sign-extended).
pub fn is_btrfs(path: &Path) -> bool {
    fs_type(path).is_ok_and(|t| t as u32 == BTRFS_SUPER_MAGIC as u32)
}

/// `Ok` when `path` can be compressed here, else the one-line reason.
pub fn support(path: &Path) -> std::result::Result<(), String> {
    if is_btrfs(path) {
        Ok(())
    } else {
        Err("Needs a Btrfs volume".into())
    }
}

/// True when any mounted filesystem is Btrfs (`/proc/self/mounts`).
pub fn any_btrfs_mounted() -> bool {
    std::fs::read_to_string("/proc/self/mounts")
        .map(|m| m.lines().any(|l| l.split_whitespace().nth(2) == Some("btrfs")))
        .unwrap_or(false)
}

fn unsupported() -> Error {
    Error::unsupported("Transparent compression", "Needs a Btrfs volume")
}

/// Open without following a final symlink.
fn open(path: &Path) -> Result<File> {
    std::fs::OpenOptions::new().read(true).custom_flags(O_NOFOLLOW).open(path).map_err(|e| {
        if e.raw_os_error() == Some(40) {
            Error::LinkNotFollowed(path.to_path_buf())
        } else {
            Error::io(path, e)
        }
    })
}

fn cname() -> CString {
    CString::new(PROP_COMPRESSION).unwrap_or_default()
}

/// Current property value; `None` when unset.
fn get_prop(f: &File) -> io::Result<Option<Vec<u8>>> {
    let name = cname();
    let mut buf = [0u8; 64];
    // SAFETY: `name` is NUL-terminated; `buf` is writable for its length.
    let n = unsafe { fgetxattr(f.as_raw_fd(), name.as_ptr(), buf.as_mut_ptr() as *mut c_void, buf.len()) };
    if n < 0 {
        let e = io::Error::last_os_error();
        return if e.raw_os_error() == Some(ENODATA) { Ok(None) } else { Err(e) };
    }
    Ok(Some(buf[..n as usize].to_vec()))
}

fn set_prop(f: &File, value: Option<&[u8]>) -> io::Result<()> {
    let name = cname();
    let r = match value {
        // SAFETY: `name` NUL-terminated; `v` readable for its length.
        Some(v) => unsafe { fsetxattr(f.as_raw_fd(), name.as_ptr(), v.as_ptr() as *const c_void, v.len(), 0) },
        // SAFETY: `name` NUL-terminated.
        None => unsafe { fremovexattr(f.as_raw_fd(), name.as_ptr()) },
    };
    if r != 0 {
        let e = io::Error::last_os_error();
        if value.is_none() && e.raw_os_error() == Some(ENODATA) {
            return Ok(());
        }
        return Err(e);
    }
    Ok(())
}

/// The defrag arguments that rewrite the whole file, compressed or not.
pub fn defrag_args(compress: bool) -> DefragRangeArgs {
    DefragRangeArgs {
        start: 0,
        len: u64::MAX,
        flags: BTRFS_DEFRAG_RANGE_START_IO | if compress { BTRFS_DEFRAG_RANGE_COMPRESS } else { 0 },
        // Rewrite every extent, whatever its size.
        extent_thresh: u32::MAX,
        compress_type: if compress { BTRFS_COMPRESS_ZSTD } else { 0 },
        unused: [0; 4],
    }
}

fn defrag(f: &File, compress: bool) -> io::Result<()> {
    let mut args = defrag_args(compress);
    // SAFETY: `args` matches `struct btrfs_ioctl_defrag_range_args` (layout test) and
    // outlives this synchronous call; the fd is open.
    let r = unsafe { ioctl(f.as_raw_fd(), BTRFS_IOC_DEFRAG_RANGE as _, &mut args as *mut DefragRangeArgs) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn compress(path: &Path, algo: FsCompressAlgo) -> Result<()> {
    if algo != FsCompressAlgo::BtrfsZstd {
        return Err(Error::unsupported("Transparent compression", "WOF algorithms are Windows only"));
    }
    if !is_btrfs(path) {
        return Err(unsupported());
    }
    let f = open(path)?;
    let previous = get_prop(&f).map_err(|e| Error::io(path, e))?;
    set_prop(&f, Some(b"zstd")).map_err(|e| Error::io(path, e))?;
    if let Err(e) = defrag(&f, true) {
        // Leave the file as it was.
        let _ = set_prop(&f, previous.as_deref());
        return Err(Error::io(path, e));
    }
    Ok(())
}

pub fn decompress(path: &Path) -> Result<()> {
    if !is_btrfs(path) {
        return Err(unsupported());
    }
    let f = open(path)?;
    set_prop(&f, None).map_err(|e| Error::io(path, e))?;
    defrag(&f, false).map_err(|e| Error::io(path, e))
}

/// `Some(BtrfsZstd)` for any requested Btrfs compression (zstd, zlib or lzo:
/// the contract has one Btrfs value), `None` otherwise or off Btrfs.
pub fn state(path: &Path) -> Result<Option<FsCompressAlgo>> {
    if !is_btrfs(path) {
        return Ok(None);
    }
    let f = open(path)?;
    match get_prop(&f).map_err(|e| Error::io(path, e))? {
        Some(v) if !v.is_empty() && v != b"no" && v != b"none" => return Ok(Some(FsCompressAlgo::BtrfsZstd)),
        Some(_) => return Ok(None),
        None => {}
    }
    let mut flags: std::os::raw::c_long = 0;
    // SAFETY: FS_IOC_GETFLAGS writes one `long` (the kernel writes an int; the
    // larger slot is safe) into `flags`; the fd is open.
    let r = unsafe { ioctl(f.as_raw_fd(), FS_IOC_GETFLAGS as _, &mut flags as *mut std::os::raw::c_long) };
    if r == 0 && (flags as u32) & FS_COMPR_FL != 0 {
        return Ok(Some(FsCompressAlgo::BtrfsZstd));
    }
    Ok(None)
}

/// Bytes allocated on disk (`st_blocks * 512`; Btrfs reports compressed extents).
pub fn on_disk_size(path: &Path) -> Result<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).map(|m| m.blocks() * 512).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn layouts_match_the_kernel_headers() {
        assert_eq!(std::mem::size_of::<DefragRangeArgs>(), 48);
        assert_eq!(BTRFS_IOC_DEFRAG_RANGE, 0x4030_9410);
        let a = defrag_args(true);
        assert_eq!((a.flags, a.compress_type, a.len), (3, 3, u64::MAX));
        assert_eq!(defrag_args(false).flags, 2);
    }

    #[test]
    fn detection_and_unsupported_path_on_this_filesystem() {
        let t = TempDir::new("btrfs").unwrap();
        let f = t.write("a.txt", &b"hello ".repeat(1000)).unwrap();
        assert!(fs_type(&f).is_ok());
        assert!(fs_type(&t.join("missing")).is_err());
        if is_btrfs(&f) {
            // Real Btrfs: round trip, content unchanged.
            compress(&f, FsCompressAlgo::BtrfsZstd).unwrap();
            assert_eq!(state(&f).unwrap(), Some(FsCompressAlgo::BtrfsZstd));
            decompress(&f).unwrap();
            assert_eq!(state(&f).unwrap(), None);
            assert_eq!(std::fs::read(&f).unwrap(), b"hello ".repeat(1000));
        } else {
            assert_eq!(support(&f), Err("Needs a Btrfs volume".into()));
            assert!(matches!(compress(&f, FsCompressAlgo::BtrfsZstd), Err(Error::Unsupported { .. })));
            assert!(matches!(decompress(&f), Err(Error::Unsupported { .. })));
            assert_eq!(state(&f).unwrap(), None);
        }
        assert!(matches!(compress(&f, FsCompressAlgo::Lzx), Err(Error::Unsupported { .. })));
        assert!(on_disk_size(&f).is_ok());
    }

    #[test]
    fn symlinks_are_not_followed() {
        let t = TempDir::new("btrfs-link").unwrap();
        let f = t.write("a.txt", b"x").unwrap();
        let l = t.join("l");
        std::os::unix::fs::symlink(&f, &l).unwrap();
        assert!(matches!(open(&l), Err(Error::LinkNotFollowed(_))));
    }
}
