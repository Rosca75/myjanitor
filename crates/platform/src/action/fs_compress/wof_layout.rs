//! Windows Overlay Filter (WOF) constants and buffer layouts, as pure code so
//! they compile and are unit-tested on every platform. The FFI that sends
//! these buffers lives in `wof.rs` (Windows only).
//!
//! Sources: `winioctl.h` (FSCTL codes, `CTL_CODE`), `wofapi.h` / `ntifs.h`
//! (`WOF_EXTERNAL_INFO`, `FILE_PROVIDER_EXTERNAL_INFO_V1`, provider and
//! algorithm values), `winerror.h` (error codes).

use mj_core::FsCompressAlgo;

/// `CTL_CODE(DeviceType, Function, Method, Access)` from `winioctl.h`.
pub const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

/// `FILE_DEVICE_FILE_SYSTEM` (`winioctl.h`).
pub const FILE_DEVICE_FILE_SYSTEM: u32 = 0x0000_0009;
/// `METHOD_BUFFERED` (`winioctl.h`).
pub const METHOD_BUFFERED: u32 = 0;
/// `FILE_ANY_ACCESS` / `FILE_SPECIAL_ACCESS` (`winioctl.h`), both 0.
pub const FILE_SPECIAL_ACCESS: u32 = 0;

/// `FSCTL_SET_EXTERNAL_BACKING` = `CTL_CODE(FILE_DEVICE_FILE_SYSTEM, 195, METHOD_BUFFERED, FILE_SPECIAL_ACCESS)` = 0x9030C.
pub const FSCTL_SET_EXTERNAL_BACKING: u32 =
    ctl_code(FILE_DEVICE_FILE_SYSTEM, 195, METHOD_BUFFERED, FILE_SPECIAL_ACCESS);
/// `FSCTL_GET_EXTERNAL_BACKING` = function 196 = 0x90310.
pub const FSCTL_GET_EXTERNAL_BACKING: u32 =
    ctl_code(FILE_DEVICE_FILE_SYSTEM, 196, METHOD_BUFFERED, FILE_SPECIAL_ACCESS);
/// `FSCTL_DELETE_EXTERNAL_BACKING` = function 197 = 0x90314.
pub const FSCTL_DELETE_EXTERNAL_BACKING: u32 =
    ctl_code(FILE_DEVICE_FILE_SYSTEM, 197, METHOD_BUFFERED, FILE_SPECIAL_ACCESS);

/// `WOF_CURRENT_VERSION` (`wofapi.h`).
pub const WOF_CURRENT_VERSION: u32 = 1;
/// `WOF_PROVIDER_WIM` (`wofapi.h`): WIMBoot-backed file, never created by MyJanitor.
pub const WOF_PROVIDER_WIM: u32 = 1;
/// `WOF_PROVIDER_FILE` (`wofapi.h`): per-file compression.
pub const WOF_PROVIDER_FILE: u32 = 2;
/// `FILE_PROVIDER_CURRENT_VERSION` (`wofapi.h`).
pub const FILE_PROVIDER_CURRENT_VERSION: u32 = 1;
/// `FILE_PROVIDER_COMPRESSION_XPRESS4K` (`wofapi.h`).
pub const FILE_PROVIDER_COMPRESSION_XPRESS4K: u32 = 0;
/// `FILE_PROVIDER_COMPRESSION_LZX` (`wofapi.h`).
pub const FILE_PROVIDER_COMPRESSION_LZX: u32 = 1;
/// `FILE_PROVIDER_COMPRESSION_XPRESS8K` (`wofapi.h`).
pub const FILE_PROVIDER_COMPRESSION_XPRESS8K: u32 = 2;
/// `FILE_PROVIDER_COMPRESSION_XPRESS16K` (`wofapi.h`).
pub const FILE_PROVIDER_COMPRESSION_XPRESS16K: u32 = 3;

/// `ERROR_COMPRESSION_NOT_BENEFICIAL` (`winerror.h`): Windows found the file incompressible.
pub const ERROR_COMPRESSION_NOT_BENEFICIAL: u32 = 344;
/// `ERROR_OBJECT_NOT_EXTERNALLY_BACKED` (`winerror.h`): the file is not WOF-compressed.
pub const ERROR_OBJECT_NOT_EXTERNALLY_BACKED: u32 = 342;
/// `ERROR_INVALID_FUNCTION` (`winerror.h`): volume or filter does not support the request.
pub const ERROR_INVALID_FUNCTION: u32 = 1;
/// `ERROR_NOT_SUPPORTED` (`winerror.h`).
pub const ERROR_NOT_SUPPORTED: u32 = 50;

/// Size of `WOF_EXTERNAL_INFO` (two `ULONG`s).
pub const WOF_EXTERNAL_INFO_SIZE: usize = 8;
/// Size of `FILE_PROVIDER_EXTERNAL_INFO_V1` (three `ULONG`s).
pub const FILE_PROVIDER_EXTERNAL_INFO_V1_SIZE: usize = 12;
/// Input of `FSCTL_SET_EXTERNAL_BACKING` / output of `FSCTL_GET_EXTERNAL_BACKING`.
pub const BACKING_BUFFER_SIZE: usize = WOF_EXTERNAL_INFO_SIZE + FILE_PROVIDER_EXTERNAL_INFO_V1_SIZE;

/// WOF algorithm number for an algorithm, `None` for non-WOF algorithms.
pub fn wof_algorithm(algo: FsCompressAlgo) -> Option<u32> {
    match algo {
        FsCompressAlgo::Xpress4k => Some(FILE_PROVIDER_COMPRESSION_XPRESS4K),
        FsCompressAlgo::Xpress8k => Some(FILE_PROVIDER_COMPRESSION_XPRESS8K),
        FsCompressAlgo::Xpress16k => Some(FILE_PROVIDER_COMPRESSION_XPRESS16K),
        FsCompressAlgo::Lzx => Some(FILE_PROVIDER_COMPRESSION_LZX),
        FsCompressAlgo::BtrfsZstd => None,
    }
}

/// The `FSCTL_SET_EXTERNAL_BACKING` input: `WOF_EXTERNAL_INFO { Version, Provider }`
/// immediately followed by `FILE_PROVIDER_EXTERNAL_INFO_V1 { Version, Algorithm, Flags }`,
/// every field a little-endian `ULONG`.
pub fn set_backing_input(algo: FsCompressAlgo) -> Option<[u8; BACKING_BUFFER_SIZE]> {
    let a = wof_algorithm(algo)?;
    let mut b = [0u8; BACKING_BUFFER_SIZE];
    let fields = [WOF_CURRENT_VERSION, WOF_PROVIDER_FILE, FILE_PROVIDER_CURRENT_VERSION, a, 0];
    for (i, f) in fields.iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    Some(b)
}

/// Parse the `FSCTL_GET_EXTERNAL_BACKING` output. `None` for a provider other than
/// `WOF_PROVIDER_FILE` (WIMBoot) or an unknown algorithm.
pub fn parse_backing_output(b: &[u8]) -> Option<FsCompressAlgo> {
    let u = |i: usize| b.get(i * 4..i * 4 + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    if u(1)? != WOF_PROVIDER_FILE {
        return None;
    }
    match u(3)? {
        FILE_PROVIDER_COMPRESSION_XPRESS4K => Some(FsCompressAlgo::Xpress4k),
        FILE_PROVIDER_COMPRESSION_XPRESS8K => Some(FsCompressAlgo::Xpress8k),
        FILE_PROVIDER_COMPRESSION_XPRESS16K => Some(FsCompressAlgo::Xpress16k),
        FILE_PROVIDER_COMPRESSION_LZX => Some(FsCompressAlgo::Lzx),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsctl_codes_match_winioctl() {
        assert_eq!(FSCTL_SET_EXTERNAL_BACKING, 0x0009_030C);
        assert_eq!(FSCTL_GET_EXTERNAL_BACKING, 0x0009_0310);
        assert_eq!(FSCTL_DELETE_EXTERNAL_BACKING, 0x0009_0314);
    }

    #[test]
    fn set_input_layout() {
        let b = set_backing_input(FsCompressAlgo::Xpress8k).unwrap();
        assert_eq!(b, [1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(set_backing_input(FsCompressAlgo::Lzx).unwrap()[12], 1);
        assert!(set_backing_input(FsCompressAlgo::BtrfsZstd).is_none());
    }

    #[test]
    fn output_round_trips() {
        for a in [FsCompressAlgo::Xpress4k, FsCompressAlgo::Xpress8k, FsCompressAlgo::Xpress16k, FsCompressAlgo::Lzx] {
            assert_eq!(parse_backing_output(&set_backing_input(a).unwrap()), Some(a));
        }
        let mut wim = set_backing_input(FsCompressAlgo::Lzx).unwrap();
        wim[4] = WOF_PROVIDER_WIM as u8;
        assert_eq!(parse_backing_output(&wim), None);
        assert_eq!(parse_backing_output(&[0; 6]), None);
    }
}
