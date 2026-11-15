//! NTFS MFT enumeration (`FSCTL_ENUM_USN_DATA`), SCAN-03. Owner: W2-A.
//!
//! Windows only. Steps:
//! 1. find the volume of `root` (`GetVolumePathNameW`), require NTFS, open
//!    the volume (`\\?\Volume{guid}`) for reading. Without administrator
//!    rights this fails with access denied, and the function returns
//!    `Unsupported` so the caller walks directories instead (no elevation is
//!    ever requested);
//! 2. enumerate every MFT entry with `FSCTL_ENUM_USN_DATA` (name, parent
//!    reference, attributes), parsed by [`super::usn`];
//! 3. keep the entries below `root` ([`super::usn::resolve_below`]), never
//!    entering name-surrogate reparse points (junctions, symlinks, mount points);
//! 4. read each entry's FILE record with `FSCTL_GET_NTFS_FILE_RECORD` (no file
//!    open, so online-only placeholders are never recalled), parsed by
//!    [`super::ntfs`], for sizes, times and link count. Records that cannot be
//!    parsed in place (attribute lists) fall back to a per-path stat.
//!
//! Any failure before the first entry is emitted, or a per-entry failure that
//! cannot be recovered, returns an error: the caller discards what it got and
//! walks, so a partial MFT result can never look complete.

use std::path::Path;

#[cfg(not(windows))]
use mj_core::Error;
use mj_core::{CancelToken, FileEntry, FileId, Result};

use super::metadata::{entry_from_win32, win_attr};
use super::ntfs::NtfsFileInfo;

/// Feature name used in `Unsupported` errors.
pub const FEATURE: &str = "Fast indexing";

/// Build the entry for one MFT item from its USN attributes and parsed FILE
/// record. `None` when the record lacks what is needed (the caller stats the
/// path instead). Pure.
pub fn entry_from_mft(
    path: &Path,
    usn_attributes: u32,
    info: &NtfsFileInfo,
    file_ref: u64,
    volume_serial: u32,
) -> Option<FileEntry> {
    let is_dir = info.is_dir || usn_attributes & win_attr::DIRECTORY != 0;
    if !info.in_use || info.base_record != 0 || (!is_dir && info.data_size.is_none()) {
        return None;
    }
    let mut attrs = usn_attributes | info.attributes.unwrap_or(0);
    if is_dir {
        attrs |= win_attr::DIRECTORY;
    }
    let size = if is_dir { 0 } else { info.data_size.unwrap_or(0) };
    let mut e = entry_from_win32(
        path,
        attrs,
        info.reparse_tag,
        size,
        info.created.unwrap_or(0),
        info.accessed.unwrap_or(0),
        info.modified.unwrap_or(0),
    );
    if e.is_file() && !e.is_online_only() {
        e.allocated = Some(if info.resident { 0 } else { info.compressed.or(info.allocated).unwrap_or(size) });
    }
    e.file_id = Some(FileId { volume: u64::from(volume_serial), index: file_ref });
    e.links = Some(u32::from(info.link_count));
    Some(e)
}

/// Enumerate everything under `root` from the MFT.
pub fn fast_enumerate(root: &Path, sink: &mut dyn FnMut(FileEntry), cancel: &CancelToken) -> Result<()> {
    #[cfg(windows)]
    {
        imp::enumerate(root, sink, cancel)
    }
    #[cfg(not(windows))]
    {
        let _ = (root, sink, cancel);
        Err(Error::unsupported(FEATURE, "NTFS MFT enumeration is Windows only"))
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::{c_void, OsStr, OsString};
    use std::os::windows::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    use mj_core::{CancelToken, Error, FileEntry, Result};

    use super::super::metadata::{metadata, reparse, win_attr, win_identity};
    use super::super::ntfs::{apply_fixups, parse_file_record, NtfsFileInfo};
    use super::super::usn::{parse_enum_buffer, resolve_below, UsnRecord};
    use super::super::win::*;
    use super::{entry_from_mft, FEATURE};
    use crate::ffi::windows::{
        wide, CreateFileW, DeviceIoControl, OwnedHandle, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, OPEN_EXISTING,
    };

    /// Upper bound on entries below one root (memory guard).
    const MAX_ENTRIES: usize = 50_000_000;

    struct Volume {
        handle: OwnedHandle,
        serial: u32,
        record_size: usize,
    }

    fn io_err(path: &Path) -> Error {
        Error::io(path, last_error())
    }

    fn open_volume(root: &Path) -> Result<Volume> {
        let w = wide(root.as_os_str());
        let mut mount = [0u16; 1024];
        // SAFETY: `w` is NUL-terminated; `mount` is a writable buffer of the stated length.
        if unsafe { GetVolumePathNameW(w.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
            return Err(io_err(root));
        }
        let mut fs_name = [0u16; 64];
        let (mut serial, mut max_comp, mut flags) = (0u32, 0u32, 0u32);
        // SAFETY: `mount` is NUL-terminated (written by the OS); out buffers and pointers are valid.
        let ok = unsafe {
            GetVolumeInformationW(
                mount.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut serial,
                &mut max_comp,
                &mut flags,
                fs_name.as_mut_ptr(),
                fs_name.len() as u32,
            )
        };
        if ok == 0 {
            return Err(io_err(root));
        }
        let fs = from_wide_buf(&fs_name);
        if !fs.eq_ignore_ascii_case("NTFS") {
            return Err(Error::unsupported(FEATURE, format!("{fs} volume: the MFT index exists on NTFS only")));
        }
        let mut guid = [0u16; 128];
        // SAFETY: `mount` is NUL-terminated; `guid` is a writable buffer of the stated length.
        if unsafe { GetVolumeNameForVolumeMountPointW(mount.as_ptr(), guid.as_mut_ptr(), guid.len() as u32) } == 0 {
            return Err(io_err(root));
        }
        // "\\?\Volume{...}\" -> "\\?\Volume{...}" opens the volume, not its root directory.
        let mut name = from_wide_buf(&guid);
        if name.ends_with('\\') {
            name.pop();
        }
        let wn = wide(OsStr::new(&name));
        // SAFETY: `wn` is NUL-terminated; null security attributes and template are allowed.
        let h = unsafe {
            CreateFileW(
                wn.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        let handle = OwnedHandle::new(h).map_err(|e| {
            if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
                Error::unsupported(
                    FEATURE,
                    "reading the NTFS index needs administrator rights; the directory walk is used instead",
                )
            } else {
                Error::io(root, e)
            }
        })?;
        let mut vd = [0u64; 12];
        let mut ret = 0u32;
        // SAFETY: valid volume handle; `vd` is a writable 96-byte NTFS_VOLUME_DATA_BUFFER.
        let ok = unsafe {
            DeviceIoControl(
                handle.0,
                FSCTL_GET_NTFS_VOLUME_DATA,
                std::ptr::null_mut(),
                0,
                vd.as_mut_ptr() as *mut c_void,
                96,
                &mut ret,
                std::ptr::null_mut(),
            )
        };
        // BytesPerFileRecordSegment: u32 at offset 48 (low half of vd[6]).
        let record_size = if ok != 0 { (vd[6] & 0xFFFF_FFFF) as usize } else { 4096 };
        Ok(Volume { handle, serial, record_size: record_size.clamp(1024, 65_536) })
    }

    fn enum_records(vol: &Volume, root: &Path, cancel: &CancelToken) -> Result<Vec<UsnRecord>> {
        let mut input = MFT_ENUM_DATA_V0 { StartFileReferenceNumber: 0, LowUsn: 0, HighUsn: i64::MAX };
        let mut buf = vec![0u64; 128 * 1024]; // 1 MiB, 8-byte aligned
        let mut out = Vec::new();
        loop {
            cancel.checkpoint()?;
            let mut ret = 0u32;
            // SAFETY: valid volume handle; `input` is a MFT_ENUM_DATA_V0 of the stated size; `buf` is
            // writable for the stated byte length. Synchronous call (no OVERLAPPED).
            let ok = unsafe {
                DeviceIoControl(
                    vol.handle.0,
                    FSCTL_ENUM_USN_DATA,
                    &mut input as *mut _ as *mut c_void,
                    std::mem::size_of::<MFT_ENUM_DATA_V0>() as u32,
                    buf.as_mut_ptr() as *mut c_void,
                    (buf.len() * 8) as u32,
                    &mut ret,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let e = last_error();
                if e.raw_os_error() == Some(ERROR_HANDLE_EOF) {
                    return Ok(out);
                }
                return Err(Error::io(root, e));
            }
            // SAFETY: `buf` holds `ret` initialised bytes (ret <= its byte length); u8 view of u64s.
            let bytes =
                unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, (ret as usize).min(buf.len() * 8)) };
            let (next, recs) = parse_enum_buffer(bytes)?;
            if recs.is_empty() && next <= input.StartFileReferenceNumber {
                return Ok(out);
            }
            out.extend(recs);
            input.StartFileReferenceNumber = next;
        }
    }

    fn file_record(vol: &Volume, frn: u64, buf: &mut [u64]) -> Option<NtfsFileInfo> {
        // The input is the record segment number (low 48 bits); the sequence number is not part of it.
        let mut input = frn & 0x0000_FFFF_FFFF_FFFF;
        let mut ret = 0u32;
        // SAFETY: valid volume handle; `input` is the 8-byte NTFS_FILE_RECORD_INPUT_BUFFER; `buf`
        // is writable for the stated byte length.
        let ok = unsafe {
            DeviceIoControl(
                vol.handle.0,
                FSCTL_GET_NTFS_FILE_RECORD,
                &mut input as *mut u64 as *mut c_void,
                8,
                buf.as_mut_ptr() as *mut c_void,
                (buf.len() * 8) as u32,
                &mut ret,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || ret < 12 {
            return None;
        }
        // SAFETY: `buf` holds `ret` initialised bytes.
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, (ret as usize).min(buf.len() * 8)) };
        // NTFS_FILE_RECORD_OUTPUT_BUFFER: i64 reference, u32 length, record bytes.
        let got = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        // The call returns the nearest lower in-use record: the 48-bit index must match.
        if got & 0x0000_FFFF_FFFF_FFFF != frn & 0x0000_FFFF_FFFF_FFFF {
            return None;
        }
        let len = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
        let mut rec = bytes.get(12..12 + len)?.to_vec();
        apply_fixups(&mut rec).ok()?;
        parse_file_record(&rec).ok()
    }

    pub fn enumerate(root: &Path, sink: &mut dyn FnMut(FileEntry), cancel: &CancelToken) -> Result<()> {
        let vol = open_volume(root)?;
        let (root_id, _) = win_identity(root)
            .ok_or_else(|| Error::unsupported(FEATURE, "the scan location could not be identified on its volume"))?;
        let records = enum_records(&vol, root, cancel)?;
        let mut rbuf = vec![0u64; (12 + vol.record_size).div_ceil(8) + 1];
        // Reparse directories are entered only when their tag is known not to be a name surrogate
        // (cloud-files folders are; junctions, symlinks and mount points are not).
        let descend = |i: usize| -> bool {
            let r = &records[i];
            if r.attributes & win_attr::REPARSE_POINT == 0 {
                return true;
            }
            let mut b = vec![0u64; (12 + vol.record_size).div_ceil(8) + 1];
            file_record(&vol, r.file_ref as u64, &mut b)
                .and_then(|info| info.reparse_tag)
                .is_some_and(|t| !reparse::is_name_surrogate(t))
        };
        let resolved = resolve_below(&records, u128::from(root_id.index), &descend, MAX_ENTRIES);
        for (n, item) in resolved.iter().enumerate() {
            if n % 4096 == 0 {
                cancel.checkpoint()?;
            }
            let mut path: PathBuf = root.to_path_buf();
            for &c in &item.components {
                path.push(OsString::from_wide(&records[c].name));
            }
            let r = &records[item.record];
            let frn = r.file_ref as u64;
            let entry = file_record(&vol, frn, &mut rbuf)
                .and_then(|info| entry_from_mft(&path, r.attributes, &info, frn, vol.serial));
            let entry = match entry {
                Some(e) => e,
                None => match metadata(&path) {
                    Ok(e) => e,
                    // Deleted since the enumeration: it no longer exists, nothing to report.
                    Err(Error::NotFound(_)) => continue,
                    Err(e) => return Err(e),
                },
            };
            sink(entry);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::ntfs::{apply_fixups, parse_file_record, tests::Rec};
    use super::*;
    use mj_core::{Attributes, EntryKind, Timestamp};

    fn info(r: Rec) -> NtfsFileInfo {
        let mut b = r.build();
        apply_fixups(&mut b).unwrap();
        parse_file_record(&b).unwrap()
    }

    #[test]
    fn mft_entries_from_records() {
        let p = Path::new(r"C:\Users\me\a.txt");
        let ft = 133_000_000_000_000_000u64;
        let f = entry_from_mft(
            p,
            0x20,
            &info(Rec::new(0x1).std_info(ft, ft, ft, 0x20).nonresident_data(8192, 5000, None)),
            0x0002_0000_0000_0040,
            0xABCD,
        )
        .unwrap();
        assert_eq!((f.kind, f.size, f.allocated), (EntryKind::File, 5000, Some(8192)));
        assert_eq!(f.file_id, Some(FileId { volume: 0xABCD, index: 0x0002_0000_0000_0040 }));
        assert_eq!(f.modified, Some(Timestamp::from_filetime(ft)));
        assert_eq!(f.links, Some(1));
        // Resident: no clusters. Compressed: on-disk size.
        let r = entry_from_mft(p, 0x20, &info(Rec::new(0x1).resident_data(100)), 1, 1).unwrap();
        assert_eq!((r.size, r.allocated), (100, Some(0)));
        let c = entry_from_mft(p, 0x820, &info(Rec::new(0x1).nonresident_data(1 << 20, 900_000, Some(4096))), 1, 1)
            .unwrap();
        assert_eq!(c.allocated, Some(4096));
        assert!(c.attrs.contains(Attributes::COMPRESSED));
        // Directory from the record header; junction is a link; cloud folder stays a directory.
        let d = entry_from_mft(p, 0, &info(Rec::new(0x3)), 1, 1).unwrap();
        assert_eq!(d.kind, EntryKind::Dir);
        let j =
            entry_from_mft(p, 0x410, &info(Rec::new(0x3).std_info(0, 0, 0, 0x400).reparse(0xA000_0003)), 1, 1).unwrap();
        assert_eq!(j.kind, EntryKind::Link);
        let cl =
            entry_from_mft(p, 0x410, &info(Rec::new(0x3).std_info(0, 0, 0, 0x400).reparse(0x9000_101A)), 1, 1).unwrap();
        assert_eq!(cl.kind, EntryKind::Dir);
        // Online-only placeholder: excluded from totals.
        let o = entry_from_mft(
            p,
            0x420,
            &info(Rec::new(0x1).std_info(0, 0, 0, 0x40_0420).reparse(0x9000_001A).nonresident_data(0, 7000, None)),
            1,
            1,
        )
        .unwrap();
        assert!(o.is_online_only());
        assert_eq!(o.reclaimable(), 0);
    }

    #[test]
    fn incomplete_records_ask_for_a_fallback() {
        let p = Path::new(r"C:\x");
        // Attribute list without $DATA in the base record, extension record, unused record.
        assert!(entry_from_mft(p, 0x20, &info(Rec::new(0x1).attr_list()), 1, 1).is_none());
        assert!(entry_from_mft(p, 0x20, &info(Rec::new(0x1).base(7).resident_data(5)), 1, 1).is_none());
        assert!(entry_from_mft(p, 0x20, &info(Rec::new(0x0).resident_data(5)), 1, 1).is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_off_windows() {
        let err = fast_enumerate(Path::new("/"), &mut |_| {}, &CancelToken::new()).unwrap_err();
        assert_eq!(err.code(), "unsupported");
    }
}
