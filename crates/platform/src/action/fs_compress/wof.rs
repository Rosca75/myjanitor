//! Windows WOF compression. Owner: W3-B.
//!
//! Per-file compression through the Windows Overlay Filter, as `compact.exe
//! /c /exe:<algo>` does: `FSCTL_SET_EXTERNAL_BACKING` with
//! `WOF_PROVIDER_FILE`, revert with `FSCTL_DELETE_EXTERNAL_BACKING`, query
//! with `FSCTL_GET_EXTERNAL_BACKING`. File content is unchanged; Windows
//! decompresses transparently on read and drops the compression when the
//! file is written. Buffer layouts are in `wof_layout.rs`.
//!
//! Compile-checked only from the Linux session (no Windows runner here).
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::io;
use std::path::Path;

use mj_core::{Error, FsCompressAlgo, Result};

use super::wof_layout::*;
use crate::ffi::windows::{
    wide_path, CreateFileW, DeviceIoControl, GetLastError, OwnedHandle, BOOL, DWORD, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE, LPCWSTR, OPEN_EXISTING,
};

/// `INVALID_FILE_SIZE` (`fileapi.h`).
const INVALID_FILE_SIZE: DWORD = 0xFFFF_FFFF;
/// `NO_ERROR` (`winerror.h`).
const NO_ERROR: DWORD = 0;
/// `MAX_PATH` (`minwindef.h`); volume names are short.
const MAX_PATH: usize = 260;

#[link(name = "kernel32")]
extern "system" {
    fn GetCompressedFileSizeW(lpFileName: LPCWSTR, lpFileSizeHigh: *mut DWORD) -> DWORD;
    fn GetVolumePathNameW(lpszFileName: LPCWSTR, lpszVolumePathName: *mut u16, cchBufferLength: DWORD) -> BOOL;
    fn GetVolumeInformationW(
        lpRootPathName: LPCWSTR,
        lpVolumeNameBuffer: *mut u16,
        nVolumeNameSize: DWORD,
        lpVolumeSerialNumber: *mut DWORD,
        lpMaximumComponentLength: *mut DWORD,
        lpFileSystemFlags: *mut DWORD,
        lpFileSystemNameBuffer: *mut u16,
        nFileSystemNameSize: DWORD,
    ) -> BOOL;
}

/// Refuse links: WOF acts on the file data, and destructive code never follows links.
fn ensure_plain_file(path: &Path) -> Result<()> {
    let md = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if md.file_type().is_symlink() {
        return Err(Error::LinkNotFollowed(path.to_path_buf()));
    }
    if !md.is_file() {
        return Err(Error::InvalidInput(format!("{} is not a regular file", path.display())));
    }
    Ok(())
}

/// Open a regular file for reading and writing (no reparse-point flag: a WOF
/// file is itself a reparse point and must be opened through the filter).
fn open(path: &Path, write: bool) -> Result<OwnedHandle> {
    ensure_plain_file(path)?;
    let w = wide_path(path);
    let access = if write { GENERIC_READ | GENERIC_WRITE } else { GENERIC_READ };
    // SAFETY: `w` is a NUL-terminated wide string that outlives the call; the
    // other pointers are null, which CreateFileW accepts.
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    OwnedHandle::new(h).map_err(|e| Error::io(path, e))
}

fn ioctl(
    h: &OwnedHandle,
    code: DWORD,
    input: Option<&mut [u8]>,
    output: Option<&mut [u8]>,
) -> std::result::Result<(), DWORD> {
    let (ip, il) = match input {
        Some(b) => (b.as_mut_ptr() as *mut c_void, b.len() as DWORD),
        None => (std::ptr::null_mut(), 0),
    };
    let (op, ol) = match output {
        Some(b) => (b.as_mut_ptr() as *mut c_void, b.len() as DWORD),
        None => (std::ptr::null_mut(), 0),
    };
    let mut returned: DWORD = 0;
    // SAFETY: the handle is open; the buffers are valid for the given lengths
    // for the duration of this synchronous call (no OVERLAPPED).
    let ok = unsafe { DeviceIoControl(h.0, code, ip, il, op, ol, &mut returned, std::ptr::null_mut()) };
    if ok != 0 {
        Ok(())
    } else {
        // SAFETY: no preconditions.
        Err(unsafe { GetLastError() })
    }
}

fn os_error(path: &Path, code: DWORD) -> Error {
    match code {
        ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED => Error::unsupported(
            "Transparent compression",
            "Needs an NTFS volume with the Windows Overlay Filter (Windows 10 or later)",
        ),
        ERROR_COMPRESSION_NOT_BENEFICIAL => {
            Error::unsupported("Transparent compression", "Windows found this file not compressible")
        }
        _ => Error::io(path, io::Error::from_raw_os_error(code as i32)),
    }
}

pub fn compress(path: &Path, algo: FsCompressAlgo) -> Result<()> {
    let Some(mut input) = set_backing_input(algo) else {
        return Err(Error::unsupported("Transparent compression", "Btrfs compression is Linux only"));
    };
    let h = open(path, true)?;
    ioctl(&h, FSCTL_SET_EXTERNAL_BACKING, Some(&mut input), None).map_err(|c| os_error(path, c))
}

pub fn decompress(path: &Path) -> Result<()> {
    let h = open(path, true)?;
    match ioctl(&h, FSCTL_DELETE_EXTERNAL_BACKING, None, None) {
        Ok(()) | Err(ERROR_OBJECT_NOT_EXTERNALLY_BACKED) => Ok(()),
        Err(c) => Err(os_error(path, c)),
    }
}

pub fn state(path: &Path) -> Result<Option<FsCompressAlgo>> {
    let h = open(path, false)?;
    let mut out = [0u8; BACKING_BUFFER_SIZE];
    match ioctl(&h, FSCTL_GET_EXTERNAL_BACKING, None, Some(&mut out)) {
        Ok(()) => Ok(parse_backing_output(&out)),
        Err(ERROR_OBJECT_NOT_EXTERNALLY_BACKED) => Ok(None),
        Err(c) => Err(os_error(path, c)),
    }
}

/// Bytes the file occupies on disk (`GetCompressedFileSizeW`).
pub fn on_disk_size(path: &Path) -> Result<u64> {
    let w = wide_path(path);
    let mut high: DWORD = 0;
    // SAFETY: `w` is NUL-terminated; `high` is a valid out pointer.
    let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
    if low == INVALID_FILE_SIZE {
        // SAFETY: no preconditions.
        let e = unsafe { GetLastError() };
        if e != NO_ERROR {
            return Err(Error::io(path, io::Error::from_raw_os_error(e as i32)));
        }
    }
    Ok(((high as u64) << 32) | low as u64)
}

/// `Ok` when `path` is on NTFS, else the one-line reason.
pub fn support(path: &Path) -> std::result::Result<(), String> {
    let w = wide_path(path);
    let mut vol = [0u16; MAX_PATH + 1];
    // SAFETY: `w` is NUL-terminated; `vol` holds MAX_PATH + 1 code units.
    if unsafe { GetVolumePathNameW(w.as_ptr(), vol.as_mut_ptr(), vol.len() as DWORD) } == 0 {
        return Err("Could not determine the volume of this file".into());
    }
    let mut fs = [0u16; MAX_PATH + 1];
    // SAFETY: `vol` is NUL-terminated by the previous call; every out pointer is
    // either null (not requested) or a buffer of the stated length.
    let ok = unsafe {
        GetVolumeInformationW(
            vol.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as DWORD,
        )
    };
    if ok == 0 {
        return Err("Could not read the volume's file system".into());
    }
    let len = fs.iter().position(|c| *c == 0).unwrap_or(fs.len());
    if String::from_utf16_lossy(&fs[..len]).eq_ignore_ascii_case("NTFS") {
        Ok(())
    } else {
        Err("Needs an NTFS volume".into())
    }
}
