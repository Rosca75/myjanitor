//! Win32 declarations used only by the scan side (W2-A). Common ones
//! (`CreateFileW`, `DeviceIoControl`, `wide_path`, handles) live in the frozen
//! `ffi::windows`. Struct layouts follow the Windows SDK headers
//! (`minwinbase.h`, `fileapi.h`, `winioctl.h`); sizes are asserted at compile time.
#![allow(non_camel_case_types, non_snake_case, clippy::upper_case_acronyms, dead_code)]

use std::ffi::{c_void, OsString};
use std::io;
use std::os::windows::ffi::OsStringExt;

use crate::ffi::windows::{BOOL, DWORD, HANDLE, LPCWSTR};

/// `FILETIME`: 100 ns ticks since 1601 split in two halves.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FILETIME {
    pub dwLowDateTime: DWORD,
    pub dwHighDateTime: DWORD,
}

impl FILETIME {
    /// The 64-bit tick count.
    pub fn ticks(self) -> u64 {
        (u64::from(self.dwHighDateTime) << 32) | u64::from(self.dwLowDateTime)
    }
}

/// `WIN32_FILE_ATTRIBUTE_DATA` (`GetFileExInfoStandard`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WIN32_FILE_ATTRIBUTE_DATA {
    pub dwFileAttributes: DWORD,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub nFileSizeHigh: DWORD,
    pub nFileSizeLow: DWORD,
}

/// `BY_HANDLE_FILE_INFORMATION`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BY_HANDLE_FILE_INFORMATION {
    pub dwFileAttributes: DWORD,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub dwVolumeSerialNumber: DWORD,
    pub nFileSizeHigh: DWORD,
    pub nFileSizeLow: DWORD,
    pub nNumberOfLinks: DWORD,
    pub nFileIndexHigh: DWORD,
    pub nFileIndexLow: DWORD,
}

/// `WIN32_FIND_DATAW` (MAX_PATH = 260).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WIN32_FIND_DATAW {
    pub dwFileAttributes: DWORD,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub nFileSizeHigh: DWORD,
    pub nFileSizeLow: DWORD,
    /// Reparse tag when `FILE_ATTRIBUTE_REPARSE_POINT` is set.
    pub dwReserved0: DWORD,
    pub dwReserved1: DWORD,
    pub cFileName: [u16; 260],
    pub cAlternateFileName: [u16; 14],
}

impl WIN32_FIND_DATAW {
    /// Zeroed buffer.
    pub fn zeroed() -> Self {
        WIN32_FIND_DATAW {
            dwFileAttributes: 0,
            ftCreationTime: FILETIME::default(),
            ftLastAccessTime: FILETIME::default(),
            ftLastWriteTime: FILETIME::default(),
            nFileSizeHigh: 0,
            nFileSizeLow: 0,
            dwReserved0: 0,
            dwReserved1: 0,
            cFileName: [0; 260],
            cAlternateFileName: [0; 14],
        }
    }

    /// `cFileName` up to its NUL.
    pub fn name(&self) -> OsString {
        let len = self.cFileName.iter().position(|&c| c == 0).unwrap_or(self.cFileName.len());
        OsString::from_wide(&self.cFileName[..len])
    }
}

const _: () = assert!(std::mem::size_of::<WIN32_FILE_ATTRIBUTE_DATA>() == 36);
const _: () = assert!(std::mem::size_of::<BY_HANDLE_FILE_INFORMATION>() == 52);
const _: () = assert!(std::mem::size_of::<WIN32_FIND_DATAW>() == 592);

pub const GET_FILE_EX_INFO_STANDARD: i32 = 0;
pub const FIND_EX_INFO_BASIC: i32 = 1;
pub const FIND_EX_SEARCH_NAME_MATCH: i32 = 0;
pub const FIND_FIRST_EX_LARGE_FETCH: DWORD = 2;
pub const FILE_READ_ATTRIBUTES: DWORD = 0x80;
pub const INVALID_FILE_SIZE: DWORD = 0xFFFF_FFFF;
pub const INVALID_FILE_ATTRIBUTES: DWORD = 0xFFFF_FFFF;

pub const ERROR_FILE_NOT_FOUND: i32 = 2;
pub const ERROR_ACCESS_DENIED: i32 = 5;
pub const ERROR_NO_MORE_FILES: i32 = 18;
pub const ERROR_NOT_READY: i32 = 21;
pub const ERROR_HANDLE_EOF: i32 = 38;
pub const ERROR_INSUFFICIENT_BUFFER: i32 = 122;
pub const ERROR_MORE_DATA: i32 = 234;

pub const DRIVE_REMOVABLE: u32 = 2;
pub const DRIVE_FIXED: u32 = 3;

pub const FILE_SUPPORTS_HARD_LINKS: DWORD = 0x0040_0000;
pub const FILE_SUPPORTS_BLOCK_REFCOUNTING: DWORD = 0x0800_0000;
pub const FILE_READ_ONLY_VOLUME: DWORD = 0x0008_0000;

/// `FSCTL_ENUM_USN_DATA` = CTL_CODE(FILE_DEVICE_FILE_SYSTEM, 44, METHOD_NEITHER, FILE_ANY_ACCESS).
pub const FSCTL_ENUM_USN_DATA: DWORD = 0x0009_00B3;
/// `FSCTL_GET_NTFS_FILE_RECORD` = CTL_CODE(FILE_DEVICE_FILE_SYSTEM, 26, METHOD_BUFFERED, FILE_ANY_ACCESS).
pub const FSCTL_GET_NTFS_FILE_RECORD: DWORD = 0x0009_0068;
/// `FSCTL_GET_NTFS_VOLUME_DATA` = CTL_CODE(FILE_DEVICE_FILE_SYSTEM, 25, METHOD_BUFFERED, FILE_ANY_ACCESS).
pub const FSCTL_GET_NTFS_VOLUME_DATA: DWORD = 0x0009_0064;

/// `MFT_ENUM_DATA_V0`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MFT_ENUM_DATA_V0 {
    pub StartFileReferenceNumber: u64,
    pub LowUsn: i64,
    pub HighUsn: i64,
}
const _: () = assert!(std::mem::size_of::<MFT_ENUM_DATA_V0>() == 24);

#[link(name = "kernel32")]
extern "system" {
    pub fn GetFileAttributesExW(lpFileName: LPCWSTR, fInfoLevelId: i32, lpFileInformation: *mut c_void) -> BOOL;
    pub fn GetFileInformationByHandle(hFile: HANDLE, lpFileInformation: *mut BY_HANDLE_FILE_INFORMATION) -> BOOL;
    pub fn GetCompressedFileSizeW(lpFileName: LPCWSTR, lpFileSizeHigh: *mut DWORD) -> DWORD;
    pub fn FindFirstFileExW(
        lpFileName: LPCWSTR,
        fInfoLevelId: i32,
        lpFindFileData: *mut WIN32_FIND_DATAW,
        fSearchOp: i32,
        lpSearchFilter: *mut c_void,
        dwAdditionalFlags: DWORD,
    ) -> HANDLE;
    pub fn FindNextFileW(hFindFile: HANDLE, lpFindFileData: *mut WIN32_FIND_DATAW) -> BOOL;
    pub fn FindClose(hFindFile: HANDLE) -> BOOL;
    pub fn GetVolumePathNameW(lpszFileName: LPCWSTR, lpszVolumePathName: *mut u16, cchBufferLength: DWORD) -> BOOL;
    pub fn GetVolumeNameForVolumeMountPointW(
        lpszVolumeMountPoint: LPCWSTR,
        lpszVolumeName: *mut u16,
        cchBufferLength: DWORD,
    ) -> BOOL;
    pub fn GetVolumeInformationW(
        lpRootPathName: LPCWSTR,
        lpVolumeNameBuffer: *mut u16,
        nVolumeNameSize: DWORD,
        lpVolumeSerialNumber: *mut DWORD,
        lpMaximumComponentLength: *mut DWORD,
        lpFileSystemFlags: *mut DWORD,
        lpFileSystemNameBuffer: *mut u16,
        nFileSystemNameSize: DWORD,
    ) -> BOOL;
    pub fn GetDiskFreeSpaceExW(
        lpDirectoryName: LPCWSTR,
        lpFreeBytesAvailableToCaller: *mut u64,
        lpTotalNumberOfBytes: *mut u64,
        lpTotalNumberOfFreeBytes: *mut u64,
    ) -> BOOL;
    pub fn GetDiskFreeSpaceW(
        lpRootPathName: LPCWSTR,
        lpSectorsPerCluster: *mut DWORD,
        lpBytesPerSector: *mut DWORD,
        lpNumberOfFreeClusters: *mut DWORD,
        lpTotalNumberOfClusters: *mut DWORD,
    ) -> BOOL;
    pub fn GetLogicalDriveStringsW(nBufferLength: DWORD, lpBuffer: *mut u16) -> DWORD;
    pub fn GetDriveTypeW(lpRootPathName: LPCWSTR) -> u32;
    pub fn SetErrorMode(uMode: u32) -> u32;
}

/// The last Win32 error as an `io::Error`.
pub fn last_error() -> io::Error {
    io::Error::last_os_error()
}

/// UTF-16 buffer up to its first NUL, as a String (lossy).
pub fn from_wide_buf(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// A `FindFirstFileExW` handle closed on drop.
pub struct FindHandle(pub HANDLE);

impl Drop for FindHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful FindFirstFileExW and is closed once.
        unsafe {
            FindClose(self.0);
        }
    }
}
