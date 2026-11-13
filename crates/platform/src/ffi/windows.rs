//! Common Win32 FFI (kernel32): handles, files, errors, wide strings.
#![allow(non_camel_case_types, non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::{c_void, OsStr};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

pub type HANDLE = *mut c_void;
pub type BOOL = i32;
pub type DWORD = u32;
pub type LPCWSTR = *const u16;

pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
pub const GENERIC_READ: DWORD = 0x8000_0000;
pub const GENERIC_WRITE: DWORD = 0x4000_0000;
pub const FILE_SHARE_READ: DWORD = 0x1;
pub const FILE_SHARE_WRITE: DWORD = 0x2;
pub const FILE_SHARE_DELETE: DWORD = 0x4;
pub const OPEN_EXISTING: DWORD = 3;
pub const FILE_FLAG_BACKUP_SEMANTICS: DWORD = 0x0200_0000;
pub const FILE_FLAG_OPEN_REPARSE_POINT: DWORD = 0x0020_0000;

#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: DWORD,
    pub lpSecurityDescriptor: *mut c_void,
    pub bInheritHandle: BOOL,
}

#[repr(C)]
pub struct OVERLAPPED {
    pub Internal: usize,
    pub InternalHigh: usize,
    pub Offset: DWORD,
    pub OffsetHigh: DWORD,
    pub hEvent: HANDLE,
}

#[link(name = "kernel32")]
extern "system" {
    pub fn CreateFileW(
        lpFileName: LPCWSTR,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: *mut SECURITY_ATTRIBUTES,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    pub fn CloseHandle(hObject: HANDLE) -> BOOL;
    pub fn GetLastError() -> DWORD;
    pub fn DeviceIoControl(
        hDevice: HANDLE,
        dwIoControlCode: DWORD,
        lpInBuffer: *mut c_void,
        nInBufferSize: DWORD,
        lpOutBuffer: *mut c_void,
        nOutBufferSize: DWORD,
        lpBytesReturned: *mut DWORD,
        lpOverlapped: *mut OVERLAPPED,
    ) -> BOOL;
    pub fn GetFileAttributesW(lpFileName: LPCWSTR) -> DWORD;
}

/// NUL-terminated UTF-16 for a string.
pub fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// NUL-terminated UTF-16 for a path, with the `\\?\` long-path prefix added to
/// absolute drive paths (`C:\...` -> `\\?\C:\...`, `\\server\share` -> `\\?\UNC\server\share`).
pub fn wide_path(path: &Path) -> Vec<u16> {
    let s = path.as_os_str();
    let raw: Vec<u16> = s.encode_wide().collect();
    let bs = b'\\' as u16;
    let mut out: Vec<u16> = Vec::with_capacity(raw.len() + 9);
    let has_prefix = raw.len() >= 4 && raw[0] == bs && raw[1] == bs && (raw[2] == b'?' as u16 || raw[2] == b'.' as u16);
    if !has_prefix {
        if raw.len() >= 3 && raw[1] == b':' as u16 && raw[2] == bs {
            out.extend("\\\\?\\".encode_utf16());
        } else if raw.len() >= 2 && raw[0] == bs && raw[1] == bs {
            out.extend("\\\\?\\UNC".encode_utf16());
            out.extend_from_slice(&raw[1..]);
            out.push(0);
            return out;
        }
    }
    out.extend_from_slice(&raw);
    out.push(0);
    out
}

/// A handle closed on drop.
pub struct OwnedHandle(pub HANDLE);

impl OwnedHandle {
    /// Wrap a handle returned by `CreateFileW`; fails on `INVALID_HANDLE_VALUE`.
    pub fn new(h: HANDLE) -> io::Result<OwnedHandle> {
        if h == INVALID_HANDLE_VALUE || h.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(OwnedHandle(h))
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle is valid (checked in `new`) and owned exclusively by self.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Open a file or directory handle without following reparse points.
pub fn open_no_follow(path: &Path, access: DWORD) -> io::Result<OwnedHandle> {
    let w = wide_path(path);
    // SAFETY: `w` is a valid NUL-terminated wide string that outlives the call.
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    OwnedHandle::new(h)
}
