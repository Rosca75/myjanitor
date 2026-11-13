//! `metadata(path)`: lstat semantics, never follows links. Owner: W2-A.
//! Wave 1 fallback: `std::fs::symlink_metadata` plus the std `MetadataExt` traits.

use std::path::Path;

use mj_core::{Attributes, EntryKind, Error, FileEntry, Result, Timestamp};

/// Metadata of one path (links are reported as `EntryKind::Link`, not followed).
pub fn metadata(path: &Path) -> Result<FileEntry> {
    let md = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
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
    platform_fields(&md, &mut e);
    if md.permissions().readonly() {
        e.attrs |= Attributes::READONLY;
    }
    Ok(e)
}

#[cfg(unix)]
fn platform_fields(md: &std::fs::Metadata, e: &mut FileEntry) {
    use std::os::unix::fs::MetadataExt;
    e.allocated = Some(md.blocks().saturating_mul(512));
    e.changed = Some(Timestamp(md.ctime().saturating_mul(1_000_000_000).saturating_add(md.ctime_nsec())));
    e.file_id = Some(mj_core::FileId { volume: md.dev(), index: md.ino() });
    e.links = u32::try_from(md.nlink()).ok();
    e.owner = Some(format!("uid:{}", md.uid()));
    if e.name().starts_with('.') {
        e.attrs |= Attributes::HIDDEN;
    }
    // Sparse: fewer bytes allocated than the logical size by more than one block.
    if e.kind == EntryKind::File && md.blocks().saturating_mul(512) + 4096 < md.len() {
        e.attrs |= Attributes::SPARSE;
    }
}

#[cfg(windows)]
fn platform_fields(md: &std::fs::Metadata, e: &mut FileEntry) {
    use std::os::windows::fs::MetadataExt;
    e.attrs |= attrs_from_windows(md.file_attributes());
    e.created = Some(Timestamp::from_filetime(md.creation_time()));
    e.accessed = Some(Timestamp::from_filetime(md.last_access_time()));
    e.modified = Some(Timestamp::from_filetime(md.last_write_time()));
    if e.is_online_only() {
        e.allocated = Some(0);
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_fields(_: &std::fs::Metadata, _: &mut FileEntry) {}

/// Map Win32 `FILE_ATTRIBUTE_*` bits to [`Attributes`]. Pure; tested on every platform.
pub fn attrs_from_windows(a: u32) -> Attributes {
    const MAP: &[(u32, Attributes)] = &[
        (0x1, Attributes::READONLY),
        (0x2, Attributes::HIDDEN),
        (0x4, Attributes::SYSTEM),
        (0x20, Attributes::ARCHIVE),
        (0x100, Attributes::TEMPORARY),
        (0x200, Attributes::SPARSE),
        (0x400, Attributes::REPARSE_POINT),
        (0x800, Attributes::COMPRESSED),
        (0x1000, Attributes::OFFLINE),
        (0x4000, Attributes::ENCRYPTED),
        (0x8_0000, Attributes::PINNED),
        // FILE_ATTRIBUTE_RECALL_ON_OPEN / RECALL_ON_DATA_ACCESS: content is not local.
        (0x4_0000, Attributes::ONLINE_ONLY),
        (0x40_0000, Attributes::ONLINE_ONLY),
    ];
    let mut out = Attributes::empty();
    for (bit, attr) in MAP {
        if a & bit != 0 {
            out |= *attr;
        }
    }
    out
}

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
        }
        assert_eq!(metadata(&t.join("missing")).unwrap_err().code(), "not_found");
    }

    #[test]
    fn windows_attribute_mapping() {
        let a = attrs_from_windows(0x2 | 0x4 | 0x40_0000);
        assert!(a.contains(Attributes::HIDDEN | Attributes::SYSTEM | Attributes::ONLINE_ONLY));
        assert!(!a.contains(Attributes::READONLY));
    }
}
