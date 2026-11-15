//! Common Linux FFI: the few libc symbols std already links against.
#![allow(non_camel_case_types)]

use std::ffi::CString;
use std::io;
use std::os::raw::{c_char, c_int, c_ulong, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub type size_t = usize;
pub type ssize_t = isize;

extern "C" {
    pub fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    pub fn getuid() -> u32;
    pub fn geteuid() -> u32;
    pub fn getxattr(path: *const c_char, name: *const c_char, value: *mut c_void, size: size_t) -> ssize_t;
    pub fn lgetxattr(path: *const c_char, name: *const c_char, value: *mut c_void, size: size_t) -> ssize_t;
}

/// NUL-terminated copy of a path for C calls.
pub fn cpath(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

/// The calling thread's last OS error.
pub fn last_error() -> io::Error {
    io::Error::last_os_error()
}

/// Real user id of the process.
pub fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { getuid() }
}

/// Effective user id of the process.
pub fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { geteuid() }
}
