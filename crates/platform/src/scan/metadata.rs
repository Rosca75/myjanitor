//! `metadata(path)`: lstat semantics, never follows links. Owner: W2-A.
//!
//! - Linux: `statx` (see [`super::statx`]) for logical and allocated size,
//!   birth time, inode change time, hard-link count, file id (`st_dev` +
//!   `st_ino`) and the compressed / immutable / encrypted attributes; falls
//!   back to `std::fs::symlink_metadata` when `statx` is unavailable.
//! - Windows: `GetFileAttributesExW` (no handle, so cloud placeholders are
//!   never recalled), the reparse tag from `FindFirstFileExW` when the entry is
//!   a reparse point (only name surrogates, i.e. symlinks, junctions and mount
//!   points, become [`EntryKind::Link`]; cloud placeholders stay files), then
//!   for local entries a `FILE_READ_ATTRIBUTES` handle for the file id and the
//!   link count, and `GetCompressedFileSizeW` for compressed, sparse and WOF
//!   files. Every path goes through `wide_path` (`\\?\` long paths).

use std::path::Path;

use mj_core::{Attributes, EntryKind, FileEntry, Result, Timestamp};

/// Metadata of one path (links are reported as `EntryKind::Link`, not followed).
pub fn metadata(path: &Path) -> Result<FileEntry> {
    native(path)
}

#[cfg(target_os = "linux")]
fn native(path: &Path) -> Result<FileEntry> {
    match super::statx::statx_path(path) {
        Some(Ok(s)) => Ok(super::statx::entry_from_statx(path, &s)),
        Some(Err(e)) => Err(mj_core::Error::io(path, e)),
        None => std_metadata(path),
    }
}

#[cfg(windows)]
fn native(path: &Path) -> Result<FileEntry> {
    win_md::metadata(path)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn native(path: &Path) -> Result<FileEntry> {
    std_metadata(path)
}

/// Portable fallback: `std::fs::symlink_metadata` plus the std `MetadataExt` traits.
#[cfg_attr(windows, allow(dead_code))]
pub fn std_metadata(path: &Path) -> Result<FileEntry> {
    let md = std::fs::symlink_metadata(path).map_err(|e| mj_core::Error::io(path, e))?;
    let ft = md.file_type();
    let kind = if ft.is_symlink() {
        EntryKind::Link
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let size = if kind == EntryKind::File { md.len() } else { 0 };
    let mut e = FileEntry::new(path, kind, size);
    e.modified = md.modified().ok().map(Timestamp::from_system_time);
    e.accessed = md.accessed().ok().map(Timestamp::from_system_time);
    e.created = md.created().ok().map(Timestamp::from_system_time);
    std_platform_fields(&md, &mut e);
    if md.permissions().readonly() {
        e.attrs |= Attributes::READONLY;
    }
    Ok(e)
}

#[cfg(unix)]
fn std_platform_fields(md: &std::fs::Metadata, e: &mut FileEntry) {
    use std::os::unix::fs::MetadataExt;
    e.allocated = Some(md.blocks().saturating_mul(512));
    e.changed = Some(Timestamp(md.ctime().saturating_mul(1_000_000_000).saturating_add(md.ctime_nsec())));
    e.file_id = Some(mj_core::FileId { volume: md.dev(), index: md.ino() });
    e.links = u32::try_from(md.nlink()).ok();
    e.owner = Some(format!("uid:{}", md.uid()));
    if e.name().starts_with('.') {
        e.attrs |= Attributes::HIDDEN;
    }
    if e.kind == EntryKind::File && md.blocks().saturating_mul(512) + 4096 < md.len() {
        e.attrs |= Attributes::SPARSE;
    }
}

#[cfg(windows)]
fn std_platform_fields(md: &std::fs::Metadata, e: &mut FileEntry) {
    use std::os::windows::fs::MetadataExt;
    e.attrs |= attrs_from_windows(md.file_attributes());
    if e.is_online_only() {
        e.allocated = Some(0);
    }
}

#[cfg(not(any(unix, windows)))]
fn std_platform_fields(_: &std::fs::Metadata, _: &mut FileEntry) {}

/// `FILE_ATTRIBUTE_*` values used here.
pub mod win_attr {
    pub const READONLY: u32 = 0x1;
    pub const HIDDEN: u32 = 0x2;
    pub const SYSTEM: u32 = 0x4;
    pub const DIRECTORY: u32 = 0x10;
    pub const ARCHIVE: u32 = 0x20;
    pub const DEVICE: u32 = 0x40;
    pub const TEMPORARY: u32 = 0x100;
    pub const SPARSE_FILE: u32 = 0x200;
    pub const REPARSE_POINT: u32 = 0x400;
    pub const COMPRESSED: u32 = 0x800;
    pub const OFFLINE: u32 = 0x1000;
    pub const ENCRYPTED: u32 = 0x4000;
    pub const RECALL_ON_OPEN: u32 = 0x4_0000;
    pub const PINNED: u32 = 0x8_0000;
    pub const UNPINNED: u32 = 0x10_0000;
    pub const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
}

/// Reparse tags used here.
pub mod reparse {
    /// Bit 29: the reparse point names another entry (symlink, junction, mount point).
    pub const NAME_SURROGATE: u32 = 0x2000_0000;
    pub const MOUNT_POINT: u32 = 0xA000_0003;
    pub const SYMLINK: u32 = 0xA000_000C;
    pub const WOF: u32 = 0x8000_0017;
    /// `IO_REPARSE_TAG_CLOUD` .. `IO_REPARSE_TAG_CLOUD_F` differ only in bits 12-15.
    pub const CLOUD: u32 = 0x9000_001A;
    pub const CLOUD_MASK: u32 = 0xFFFF_0FFF;

    /// Symlinks, junctions and mount points: never followed.
    pub fn is_name_surrogate(tag: u32) -> bool {
        tag & NAME_SURROGATE != 0
    }

    /// OneDrive / cloud-files placeholders (`cldflt.sys`).
    pub fn is_cloud(tag: u32) -> bool {
        tag & CLOUD_MASK == CLOUD
    }
}

/// Map Win32 `FILE_ATTRIBUTE_*` bits to [`Attributes`]. Pure; tested on every platform.
pub fn attrs_from_windows(a: u32) -> Attributes {
    use win_attr as w;
    const MAP: &[(u32, Attributes)] = &[
        (w::READONLY, Attributes::READONLY),
        (w::HIDDEN, Attributes::HIDDEN),
        (w::SYSTEM, Attributes::SYSTEM),
        (w::ARCHIVE, Attributes::ARCHIVE),
        (w::TEMPORARY, Attributes::TEMPORARY),
        (w::SPARSE_FILE, Attributes::SPARSE),
        (w::REPARSE_POINT, Attributes::REPARSE_POINT),
        (w::COMPRESSED, Attributes::COMPRESSED),
        (w::OFFLINE, Attributes::OFFLINE),
        (w::ENCRYPTED, Attributes::ENCRYPTED),
        (w::PINNED, Attributes::PINNED),
        // RECALL_ON_OPEN / RECALL_ON_DATA_ACCESS: content is not local (online-only).
        (w::RECALL_ON_OPEN, Attributes::ONLINE_ONLY),
        (w::RECALL_ON_DATA_ACCESS, Attributes::ONLINE_ONLY),
    ];
    let mut out = Attributes::empty();
    for (bit, attr) in MAP {
        if a & bit != 0 {
            out |= *attr;
        }
    }
    out
}

/// Build an entry from what a Win32 directory entry says (attributes, reparse tag,
/// size, FILETIME ticks). Pure; used by `metadata`, `read_dir` and the MFT path.
///
/// A reparse point whose tag could not be read is treated as a link: the safe
/// default is to never descend into something that might point elsewhere.
pub fn entry_from_win32(
    path: &Path,
    attrs: u32,
    tag: Option<u32>,
    size: u64,
    created: u64,
    accessed: u64,
    written: u64,
) -> FileEntry {
    let reparse = attrs & win_attr::REPARSE_POINT != 0;
    let link = reparse && tag.is_none_or(reparse::is_name_surrogate);
    let kind = if link {
        EntryKind::Link
    } else if attrs & win_attr::DIRECTORY != 0 {
        EntryKind::Dir
    } else if attrs & win_attr::DEVICE != 0 {
        EntryKind::Other
    } else {
        EntryKind::File
    };
    let mut e = FileEntry::new(path, kind, if kind == EntryKind::File { size } else { 0 });
    e.attrs = attrs_from_windows(attrs);
    let ts = |t: u64| (t != 0).then(|| Timestamp::from_filetime(t));
    e.created = ts(created);
    e.accessed = ts(accessed);
    e.modified = ts(written);
    if let Some(tag) = tag.filter(|_| reparse) {
        if reparse::is_cloud(tag) && !e.is_online_only() {
            e.attrs |= Attributes::CLOUD_LOCAL;
        }
        if tag == reparse::WOF {
            e.attrs |= Attributes::WOF_COMPRESSED;
        }
    }
    if e.is_online_only() && kind == EntryKind::File {
        e.allocated = Some(0);
    }
    e
}

/// Round `size` up to whole clusters (allocation estimate for plain NTFS files).
pub fn round_to_cluster(size: u64, cluster: u64) -> u64 {
    if cluster == 0 {
        size
    } else {
        size.div_ceil(cluster).saturating_mul(cluster)
    }
}

/// The volume root of a Windows path string: `C:\` for `C:\x\y`, `\\srv\share\`
/// for UNC paths (with or without the `\\?\` prefix). Pure.
pub fn windows_volume_root(path: &str) -> Option<String> {
    let p = path.replace('/', "\\");
    let p = p.strip_prefix(r"\\?\").map(|s| s.to_string()).unwrap_or(p);
    let b = p.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return Some(format!("{}:\\", (b[0] as char).to_ascii_uppercase()));
    }
    let unc = p.strip_prefix(r"UNC\").or_else(|| p.strip_prefix(r"\\"))?;
    let mut parts = unc.split('\\').filter(|s| !s.is_empty());
    let (server, share) = (parts.next()?, parts.next()?);
    Some(format!(r"\\{server}\{share}\").to_ascii_uppercase())
}

#[cfg(windows)]
mod win_md {
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    use mj_core::{Attributes, EntryKind, Error, FileEntry, FileId, Result};

    use super::super::win::*;
    use super::{entry_from_win32, round_to_cluster, win_attr, windows_volume_root};
    use crate::ffi::windows::{open_no_follow, wide, wide_path, INVALID_HANDLE_VALUE};

    /// Find data of one path (not a pattern), without opening the file.
    pub fn find_data(w: &[u16]) -> Option<WIN32_FIND_DATAW> {
        let mut fd = WIN32_FIND_DATAW::zeroed();
        // SAFETY: `w` is NUL-terminated and `fd` is a writable WIN32_FIND_DATAW; both outlive the call.
        let h = unsafe {
            FindFirstFileExW(
                w.as_ptr(),
                FIND_EX_INFO_BASIC,
                &mut fd,
                FIND_EX_SEARCH_NAME_MATCH,
                std::ptr::null_mut(),
                0,
            )
        };
        if h == INVALID_HANDLE_VALUE || h.is_null() {
            return None;
        }
        let _close = FindHandle(h);
        Some(fd)
    }

    fn cluster_size(path: &Path) -> Option<u64> {
        static CACHE: OnceLock<Mutex<HashMap<String, Option<u64>>>> = OnceLock::new();
        let root = windows_volume_root(&path.to_string_lossy())?;
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(v) = mj_core::lock(cache).get(&root) {
            return *v;
        }
        let w = wide(std::ffi::OsStr::new(&root));
        let (mut spc, mut bps, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
        // SAFETY: `w` is NUL-terminated; the four out pointers are valid u32s.
        let ok = unsafe { GetDiskFreeSpaceW(w.as_ptr(), &mut spc, &mut bps, &mut free, &mut total) };
        let v = (ok != 0).then(|| u64::from(spc) * u64::from(bps)).filter(|&c| c > 0);
        mj_core::lock(cache).insert(root, v);
        v
    }

    fn compressed_size(w: &[u16]) -> Option<u64> {
        let mut high = 0u32;
        // SAFETY: `w` is NUL-terminated; `high` is a valid out pointer.
        let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
        if low == INVALID_FILE_SIZE && last_error().raw_os_error().unwrap_or(0) != 0 {
            return None;
        }
        Some((u64::from(high) << 32) | u64::from(low))
    }

    /// Handle-based identity (file id + link count); `None` when no handle can be opened.
    pub fn identity(path: &Path) -> Option<(FileId, u32)> {
        let h = open_no_follow(path, FILE_READ_ATTRIBUTES).ok()?;
        let mut bh = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: `h` is a valid open handle; `bh` is a writable struct of the documented size.
        let ok = unsafe { GetFileInformationByHandle(h.0, &mut bh) };
        if ok == 0 {
            return None;
        }
        let index = (u64::from(bh.nFileIndexHigh) << 32) | u64::from(bh.nFileIndexLow);
        Some((FileId { volume: u64::from(bh.dwVolumeSerialNumber), index }, bh.nNumberOfLinks))
    }

    /// Allocated size of a regular file.
    fn allocated(path: &Path, w: &[u16], e: &FileEntry) -> u64 {
        if e.is_online_only() {
            return 0;
        }
        let packed = e.attrs.intersects(Attributes::COMPRESSED | Attributes::SPARSE | Attributes::WOF_COMPRESSED);
        if packed {
            if let Some(c) = compressed_size(w) {
                return c;
            }
        }
        match cluster_size(path) {
            Some(c) => round_to_cluster(e.size, c),
            None => e.size,
        }
    }

    pub fn metadata(path: &Path) -> Result<FileEntry> {
        let w = wide_path(path);
        let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
        // SAFETY: `w` is NUL-terminated; `data` is a WIN32_FILE_ATTRIBUTE_DATA as required
        // by GetFileExInfoStandard. Both outlive the call.
        let ok =
            unsafe { GetFileAttributesExW(w.as_ptr(), GET_FILE_EX_INFO_STANDARD, &mut data as *mut _ as *mut c_void) };
        let (attrs, size, c, a, m, mut tag) = if ok != 0 {
            let size = (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow);
            (
                data.dwFileAttributes,
                size,
                data.ftCreationTime.ticks(),
                data.ftLastAccessTime.ticks(),
                data.ftLastWriteTime.ticks(),
                None,
            )
        } else {
            let err = last_error();
            // Sharing violations (pagefile.sys) still answer through the directory entry.
            match find_data(&w) {
                Some(fd) => (
                    fd.dwFileAttributes,
                    (u64::from(fd.nFileSizeHigh) << 32) | u64::from(fd.nFileSizeLow),
                    fd.ftCreationTime.ticks(),
                    fd.ftLastAccessTime.ticks(),
                    fd.ftLastWriteTime.ticks(),
                    Some(fd.dwReserved0),
                ),
                None => return Err(Error::io(path, err)),
            }
        };
        if attrs & win_attr::REPARSE_POINT != 0 && tag.is_none() {
            tag = find_data(&w).map(|fd| fd.dwReserved0);
        }
        let mut e = entry_from_win32(path, attrs, tag, size, c, a, m);
        // Online-only placeholders are never opened (an open may recall content).
        if e.kind != EntryKind::Link && !e.is_online_only() {
            if let Some((id, links)) = identity(path) {
                e.file_id = Some(id);
                e.links = Some(links);
            }
        }
        if e.kind == EntryKind::File {
            e.allocated = Some(allocated(path, &w, &e));
        }
        Ok(e)
    }
}

#[cfg(windows)]
pub(crate) use win_md::identity as win_identity;

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn reads_files_dirs_and_links_without_following() {
        let t = TempDir::new("md").unwrap();
        let f = t.write("a.txt", b"hello").unwrap();
        let e = metadata(&f).unwrap();
        assert_eq!((e.kind, e.size), (EntryKind::File, 5));
        assert!(e.modified.is_some());
        assert_eq!(metadata(t.path()).unwrap().kind, EntryKind::Dir);
        #[cfg(unix)]
        {
            let l = t.join("link");
            std::os::unix::fs::symlink(t.path(), &l).unwrap();
            assert_eq!(metadata(&l).unwrap().kind, EntryKind::Link);
            let s = std_metadata(&f).unwrap();
            let n = metadata(&f).unwrap();
            assert_eq!((s.size, s.file_id, s.links, s.allocated), (n.size, n.file_id, n.links, n.allocated));
            assert_eq!(s.modified, n.modified);
        }
        assert_eq!(metadata(&t.join("missing")).unwrap_err().code(), "not_found");
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_share_a_file_id() {
        let t = TempDir::new("md-hl").unwrap();
        let f = t.write("a.txt", b"hello").unwrap();
        std::fs::hard_link(&f, t.join("b.txt")).unwrap();
        let (a, b) = (metadata(&f).unwrap(), metadata(&t.join("b.txt")).unwrap());
        assert_eq!(a.file_id, b.file_id);
        assert_eq!(a.links, Some(2));
    }

    #[test]
    fn windows_attribute_mapping() {
        let a = attrs_from_windows(0x2 | 0x4 | 0x40_0000);
        assert!(a.contains(Attributes::HIDDEN | Attributes::SYSTEM | Attributes::ONLINE_ONLY));
        assert!(!a.contains(Attributes::READONLY));
        assert!(attrs_from_windows(0x4_0000).contains(Attributes::ONLINE_ONLY));
        assert!(attrs_from_windows(0x1000 | 0x800 | 0x4000)
            .contains(Attributes::OFFLINE | Attributes::COMPRESSED | Attributes::ENCRYPTED));
    }

    #[test]
    fn win32_entries_classify_reparse_points() {
        let p = Path::new("x");
        let ft = 132_000_000_000_000_000u64;
        // Junction and symlink: links. Unknown tag: link (safe default).
        let j = entry_from_win32(p, 0x10 | 0x400, Some(reparse::MOUNT_POINT), 0, 0, 0, ft);
        assert_eq!(j.kind, EntryKind::Link);
        assert_eq!(entry_from_win32(p, 0x20 | 0x400, Some(reparse::SYMLINK), 9, 0, 0, 0).kind, EntryKind::Link);
        assert_eq!(entry_from_win32(p, 0x10 | 0x400, None, 0, 0, 0, 0).kind, EntryKind::Link);
        // Hydrated OneDrive file: a file, flagged cloud-local.
        let h = entry_from_win32(p, 0x20 | 0x400, Some(0x9000_301A), 100, ft, ft, ft);
        assert_eq!((h.kind, h.size), (EntryKind::File, 100));
        assert!(h.attrs.contains(Attributes::CLOUD_LOCAL));
        assert_eq!(h.modified, Some(Timestamp::from_filetime(ft)));
        // Online-only placeholder: excluded from totals (allocated 0).
        let o = entry_from_win32(p, 0x20 | 0x400 | 0x40_0000 | 0x10_0000, Some(reparse::CLOUD), 5000, 0, 0, 0);
        assert!(o.is_online_only());
        assert!(!o.attrs.contains(Attributes::CLOUD_LOCAL));
        assert_eq!((o.allocated, o.reclaimable()), (Some(0), 0));
        // WOF-compressed file.
        let w = entry_from_win32(p, 0x20 | 0x400, Some(reparse::WOF), 10, 0, 0, 0);
        assert!(w.attrs.contains(Attributes::WOF_COMPRESSED));
        assert_eq!(w.kind, EntryKind::File);
        // Plain directory, device.
        assert_eq!(entry_from_win32(p, 0x10, None, 0, 0, 0, 0).kind, EntryKind::Dir);
        assert_eq!(entry_from_win32(p, 0x40, None, 0, 0, 0, 0).kind, EntryKind::Other);
    }

    #[test]
    fn clusters_and_volume_roots() {
        assert_eq!(round_to_cluster(0, 4096), 0);
        assert_eq!(round_to_cluster(1, 4096), 4096);
        assert_eq!(round_to_cluster(8192, 4096), 8192);
        assert_eq!(round_to_cluster(5, 0), 5);
        assert_eq!(windows_volume_root(r"c:\Users\me").as_deref(), Some(r"C:\"));
        assert_eq!(windows_volume_root(r"\\?\D:\x").as_deref(), Some(r"D:\"));
        assert_eq!(windows_volume_root(r"\\srv\Share\a\b").as_deref(), Some(r"\\SRV\SHARE\"));
        assert_eq!(windows_volume_root(r"\\?\UNC\srv\share\a").as_deref(), Some(r"\\SRV\SHARE\"));
        assert_eq!(windows_volume_root("relative"), None);
    }
}
