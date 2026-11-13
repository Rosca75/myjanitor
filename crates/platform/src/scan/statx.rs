//! Linux `statx(2)` through a hand-written `syscall` FFI (no `libc` crate),
//! with a `std::fs` fallback. Owner: W2-A.
//!
//! Why `syscall(SYS_statx, ...)` and not glibc's `statx` wrapper: the wrapper
//! only exists from glibc 2.28, and linking against a missing symbol would
//! make the binary fail to start on older systems. `syscall` exists in every
//! glibc. When the kernel lacks `statx` (`ENOSYS`, kernels before 4.11) or a
//! seccomp filter refuses it (`EPERM`), the module remembers that and every
//! later call goes straight to `std::fs::symlink_metadata`.
//!
//! The kernel fills a 256-byte `struct statx`. Instead of a `#[repr(C)]`
//! mirror, the buffer is decoded by [`parse_statx`], a pure function over
//! bytes, so the layout is unit-tested with hand-built buffers.
//!
//! `struct statx` layout (include/uapi/linux/stat.h), little-endian offsets:
//!
//! ```text
//!   0 u32 stx_mask          4 u32 stx_blksize       8 u64 stx_attributes
//!  16 u32 stx_nlink        20 u32 stx_uid          24 u32 stx_gid
//!  28 u16 stx_mode         30 u16 spare            32 u64 stx_ino
//!  40 u64 stx_size         48 u64 stx_blocks       56 u64 stx_attributes_mask
//!  64 ts  stx_atime        80 ts  stx_btime        96 ts  stx_ctime
//! 112 ts  stx_mtime       128 u32 rdev_major      132 u32 rdev_minor
//! 136 u32 dev_major       140 u32 dev_minor       144 u64 stx_mnt_id ...
//! ts = { i64 tv_sec; u32 tv_nsec; i32 reserved } (16 bytes); total size 256.
//! ```

use std::path::Path;

use mj_core::{Attributes, EntryKind, FileEntry, FileId, Timestamp};

/// Size of `struct statx`.
pub const STATX_LEN: usize = 256;

/// `stx_mask` bits (what the filesystem actually filled in).
pub mod mask {
    pub const TYPE: u32 = 0x1;
    pub const MODE: u32 = 0x2;
    pub const NLINK: u32 = 0x4;
    pub const UID: u32 = 0x8;
    pub const ATIME: u32 = 0x20;
    pub const MTIME: u32 = 0x40;
    pub const CTIME: u32 = 0x80;
    pub const INO: u32 = 0x100;
    pub const SIZE: u32 = 0x200;
    pub const BLOCKS: u32 = 0x400;
    /// Everything `stat(2)` returns.
    pub const BASIC_STATS: u32 = 0x7ff;
    pub const BTIME: u32 = 0x800;
}

/// `stx_attributes` bits mapped to [`Attributes`].
pub mod attr {
    pub const COMPRESSED: u64 = 0x4;
    pub const IMMUTABLE: u64 = 0x10;
    pub const ENCRYPTED: u64 = 0x800;
}

const S_IFMT: u16 = 0o170_000;
const S_IFDIR: u16 = 0o040_000;
const S_IFREG: u16 = 0o100_000;
const S_IFLNK: u16 = 0o120_000;

/// One `statx_timestamp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatxTime {
    pub sec: i64,
    pub nsec: u32,
}

impl StatxTime {
    fn to_timestamp(self) -> Timestamp {
        Timestamp(self.sec.saturating_mul(1_000_000_000).saturating_add(i64::from(self.nsec)))
    }
}

/// The fields of `struct statx` MyJanitor uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Statx {
    pub mask: u32,
    pub attributes: u64,
    pub nlink: u32,
    pub uid: u32,
    pub mode: u16,
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub atime: StatxTime,
    pub btime: StatxTime,
    pub ctime: StatxTime,
    pub mtime: StatxTime,
    pub dev_major: u32,
    pub dev_minor: u32,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}
fn ts_at(b: &[u8], o: usize) -> StatxTime {
    StatxTime { sec: u64_at(b, o) as i64, nsec: u32_at(b, o + 8) }
}

/// Decode a `struct statx` buffer. `None` when it is shorter than the fields we read.
pub fn parse_statx(b: &[u8]) -> Option<Statx> {
    if b.len() < 144 {
        return None;
    }
    Some(Statx {
        mask: u32_at(b, 0),
        attributes: u64_at(b, 8),
        nlink: u32_at(b, 16),
        uid: u32_at(b, 20),
        mode: u16_at(b, 28),
        ino: u64_at(b, 32),
        size: u64_at(b, 40),
        blocks: u64_at(b, 48),
        atime: ts_at(b, 64),
        btime: ts_at(b, 80),
        ctime: ts_at(b, 96),
        mtime: ts_at(b, 112),
        dev_major: u32_at(b, 136),
        dev_minor: u32_at(b, 140),
    })
}

/// glibc's `gnu_dev_makedev`: the `st_dev` value `std::fs::Metadata::dev()` reports,
/// so file ids from `statx` and from the std fallback are identical.
pub fn makedev(major: u32, minor: u32) -> u64 {
    let (major, minor) = (u64::from(major), u64::from(minor));
    ((major & 0xffff_f000) << 32) | ((major & 0x0000_0fff) << 8) | ((minor & 0xffff_ff00) << 12) | (minor & 0x0000_00ff)
}

/// Build the [`FileEntry`] for `path` from decoded `statx` data. Pure.
/// Fields the filesystem did not fill in (`mask`) stay `None`.
pub fn entry_from_statx(path: &Path, s: &Statx) -> FileEntry {
    let kind = match s.mode & S_IFMT {
        S_IFLNK => EntryKind::Link,
        S_IFDIR => EntryKind::Dir,
        S_IFREG => EntryKind::File,
        _ => EntryKind::Other,
    };
    let has = |bit: u32| s.mask & bit != 0;
    let size = if kind == EntryKind::File && has(mask::SIZE) { s.size } else { 0 };
    let mut e = FileEntry::new(path, kind, size);
    if has(mask::MTIME) {
        e.modified = Some(s.mtime.to_timestamp());
    }
    if has(mask::ATIME) {
        e.accessed = Some(s.atime.to_timestamp());
    }
    if has(mask::BTIME) {
        e.created = Some(s.btime.to_timestamp());
    }
    if has(mask::CTIME) {
        e.changed = Some(s.ctime.to_timestamp());
    }
    let allocated = s.blocks.saturating_mul(512);
    if has(mask::BLOCKS) {
        e.allocated = Some(allocated);
    }
    if has(mask::INO) {
        e.file_id = Some(FileId { volume: makedev(s.dev_major, s.dev_minor), index: s.ino });
    }
    if has(mask::NLINK) {
        e.links = Some(s.nlink);
    }
    if has(mask::UID) {
        e.owner = Some(format!("uid:{}", s.uid));
    }
    if has(mask::MODE) && s.mode & 0o222 == 0 {
        e.attrs |= Attributes::READONLY;
    }
    if s.attributes & attr::COMPRESSED != 0 {
        e.attrs |= Attributes::COMPRESSED;
    }
    if s.attributes & attr::IMMUTABLE != 0 {
        e.attrs |= Attributes::IMMUTABLE;
    }
    if s.attributes & attr::ENCRYPTED != 0 {
        e.attrs |= Attributes::ENCRYPTED;
    }
    if e.name().starts_with('.') {
        e.attrs |= Attributes::HIDDEN;
    }
    // Sparse: fewer bytes allocated than the logical size by more than one block.
    if kind == EntryKind::File && has(mask::BLOCKS) && allocated + 4096 < s.size {
        e.attrs |= Attributes::SPARSE;
    }
    e
}

#[cfg(target_os = "linux")]
pub use sys::statx_path;

#[cfg(target_os = "linux")]
mod sys {
    use std::io;
    use std::os::raw::{c_char, c_int, c_long, c_uint};
    use std::path::Path;
    use std::sync::atomic::{AtomicU8, Ordering};

    use super::{parse_statx, Statx, STATX_LEN};
    use crate::ffi::linux::cpath;

    extern "C" {
        /// glibc's generic system call entry point (variadic).
        fn syscall(number: c_long, ...) -> c_long;
    }

    #[cfg(target_arch = "x86_64")]
    const SYS_STATX: Option<c_long> = Some(332);
    #[cfg(target_arch = "aarch64")]
    const SYS_STATX: Option<c_long> = Some(291);
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    const SYS_STATX: Option<c_long> = None;

    const AT_FDCWD: c_int = -100;
    const AT_SYMLINK_NOFOLLOW: c_int = 0x100;
    /// Do not trigger an automount on the last component (scanning must not mount things).
    const AT_NO_AUTOMOUNT: c_int = 0x800;
    const ENOSYS: i32 = 38;
    const EPERM: i32 = 1;

    /// 0 = untried, 1 = works, 2 = unavailable (use std).
    static STATE: AtomicU8 = AtomicU8::new(0);

    /// `statx` of `path` without following links. `None` when `statx` is not
    /// available here (the caller falls back to std); `Some(Err)` for real errors.
    pub fn statx_path(path: &Path) -> Option<io::Result<Statx>> {
        let nr = SYS_STATX?;
        if STATE.load(Ordering::Relaxed) == 2 {
            return None;
        }
        let c = match cpath(path) {
            Ok(c) => c,
            Err(e) => return Some(Err(e)),
        };
        // u64 array: 8-byte aligned like the kernel struct, exactly STATX_LEN bytes.
        let mut buf = [0u64; STATX_LEN / 8];
        let flags = AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT;
        let want: c_uint = super::mask::BASIC_STATS | super::mask::BTIME;
        // SAFETY: `c` is a valid NUL-terminated string and `buf` is a writable,
        // suitably aligned 256-byte buffer; both outlive the call. Every variadic
        // argument is passed as the register-sized type the kernel ABI expects.
        let rc = unsafe {
            syscall(
                nr,
                AT_FDCWD as c_long,
                c.as_ptr() as *const c_char,
                flags as c_long,
                want as c_long,
                buf.as_mut_ptr() as *mut u8,
            )
        };
        if rc != 0 {
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                Some(ENOSYS) | Some(EPERM) if STATE.load(Ordering::Relaxed) != 1 => {
                    STATE.store(2, Ordering::Relaxed);
                    return None;
                }
                _ => return Some(Err(err)),
            }
        }
        STATE.store(1, Ordering::Relaxed);
        let mut bytes = [0u8; STATX_LEN];
        for (i, w) in buf.iter().enumerate() {
            bytes[i * 8..i * 8 + 8].copy_from_slice(&w.to_ne_bytes());
        }
        parse_statx(&bytes).map(Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(b: &mut [u8], o: usize, v: &[u8]) {
        b[o..o + v.len()].copy_from_slice(v);
    }

    fn sample() -> [u8; STATX_LEN] {
        let mut b = [0u8; STATX_LEN];
        put(&mut b, 0, &(mask::BASIC_STATS | mask::BTIME).to_le_bytes());
        put(&mut b, 8, &(attr::COMPRESSED | attr::IMMUTABLE).to_le_bytes());
        put(&mut b, 16, &3u32.to_le_bytes());
        put(&mut b, 20, &1000u32.to_le_bytes());
        put(&mut b, 28, &(0o100_444u16).to_le_bytes());
        put(&mut b, 32, &77u64.to_le_bytes());
        put(&mut b, 40, &10_000u64.to_le_bytes());
        put(&mut b, 48, &8u64.to_le_bytes());
        put(&mut b, 64, &5i64.to_le_bytes());
        put(&mut b, 72, &6u32.to_le_bytes());
        put(&mut b, 80, &1i64.to_le_bytes());
        put(&mut b, 96, &3i64.to_le_bytes());
        put(&mut b, 112, &1_700_000_000i64.to_le_bytes());
        put(&mut b, 120, &123_456_789u32.to_le_bytes());
        put(&mut b, 136, &8u32.to_le_bytes());
        put(&mut b, 140, &1u32.to_le_bytes());
        b
    }

    #[test]
    fn parses_every_field_at_its_offset() {
        let s = parse_statx(&sample()).unwrap();
        assert_eq!(s.nlink, 3);
        assert_eq!(s.uid, 1000);
        assert_eq!(s.ino, 77);
        assert_eq!(s.size, 10_000);
        assert_eq!(s.blocks, 8);
        assert_eq!(s.atime, StatxTime { sec: 5, nsec: 6 });
        assert_eq!(s.mtime, StatxTime { sec: 1_700_000_000, nsec: 123_456_789 });
        assert_eq!((s.dev_major, s.dev_minor), (8, 1));
        assert!(parse_statx(&[0u8; 100]).is_none());
    }

    #[test]
    fn builds_entry_with_allocated_size_and_attributes() {
        let s = parse_statx(&sample()).unwrap();
        let e = entry_from_statx(Path::new("/x/.f"), &s);
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.size, 10_000);
        assert_eq!(e.allocated, Some(4096));
        assert!(e.attrs.contains(Attributes::SPARSE | Attributes::COMPRESSED | Attributes::IMMUTABLE));
        assert!(e.attrs.contains(Attributes::READONLY | Attributes::HIDDEN));
        assert_eq!(e.links, Some(3));
        assert_eq!(e.file_id, Some(FileId { volume: makedev(8, 1), index: 77 }));
        assert_eq!(e.modified, Some(Timestamp(1_700_000_000_123_456_789)));
        assert_eq!(e.created, Some(Timestamp(1_000_000_000)));
        assert_eq!(e.owner.as_deref(), Some("uid:1000"));
    }

    #[test]
    fn missing_mask_bits_leave_fields_unknown_and_types_map() {
        let mut b = sample();
        put(&mut b, 0, &(mask::TYPE | mask::MODE).to_le_bytes());
        put(&mut b, 28, &(0o040_755u16).to_le_bytes());
        let e = entry_from_statx(Path::new("/d"), &parse_statx(&b).unwrap());
        assert_eq!(e.kind, EntryKind::Dir);
        assert_eq!((e.size, e.allocated, e.modified, e.created, e.file_id), (0, None, None, None, None));
        put(&mut b, 28, &(0o120_777u16).to_le_bytes());
        assert_eq!(entry_from_statx(Path::new("/l"), &parse_statx(&b).unwrap()).kind, EntryKind::Link);
        put(&mut b, 28, &(0o010_644u16).to_le_bytes());
        assert_eq!(entry_from_statx(Path::new("/p"), &parse_statx(&b).unwrap()).kind, EntryKind::Other);
    }

    #[test]
    fn makedev_matches_glibc_encoding() {
        assert_eq!(makedev(8, 1), 0x801);
        assert_eq!(makedev(259, 3), (259 << 8) | 3);
        assert_eq!(makedev(0x1_2345, 0x6_789a), (0x1_2000u64 << 32) | 0x3_4500 | 0x6780_0000 | 0x9a);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_statx_agrees_with_std() {
        use std::os::unix::fs::MetadataExt;
        let t = mj_core::testutil::TempDir::new("statx").unwrap();
        let f = t.write("a.bin", &[7u8; 5000]).unwrap();
        let Some(r) = statx_path(&f) else { return };
        let e = entry_from_statx(&f, &r.unwrap());
        let md = std::fs::symlink_metadata(&f).unwrap();
        assert_eq!(e.size, 5000);
        assert_eq!(e.file_id, Some(FileId { volume: md.dev(), index: md.ino() }));
        assert_eq!(e.allocated, Some(md.blocks() * 512));
        assert_eq!(e.modified, md.modified().ok().map(Timestamp::from_system_time));
        let err = statx_path(&t.join("missing")).unwrap().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
