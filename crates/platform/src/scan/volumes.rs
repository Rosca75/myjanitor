//! Volumes / drives with free space (drive bars, `drives_list`). Owner: W2-A.
//!
//! - Linux: `/proc/self/mountinfo` (parsed by [`parse_mountinfo`], pure) plus
//!   `statvfs(3)` through hand-written FFI. Pseudo filesystems (proc, sysfs,
//!   cgroup, tmpfs, squashfs snaps...) are not listed; a device mounted at
//!   several places (bind mounts) is listed once, at its shortest mount point.
//! - Windows: `GetLogicalDriveStringsW`, `GetDriveTypeW` (fixed and removable
//!   drives), `GetVolumeInformationW`, `GetDiskFreeSpaceExW`;
//!   `volume_of` uses `GetVolumePathNameW`.

use std::path::{Path, PathBuf};

use mj_core::Result;

use crate::traits::{VolumeFeatures, VolumeInfo};

/// One line of `/proc/self/mountinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub major: u32,
    pub minor: u32,
    /// Root of the mount within its filesystem (`/` unless a bind mount).
    pub root: String,
    pub mount_point: PathBuf,
    pub fs_type: String,
    pub source: String,
    pub read_only: bool,
}

/// Decode the octal escapes the kernel uses in mountinfo (`\040` = space).
pub fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            let v = (b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0');
            out.push(v);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse `/proc/self/mountinfo`. Malformed lines are skipped. Pure.
pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let l: Vec<&str> = left.split(' ').collect();
        let r: Vec<&str> = right.split(' ').collect();
        if l.len() < 6 || r.len() < 2 {
            continue;
        }
        let Some((maj, min)) = l[2].split_once(':') else { continue };
        let (Ok(major), Ok(minor)) = (maj.parse(), min.parse()) else { continue };
        out.push(Mount {
            major,
            minor,
            root: unescape_mount(l[3]),
            mount_point: PathBuf::from(unescape_mount(l[4])),
            fs_type: r[0].to_string(),
            source: unescape_mount(r[1]),
            read_only: l[5].split(',').any(|o| o == "ro"),
        });
    }
    out
}

/// Filesystem types that are never user storage.
const PSEUDO: &[&str] = &[
    "proc",
    "sysfs",
    "devtmpfs",
    "devpts",
    "tmpfs",
    "ramfs",
    "cgroup",
    "cgroup2",
    "pstore",
    "bpf",
    "securityfs",
    "debugfs",
    "tracefs",
    "configfs",
    "fusectl",
    "mqueue",
    "hugetlbfs",
    "autofs",
    "binfmt_misc",
    "rpc_pipefs",
    "nsfs",
    "squashfs",
    "efivarfs",
    "selinuxfs",
    "fuse.gvfsd-fuse",
    "fuse.portal",
    "fuse.snapfuse",
    "nfsd",
    "overlay_internal",
];

/// True for mounts worth showing as drives. Pure.
pub fn is_user_mount(m: &Mount) -> bool {
    if PSEUDO.contains(&m.fs_type.as_str()) {
        return false;
    }
    let p = m.mount_point.to_string_lossy();
    let system = ["/proc", "/sys", "/dev", "/run", "/snap", "/boot/efi"];
    let under_system = system.iter().any(|s| p == *s || p.starts_with(&format!("{s}/")));
    !under_system || p.starts_with("/run/media/")
}

/// The user-visible mounts: pseudo filesystems removed, one entry per device
/// (shortest mount point wins), sorted by mount point. Pure.
pub fn user_mounts(all: &[Mount]) -> Vec<Mount> {
    let mut keep: Vec<Mount> = Vec::new();
    for m in all.iter().filter(|m| is_user_mount(m)) {
        let dev = (m.major, m.minor);
        match keep.iter_mut().find(|k| (k.major, k.minor) == dev) {
            Some(k) => {
                if m.mount_point.as_os_str().len() < k.mount_point.as_os_str().len() {
                    *k = m.clone();
                }
            }
            None => keep.push(m.clone()),
        }
    }
    keep.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    keep
}

/// The mount holding `path`: the longest mount point it lies within. Pure.
pub fn mount_of<'a>(all: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    all.iter()
        .filter(|m| mj_core::pathutil::is_within(&m.mount_point, path))
        .max_by_key(|m| m.mount_point.components().count())
}

/// Capabilities by Linux filesystem type. Pure.
pub fn linux_features(fs_type: &str, read_only: bool) -> VolumeFeatures {
    let no_links = ["vfat", "exfat", "msdos", "fuseblk", "iso9660", "udf"];
    VolumeFeatures {
        hardlinks: !no_links.contains(&fs_type),
        reflinks: matches!(fs_type, "btrfs" | "xfs" | "bcachefs"),
        wof: false,
        btrfs_compression: fs_type == "btrfs",
        trash: !read_only,
        mft: false,
    }
}

/// `statvfs` numbers: total, free, available to unprivileged users.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Space {
    pub total: u64,
    pub free: u64,
    pub available: u64,
}

/// Decode a 64-bit glibc `struct statvfs` (all leading fields are `unsigned long`
/// or `fsblkcnt_t`, 8 bytes each): f_bsize, f_frsize, f_blocks, f_bfree, f_bavail. Pure.
pub fn parse_statvfs(words: &[u64]) -> Option<Space> {
    let &[bsize, frsize, blocks, bfree, bavail] = words.get(0..5)? else { return None };
    let unit = if frsize != 0 { frsize } else { bsize };
    Some(Space {
        total: blocks.saturating_mul(unit),
        free: bfree.saturating_mul(unit),
        available: bavail.saturating_mul(unit),
    })
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::raw::{c_char, c_int};
    use std::path::Path;

    use mj_core::{Error, Result};

    use super::*;
    use crate::ffi::linux::cpath;

    extern "C" {
        fn statvfs(path: *const c_char, buf: *mut u64) -> c_int;
    }

    pub fn space(path: &Path) -> Result<Space> {
        let c = cpath(path).map_err(|e| Error::io(path, e))?;
        // 16 x u64 = 128 bytes, larger than the 112-byte x86_64 struct statvfs.
        let mut buf = [0u64; 16];
        // SAFETY: `c` is NUL-terminated and `buf` is a writable, 8-byte aligned buffer larger
        // than struct statvfs; both outlive the call.
        let rc = unsafe { statvfs(c.as_ptr(), buf.as_mut_ptr()) };
        if rc != 0 {
            return Err(Error::io(path, crate::ffi::linux::last_error()));
        }
        parse_statvfs(&buf).ok_or_else(|| Error::Internal("statvfs buffer".into()))
    }

    fn mounts() -> Result<Vec<Mount>> {
        let text = std::fs::read_to_string("/proc/self/mountinfo").map_err(|e| Error::io("/proc/self/mountinfo", e))?;
        Ok(parse_mountinfo(&text))
    }

    fn removable(m: &Mount) -> bool {
        let p = m.mount_point.to_string_lossy();
        if p.starts_with("/media/") || p.starts_with("/run/media/") {
            return true;
        }
        let base = format!("/sys/dev/block/{}:{}", m.major, m.minor);
        for f in [format!("{base}/removable"), format!("{base}/../removable")] {
            if let Ok(s) = std::fs::read_to_string(&f) {
                return s.trim() == "1";
            }
        }
        false
    }

    fn label(m: &Mount) -> String {
        let Ok(src) = std::fs::canonicalize(&m.source) else { return String::new() };
        let Ok(rd) = std::fs::read_dir("/dev/disk/by-label") else { return String::new() };
        for e in rd.flatten() {
            if std::fs::canonicalize(e.path()).ok().as_deref() == Some(src.as_path()) {
                let raw = e.file_name().to_string_lossy().into_owned();
                return decode_hex_escapes(&raw);
            }
        }
        String::new()
    }

    fn info(m: &Mount) -> VolumeInfo {
        let s = space(&m.mount_point).unwrap_or_default();
        VolumeInfo {
            id: m.mount_point.to_string_lossy().into_owned(),
            mount: m.mount_point.clone(),
            label: label(m),
            fs_type: m.fs_type.clone(),
            total: s.total,
            free: s.free,
            available: s.available,
            removable: removable(m),
            features: linux_features(&m.fs_type, m.read_only),
        }
    }

    pub fn volumes() -> Result<Vec<VolumeInfo>> {
        Ok(user_mounts(&mounts()?).iter().map(info).collect())
    }

    pub fn volume_of(path: &Path) -> Result<VolumeInfo> {
        let all = mounts()?;
        let abs = mj_core::pathutil::normalize_lexical(path);
        let m = mount_of(&all, &abs).ok_or_else(|| Error::NotFound(path.to_path_buf()))?;
        Ok(info(m))
    }
}

/// `\x20` escapes used by udev in `/dev/disk/by-label` names. Pure.
pub fn decode_hex_escapes(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1] == b'x' {
            if let Ok(v) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(windows)]
mod win {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    use mj_core::{Error, Result};

    use super::super::win::*;
    use super::*;
    use crate::ffi::windows::wide;

    fn info(root: &str, removable: bool) -> Result<VolumeInfo> {
        let w = wide(OsStr::new(root));
        let mut label = [0u16; 261];
        let mut fs = [0u16; 64];
        let (mut serial, mut max_comp, mut flags) = (0u32, 0u32, 0u32);
        // SAFETY: `w` is NUL-terminated; buffers and out pointers are valid for the stated lengths.
        let ok = unsafe {
            GetVolumeInformationW(
                w.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                &mut serial,
                &mut max_comp,
                &mut flags,
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        if ok == 0 {
            return Err(Error::io(root, last_error()));
        }
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        // SAFETY: `w` is NUL-terminated; the three out pointers are valid u64s.
        let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) };
        if ok == 0 {
            return Err(Error::io(root, last_error()));
        }
        let fs_type = from_wide_buf(&fs);
        let ntfs = fs_type.eq_ignore_ascii_case("NTFS");
        let id = if root.len() == 3 && root.as_bytes()[1] == b':' { root[..2].to_string() } else { root.to_string() };
        Ok(VolumeInfo {
            id,
            mount: PathBuf::from(root),
            label: from_wide_buf(&label),
            fs_type,
            total,
            free,
            available: avail,
            removable,
            features: VolumeFeatures {
                hardlinks: flags & FILE_SUPPORTS_HARD_LINKS != 0,
                reflinks: flags & FILE_SUPPORTS_BLOCK_REFCOUNTING != 0,
                wof: ntfs,
                btrfs_compression: false,
                trash: flags & FILE_READ_ONLY_VOLUME == 0,
                mft: ntfs,
            },
        })
    }

    fn drive_type(root: &str) -> u32 {
        let w = wide(OsStr::new(root));
        // SAFETY: `w` is NUL-terminated.
        unsafe { GetDriveTypeW(w.as_ptr()) }
    }

    pub fn volumes() -> Result<Vec<VolumeInfo>> {
        let mut buf = [0u16; 512];
        // SAFETY: `buf` is writable for the stated length.
        let n = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
        if n == 0 || n > buf.len() {
            return Err(Error::io("drives", last_error()));
        }
        // No "insert a disk" dialogs for empty card readers.
        // SAFETY: SetErrorMode only changes process error-mode flags.
        let old = unsafe { SetErrorMode(0x0001 | 0x8000) };
        let mut out = Vec::new();
        for root in String::from_utf16_lossy(&buf[..n]).split('\0').filter(|s| !s.is_empty()) {
            let t = drive_type(root);
            if t == DRIVE_FIXED || t == DRIVE_REMOVABLE {
                if let Ok(v) = info(root, t == DRIVE_REMOVABLE) {
                    out.push(v);
                }
            }
        }
        // SAFETY: restores the previous flags.
        unsafe { SetErrorMode(old) };
        Ok(out)
    }

    pub fn volume_of(path: &Path) -> Result<VolumeInfo> {
        let w = wide(path.as_os_str());
        let mut buf = [0u16; 1024];
        // SAFETY: `w` is NUL-terminated; `buf` is writable for the stated length.
        if unsafe { GetVolumePathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } == 0 {
            return Err(Error::io(path, last_error()));
        }
        let root = from_wide_buf(&buf);
        let removable = drive_type(&root) == DRIVE_REMOVABLE;
        info(&root, removable)
    }
}

/// All volumes worth showing.
pub fn volumes() -> Result<Vec<VolumeInfo>> {
    #[cfg(target_os = "linux")]
    {
        linux::volumes()
    }
    #[cfg(windows)]
    {
        win::volumes()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Err(mj_core::Error::unsupported("Drive list", "not available on this platform"))
    }
}

/// Volume holding `path`.
pub fn volume_of(path: &Path) -> Result<VolumeInfo> {
    #[cfg(target_os = "linux")]
    {
        linux::volume_of(path)
    }
    #[cfg(windows)]
    {
        win::volume_of(path)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = path;
        Err(mj_core::Error::unsupported("Drive list", "not available on this platform"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
22 27 0:21 / /proc rw,relatime - proc proc rw
25 24 0:23 / /dev/shm rw,relatime - tmpfs tmpfs rw
27 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
28 27 8:1 / /boot/efi rw - vfat /dev/sda1 rw
30 27 0:40 / /home rw,relatime shared:5 - btrfs /dev/nvme0n1p3 rw,compress=zstd
31 27 0:40 /data /srv/data rw - btrfs /dev/nvme0n1p3 rw
32 27 8:17 / /run/media/me/My\\040Disk ro,nosuid - exfat /dev/sdb1 ro
33 27 7:3 / /snap/core/1 ro - squashfs /dev/loop3 ro
garbage line
";

    #[test]
    fn parses_mountinfo_with_escapes() {
        let m = parse_mountinfo(SAMPLE);
        assert_eq!(m.len(), 8);
        let disk = m.iter().find(|m| m.fs_type == "exfat").unwrap();
        assert_eq!(disk.mount_point, PathBuf::from("/run/media/me/My Disk"));
        assert!(disk.read_only);
        assert_eq!((disk.major, disk.minor), (8, 17));
        assert_eq!(m[4].root, "/");
        assert_eq!(m[5].root, "/data");
        assert_eq!(unescape_mount(r"a\134b\011c\0"), "a\\b\tc\\0");
    }

    #[test]
    fn user_mounts_drop_pseudo_and_bind_duplicates() {
        let all = parse_mountinfo(SAMPLE);
        let u = user_mounts(&all);
        let points: Vec<_> = u.iter().map(|m| m.mount_point.to_string_lossy().into_owned()).collect();
        assert_eq!(points, vec!["/", "/home", "/run/media/me/My Disk"]);
        assert_eq!(mount_of(&all, Path::new("/home/me/x")).unwrap().mount_point, PathBuf::from("/home"));
        assert_eq!(mount_of(&all, Path::new("/srv/data/y")).unwrap().root, "/data");
        assert_eq!(mount_of(&all, Path::new("/usr/bin")).unwrap().mount_point, PathBuf::from("/"));
    }

    #[test]
    fn features_and_statvfs() {
        let b = linux_features("btrfs", false);
        assert!(b.reflinks && b.btrfs_compression && b.hardlinks && b.trash && !b.mft);
        let f = linux_features("vfat", true);
        assert!(!f.hardlinks && !f.trash && !f.reflinks);
        let s = parse_statvfs(&[4096, 4096, 1000, 400, 300, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(s, Space { total: 4_096_000, free: 1_638_400, available: 1_228_800 });
        let s = parse_statvfs(&[512, 0, 10, 5, 5]).unwrap();
        assert_eq!(s.total, 5120);
        assert!(parse_statvfs(&[]).is_none());
        assert_eq!(decode_hex_escapes(r"My\x20Disk"), "My Disk");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_volume_of_temp_dir() {
        let t = mj_core::testutil::TempDir::new("vol").unwrap();
        let v = volume_of(t.path()).unwrap();
        assert!(v.total >= v.free);
        assert!(mj_core::pathutil::is_within(&v.mount, t.path()));
        let _ = volumes().unwrap();
    }
}
