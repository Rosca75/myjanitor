//! Recycle Bin (`SHFileOperationW` + `FOF_ALLOWUNDO`) and the freedesktop.org Trash
//! spec (`$XDG_DATA_HOME/Trash` + per-volume `.Trash/$UID` / `.Trash-$UID`).
//! Owner: W2-E.
//!
//! ## Linux (freedesktop.org Trash specification 1.0, implemented in-house)
//!
//! - The **home trash** is `$XDG_DATA_HOME/Trash` (default `~/.local/share/Trash`),
//!   used for items on the same device as the home trash.
//! - Items on another device go to that device's top directory: `$topdir/.Trash/$uid`
//!   when `$topdir/.Trash` is a real (non-link) directory with the sticky bit, else
//!   `$topdir/.Trash-$uid` (created with mode 0700, must be a real directory owned by
//!   the user). Nothing is ever copied across devices: when no trash is usable on the
//!   item's device the call fails with `Unsupported` and the item is left alone.
//! - The name is reserved atomically by creating `info/<name>.trashinfo` with
//!   `O_EXCL` (collisions get `name.2.ext`, `name.3.ext`...), the info file is
//!   written and synced, then the item is renamed into `files/` with
//!   `RENAME_NOREPLACE` (never overwriting a stale entry).
//! - `.trashinfo` holds `Path=` (percent-encoded; absolute for the home trash,
//!   relative to `$topdir` for volume trashes) and `DeletionDate=` in local time.
//! - `directorysizes` (optional in the spec) is not maintained; file managers
//!   recompute sizes when it is missing.
//! - Restore reads the `.trashinfo` back, checks it names the expected original,
//!   refuses when the original path is occupied, and renames the item back.
//!
//! ## Windows (compile-checked only from Linux)
//!
//! `SHFileOperationW(FO_DELETE)` with `FOF_ALLOWUNDO | FOF_NOCONFIRMATION |
//! FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING`. `FOF_WANTNUKEWARNING` is added
//! to the brief's flags because without it the shell *permanently deletes* items
//! that cannot be recycled (too large for the bin) without asking; with it the
//! user is warned instead. Only paths on fixed drives are accepted (network and
//! removable drives have no Recycle Bin, so the shell would delete permanently).
//!
//! The documented API returns no handle to the recycled item, so the ticket has no
//! `location`. [`restore`] is **best effort**: it looks for the `$I` record of the
//! original in `<drive>\$Recycle.Bin\<SID>\` (an undocumented but stable format:
//! version 1 = fixed 520-byte path, version 2 = length-prefixed path), matching
//! the original path case-insensitively and the deletion time within ten
//! minutes of the ticket, and renames the paired `$R` item back. Because this is
//! not exact, `mj-actions` sends Windows "Trash" actions to the app quarantine by
//! default (docs/decisions/W2-E.md), so History can always restore them.

use std::path::{Path, PathBuf};

use mj_core::{Error, Result, Timestamp, TrashTicket};

/// Where the Linux trash lives and whether per-volume trashes may be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashConfig {
    /// The home trash (`$XDG_DATA_HOME/Trash`). Tests point this into a temp dir.
    pub home_trash: PathBuf,
    /// User id used for `.Trash/$uid` and `.Trash-$uid`.
    pub uid: u32,
    /// Allow per-volume trash directories for items on other devices.
    pub volume_trash: bool,
}

impl TrashConfig {
    /// The configuration of the running user from `XDG_DATA_HOME` / `HOME`.
    pub fn from_env() -> Result<TrashConfig> {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .filter(|p| p.is_absolute())
            .ok_or_else(|| Error::unsupported("Trash", "neither XDG_DATA_HOME nor HOME is set"))?;
        Ok(TrashConfig { home_trash: data.join("Trash"), uid: current_uid(), volume_trash: true })
    }
}

fn current_uid() -> u32 {
    #[cfg(target_os = "linux")]
    {
        crate::ffi::linux::current_uid()
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

/// Move `path` to the trash and say where it went.
pub fn trash(path: &Path) -> Result<TrashTicket> {
    #[cfg(windows)]
    {
        windows_impl::trash(path)
    }
    #[cfg(not(windows))]
    {
        trash_with(path, &TrashConfig::from_env()?, Timestamp::now())
    }
}

/// Restore a trashed item to `original`.
pub fn restore(ticket: &TrashTicket, original: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        windows_impl::restore(ticket, original)
    }
    #[cfg(not(windows))]
    {
        restore_freedesktop(ticket, original)
    }
}

// ---------------------------------------------------------------- freedesktop (pure parts)

/// Percent-encode a path for `Path=` (everything but RFC 3986 unreserved bytes and `/`).
pub fn percent_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            s.push(b as char);
        } else {
            s.push('%');
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 15) as usize] as char);
        }
    }
    s
}

/// Inverse of [`percent_encode`]; `None` on a malformed escape.
pub fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

/// The `.trashinfo` text for an item (`path_bytes` already absolute or topdir-relative).
pub fn trashinfo_text(path_bytes: &[u8], deletion_date: &str) -> String {
    format!("[Trash Info]\nPath={}\nDeletionDate={deletion_date}\n", percent_encode(path_bytes))
}

/// Parsed `.trashinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashInfo {
    /// Decoded `Path=` bytes (absolute, or relative to the volume's top directory).
    pub path: Vec<u8>,
    pub deletion_date: Option<String>,
}

/// Parse a `.trashinfo` file (the `[Trash Info]` group must come first; other groups ignored).
pub fn parse_trashinfo(text: &str) -> Option<TrashInfo> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
    if lines.next()? != "[Trash Info]" {
        return None;
    }
    let mut path = None;
    let mut date = None;
    for l in lines {
        if l.starts_with('[') {
            break;
        }
        if let Some(v) = l.strip_prefix("Path=") {
            path = Some(percent_decode(v)?);
        } else if let Some(v) = l.strip_prefix("DeletionDate=") {
            date = Some(v.to_string());
        }
    }
    Some(TrashInfo { path: path?, deletion_date: date })
}

/// `YYYY-MM-DDThh:mm:ss` from a timestamp and a UTC offset in seconds.
pub fn format_deletion_date(t: Timestamp, offset_secs: i64) -> String {
    let local = Timestamp::from_secs(t.secs() + offset_secs);
    let (y, m, d) = local.ymd();
    let (hh, mm, ss) = local.hms();
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}")
}

/// Candidate trash names: `name`, `name.2.ext`, `name.3.ext`... (folders: `name.2`).
pub fn trash_name_candidate(name: &std::ffi::OsStr, n: u32, is_dir: bool) -> std::ffi::OsString {
    if n <= 1 {
        return name.to_os_string();
    }
    let p = Path::new(name);
    match (is_dir, p.file_stem(), p.extension()) {
        (false, Some(stem), Some(ext)) => {
            let mut s = stem.to_os_string();
            s.push(format!(".{n}."));
            s.push(ext);
            s
        }
        _ => {
            let mut s = name.to_os_string();
            s.push(format!(".{n}"));
            s
        }
    }
}

// ---------------------------------------------------------------- freedesktop (I/O)

#[cfg(unix)]
mod fd {
    use std::fs;
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    use std::path::{Path, PathBuf};

    use mj_core::{Error, Result, Timestamp, TrashTicket};

    use super::*;
    use crate::action::{rename_no_replace_io, sync_dir};

    /// Create `dir` (and parents) with mode 0700 where missing.
    fn mkdir_private(dir: &Path) -> Result<()> {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| Error::io(dir, e))
    }

    /// A real (non-link) directory owned by `uid`.
    fn is_own_real_dir(p: &Path, uid: u32) -> bool {
        fs::symlink_metadata(p).map(|m| m.file_type().is_dir() && m.uid() == uid).unwrap_or(false)
    }

    /// The per-volume trash under `topdir` per the spec's checks (creating it when allowed).
    pub fn volume_trash_dir(topdir: &Path, uid: u32) -> Result<PathBuf> {
        let admin = topdir.join(".Trash");
        if let Ok(m) = fs::symlink_metadata(&admin) {
            // (1) $topdir/.Trash: a real directory with the sticky bit.
            if m.file_type().is_dir() && m.mode() & 0o1000 != 0 {
                let own = admin.join(uid.to_string());
                if fs::symlink_metadata(&own).is_err() {
                    let _ = fs::DirBuilder::new().mode(0o700).create(&own);
                }
                if is_own_real_dir(&own, uid) {
                    return Ok(own);
                }
            }
        }
        // (2) $topdir/.Trash-$uid.
        let own = topdir.join(format!(".Trash-{uid}"));
        if fs::symlink_metadata(&own).is_err() {
            let _ = fs::DirBuilder::new().mode(0o700).create(&own);
        }
        if is_own_real_dir(&own, uid) {
            return Ok(own);
        }
        Err(Error::unsupported("Trash", format!("no usable trash directory on the volume of {}", topdir.display())))
    }

    /// The top directory (mount point) of the filesystem holding `dir`.
    fn top_dir(dir: &Path, dev: u64) -> PathBuf {
        let mut top = dir.to_path_buf();
        let mut cur = dir.parent();
        while let Some(p) = cur {
            match fs::metadata(p) {
                Ok(m) if m.dev() == dev => top = p.to_path_buf(),
                _ => break,
            }
            cur = p.parent();
        }
        top
    }

    /// Local UTC offset of `t` (seconds), from `localtime_r`; 0 when unavailable.
    fn local_offset(t: Timestamp) -> i64 {
        #[cfg(target_os = "linux")]
        {
            #[repr(C)]
            struct Tm {
                tm_sec: i32,
                tm_min: i32,
                tm_hour: i32,
                tm_mday: i32,
                tm_mon: i32,
                tm_year: i32,
                tm_wday: i32,
                tm_yday: i32,
                tm_isdst: i32,
                tm_gmtoff: i64,
                tm_zone: *const std::os::raw::c_char,
            }
            extern "C" {
                fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
            }
            let secs: i64 = t.secs();
            let mut tm = Tm {
                tm_sec: 0,
                tm_min: 0,
                tm_hour: 0,
                tm_mday: 0,
                tm_mon: 0,
                tm_year: 0,
                tm_wday: 0,
                tm_yday: 0,
                tm_isdst: 0,
                tm_gmtoff: 0,
                tm_zone: std::ptr::null(),
            };
            // SAFETY: `secs` and `tm` are valid for the call; glibc's `struct tm` on 64-bit
            // Linux is nine ints followed by `long tm_gmtoff` and `const char *tm_zone`.
            let r = unsafe { localtime_r(&secs, &mut tm) };
            if r.is_null() {
                0
            } else {
                tm.tm_gmtoff.clamp(-86_400, 86_400)
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = t;
            0
        }
    }

    pub fn trash_with(path: &Path, cfg: &TrashConfig, now: Timestamp) -> Result<TrashTicket> {
        if !path.is_absolute() {
            return Err(Error::InvalidInput(format!("not an absolute path: {}", path.display())));
        }
        let md = fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
        let name = path.file_name().ok_or_else(|| Error::InvalidInput(format!("no file name: {}", path.display())))?;
        let parent = path.parent().ok_or_else(|| Error::InvalidInput(format!("no parent: {}", path.display())))?;
        // The item's device is the device of its directory entry (lstat), also for links.
        let dev = md.dev();
        mkdir_private(&cfg.home_trash)?;
        let home_dev = fs::metadata(&cfg.home_trash).map_err(|e| Error::io(&cfg.home_trash, e))?.dev();
        let (trash_dir, top) = if dev == home_dev {
            (cfg.home_trash.clone(), None)
        } else if cfg.volume_trash {
            let parent_canon = fs::canonicalize(parent).map_err(|e| Error::io(parent, e))?;
            let top = top_dir(&parent_canon, dev);
            (volume_trash_dir(&top, cfg.uid)?, Some(top))
        } else {
            return Err(Error::unsupported("Trash", format!("{} is on another volume than the trash", path.display())));
        };
        let files = trash_dir.join("files");
        let info = trash_dir.join("info");
        mkdir_private(&files)?;
        mkdir_private(&info)?;
        // Path= value: absolute for the home trash, relative to the top directory otherwise.
        let recorded: Vec<u8> = match &top {
            None => path.as_os_str().as_bytes().to_vec(),
            Some(top) => {
                let parent_canon = fs::canonicalize(parent).map_err(|e| Error::io(parent, e))?;
                let full = parent_canon.join(name);
                full.strip_prefix(top)
                    .map(|r| r.as_os_str().as_bytes().to_vec())
                    .unwrap_or_else(|_| full.as_os_str().as_bytes().to_vec())
            }
        };
        let text = trashinfo_text(&recorded, &format_deletion_date(now, local_offset(now)));
        let is_dir = md.file_type().is_dir();
        for n in 1..=10_000u32 {
            let cand = trash_name_candidate(name, n, is_dir);
            let dest = files.join(&cand);
            if fs::symlink_metadata(&dest).is_ok() {
                continue;
            }
            let mut info_name = cand.clone();
            info_name.push(".trashinfo");
            let info_path = info.join(&info_name);
            // Atomic name reservation: O_EXCL create of the info file.
            let mut f = match fs::OpenOptions::new().write(true).create_new(true).open(&info_path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::io(&info_path, e)),
            };
            let written = f.write_all(text.as_bytes()).and_then(|_| f.sync_all());
            drop(f);
            if let Err(e) = written {
                let _ = fs::remove_file(&info_path);
                return Err(Error::io(&info_path, e));
            }
            match rename_no_replace_io(path, &dest) {
                Ok(()) => {
                    let _ = sync_dir(&files);
                    let _ = sync_dir(&info);
                    let _ = sync_dir(parent);
                    return Ok(TrashTicket { location: Some(dest), info: Some(info_path), trashed_at: now });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&info_path);
                    continue;
                }
                Err(e) => {
                    let _ = fs::remove_file(&info_path);
                    if crate::action::is_cross_device(&e) {
                        return Err(Error::unsupported("Trash", format!("{} is on another volume", path.display())));
                    }
                    return Err(Error::io(path, e));
                }
            }
        }
        Err(Error::Conflict(format!("could not find a free trash name for {}", path.display())))
    }

    /// The top directory a volume trash belongs to, from its info file path.
    fn topdir_of_info(info: &Path) -> Option<PathBuf> {
        let trash_dir = info.parent()?.parent()?;
        let name = trash_dir.file_name()?.to_string_lossy().into_owned();
        if name.starts_with(".Trash-") {
            return trash_dir.parent().map(Path::to_path_buf);
        }
        let admin = trash_dir.parent()?;
        if admin.file_name()? == ".Trash" {
            return admin.parent().map(Path::to_path_buf);
        }
        None
    }

    pub fn restore_freedesktop(ticket: &TrashTicket, original: &Path) -> Result<()> {
        let (Some(loc), Some(info)) = (&ticket.location, &ticket.info) else {
            return Err(Error::InvalidInput("the trash ticket has no location".into()));
        };
        let mut text = String::new();
        {
            use std::io::Read;
            let f = fs::File::open(info).map_err(|e| Error::io(info, e))?;
            f.take(64 * 1024).read_to_string(&mut text).map_err(|e| Error::io(info, e))?;
        }
        let parsed = parse_trashinfo(&text)
            .ok_or_else(|| Error::parse("trashinfo", format!("{} is malformed", info.display())))?;
        let recorded = PathBuf::from(std::ffi::OsStr::from_bytes(&parsed.path));
        let resolved = if recorded.is_absolute() {
            recorded
        } else {
            topdir_of_info(info)
                .ok_or_else(|| Error::parse("trashinfo", "relative path outside a volume trash"))?
                .join(recorded)
        };
        let orig_parent_canon = original.parent().and_then(|p| fs::canonicalize(p).ok());
        let same = mj_core::pathutil::path_key(&resolved) == mj_core::pathutil::path_key(original)
            || orig_parent_canon.zip(original.file_name()).is_some_and(|(p, n)| {
                mj_core::pathutil::path_key(&p.join(n)) == mj_core::pathutil::path_key(&resolved)
            });
        if !same {
            return Err(Error::Conflict(format!(
                "the trash entry belongs to {}, not {}",
                resolved.display(),
                original.display()
            )));
        }
        if fs::symlink_metadata(original).is_ok() {
            return Err(Error::Conflict(format!("{} exists again; nothing was overwritten", original.display())));
        }
        fs::symlink_metadata(loc).map_err(|e| Error::io(loc, e))?;
        if let Some(parent) = original.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        rename_no_replace_io(loc, original).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Error::Conflict(format!("{} exists again; nothing was overwritten", original.display()))
            } else {
                Error::io(loc, e)
            }
        })?;
        let _ = fs::remove_file(info);
        if let Some(parent) = original.parent() {
            let _ = sync_dir(parent);
        }
        Ok(())
    }
}

#[cfg(unix)]
pub use fd::{restore_freedesktop, trash_with, volume_trash_dir};

/// Trash into an explicit configuration (Linux / Unix).
#[cfg(not(unix))]
pub fn trash_with(path: &Path, cfg: &TrashConfig, now: Timestamp) -> Result<TrashTicket> {
    let _ = (path, cfg, now);
    Err(Error::unsupported("freedesktop Trash", "not available on this platform"))
}

/// Restore from a freedesktop trash ticket (Linux / Unix).
#[cfg(not(unix))]
pub fn restore_freedesktop(ticket: &TrashTicket, original: &Path) -> Result<()> {
    let _ = (ticket, original);
    Err(Error::unsupported("freedesktop Trash", "not available on this platform"))
}

// ---------------------------------------------------------------- Windows (pure parts, tested everywhere)

/// `SHFILEOPSTRUCTW.fFlags` used for recycling.
pub const FOF_FLAGS: u16 = 0x0040 /* ALLOWUNDO */
    | 0x0010 /* NOCONFIRMATION */
    | 0x0004 /* SILENT */
    | 0x0400 /* NOERRORUI */
    | 0x4000 /* WANTNUKEWARNING */;

/// The `pFrom` buffer for one path: the plain absolute path (the shell does not accept the
/// `\\?\` prefix), followed by two NULs. `None` when the path is empty, relative, contains
/// NUL, or is too long for the shell (it is limited to `MAX_PATH`).
pub fn shell_from_buffer(wide: &[u16]) -> Option<Vec<u16>> {
    let s: Vec<u16> = wide.iter().copied().take_while(|&c| c != 0).collect();
    if s.len() != wide.iter().filter(|&&c| c != 0).count() {
        return None;
    }
    let prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    let unc: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    let plain: Vec<u16> = if s.starts_with(&unc) {
        let mut v: Vec<u16> = "\\\\".encode_utf16().collect();
        v.extend_from_slice(&s[unc.len()..]);
        v
    } else if s.starts_with(&prefix) {
        s[prefix.len()..].to_vec()
    } else {
        s
    };
    let is_drive = plain.len() >= 3 && plain[1] == b':' as u16 && plain[2] == b'\\' as u16;
    let is_unc = plain.len() >= 3 && plain[0] == b'\\' as u16 && plain[1] == b'\\' as u16;
    if !(is_drive || is_unc) || plain.len() >= 260 || plain.contains(&(b'/' as u16)) {
        return None;
    }
    let mut out = plain;
    out.push(0);
    out.push(0);
    Some(out)
}

/// Parsed `$I` record of the Windows Recycle Bin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecycleRecord {
    pub size: u64,
    /// Deletion time as a FILETIME (100 ns since 1601).
    pub deleted_filetime: u64,
    /// Original path (UTF-16, no terminator).
    pub path: Vec<u16>,
}

/// Parse a `$I` file: version 1 (Vista-8.1: path in a fixed 260-character field) or
/// version 2 (Windows 10+: `u32` character count incl. the terminator, then the path).
pub fn parse_recycle_record(data: &[u8]) -> Option<RecycleRecord> {
    let rd64 = |o: usize| -> Option<u64> {
        let s = data.get(o..o + 8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Some(u64::from_le_bytes(a))
    };
    let version = rd64(0)?;
    let size = rd64(8)?;
    let ft = rd64(16)?;
    let (start, max_chars) = match version {
        1 => (24usize, 260usize),
        2 => {
            let s = data.get(24..28)?;
            let n = u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize;
            if n == 0 || n > 32_768 {
                return None;
            }
            (28usize, n)
        }
        _ => return None,
    };
    let bytes = data.get(start..start + max_chars * 2)?;
    let path: Vec<u16> =
        bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    if path.is_empty() {
        return None;
    }
    Some(RecycleRecord { size, deleted_filetime: ft, path })
}

/// FILETIME (100 ns ticks since 1601) of a timestamp.
pub fn timestamp_to_filetime(t: Timestamp) -> u64 {
    const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
    (t.0 / 100).saturating_add(EPOCH_DIFF_100NS).max(0) as u64
}

/// Case-insensitive (ASCII + simple Unicode lower-case) comparison of two UTF-16 paths,
/// ignoring a `\\?\` prefix.
pub fn same_windows_path(a: &[u16], b: &[u16]) -> bool {
    let norm = |v: &[u16]| -> String {
        let s = String::from_utf16_lossy(v);
        let s = s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string();
        s.replace('/', "\\").trim_end_matches('\\').to_lowercase()
    };
    norm(a) == norm(b)
}

#[cfg(windows)]
#[allow(non_camel_case_types, non_snake_case, clippy::upper_case_acronyms)]
mod windows_impl {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use mj_core::{Error, Result, Timestamp, TrashTicket};

    use super::{parse_recycle_record, same_windows_path, shell_from_buffer, timestamp_to_filetime, FOF_FLAGS};
    use crate::action::rename_no_replace;
    use crate::ffi::windows::{wide, BOOL, LPCWSTR};

    const FO_DELETE: u32 = 3;
    const DRIVE_FIXED: u32 = 3;

    /// `SHFILEOPSTRUCTW` (shellapi.h). On x64 the struct uses natural alignment, which
    /// `repr(C)` reproduces: hwnd 0, wFunc 8, pFrom 16, pTo 24, fFlags 32,
    /// fAnyOperationsAborted 36, hNameMappings 40, lpszProgressTitle 48 (size 56).
    #[repr(C)]
    #[allow(non_snake_case)]
    struct SHFILEOPSTRUCTW {
        hwnd: *mut c_void,
        wFunc: u32,
        pFrom: LPCWSTR,
        pTo: LPCWSTR,
        fFlags: u16,
        fAnyOperationsAborted: BOOL,
        hNameMappings: *mut c_void,
        lpszProgressTitle: LPCWSTR,
    }

    // Layout checks against shellapi.h for x64 (natural alignment, 56 bytes).
    #[cfg(target_pointer_width = "64")]
    const _: () = {
        assert!(std::mem::size_of::<SHFILEOPSTRUCTW>() == 56);
        assert!(std::mem::offset_of!(SHFILEOPSTRUCTW, wFunc) == 8);
        assert!(std::mem::offset_of!(SHFILEOPSTRUCTW, pFrom) == 16);
        assert!(std::mem::offset_of!(SHFILEOPSTRUCTW, fFlags) == 32);
        assert!(std::mem::offset_of!(SHFILEOPSTRUCTW, fAnyOperationsAborted) == 36);
        assert!(std::mem::offset_of!(SHFILEOPSTRUCTW, lpszProgressTitle) == 48);
    };

    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(lpFileOp: *mut SHFILEOPSTRUCTW) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(lpRootPathName: LPCWSTR) -> u32;
    }

    fn drive_root(path: &Path) -> Option<String> {
        let s = path.to_string_lossy();
        let s = s.strip_prefix("\\\\?\\").unwrap_or(&s);
        let b = s.as_bytes();
        if b.len() >= 3 && b[1] == b':' && b[2] == b'\\' {
            Some(s[..3].to_string())
        } else {
            None
        }
    }

    pub fn trash(path: &Path) -> Result<TrashTicket> {
        let root = drive_root(path)
            .ok_or_else(|| Error::unsupported("Recycle Bin", format!("{} is not on a local drive", path.display())))?;
        let wroot = wide(std::ffi::OsStr::new(&root));
        // SAFETY: `wroot` is a NUL-terminated wide string that outlives the call.
        let ty = unsafe { GetDriveTypeW(wroot.as_ptr()) };
        if ty != DRIVE_FIXED {
            return Err(Error::unsupported(
                "Recycle Bin",
                format!("{root} has no Recycle Bin (network or removable drive); nothing was deleted"),
            ));
        }
        let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
        let from = shell_from_buffer(&raw).ok_or_else(|| {
            Error::unsupported("Recycle Bin", format!("{} is too long for the Recycle Bin", path.display()))
        })?;
        std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
        let mut op = SHFILEOPSTRUCTW {
            hwnd: std::ptr::null_mut(),
            wFunc: FO_DELETE,
            pFrom: from.as_ptr(),
            pTo: std::ptr::null(),
            fFlags: FOF_FLAGS,
            fAnyOperationsAborted: 0,
            hNameMappings: std::ptr::null_mut(),
            lpszProgressTitle: std::ptr::null(),
        };
        let now = Timestamp::now();
        // SAFETY: `op` is a fully initialised SHFILEOPSTRUCTW; `from` is double-NUL terminated
        // and outlives the call; no name mappings are requested.
        let rc = unsafe { SHFileOperationW(&mut op) };
        if rc != 0 {
            return Err(Error::Io {
                path: Some(path.to_path_buf()),
                kind: "other",
                message: format!("SHFileOperationW failed with code {rc:#x}"),
            });
        }
        if op.fAnyOperationsAborted != 0 {
            return Err(Error::Cancelled);
        }
        if std::fs::symlink_metadata(path).is_ok() {
            return Err(Error::Io {
                path: Some(path.to_path_buf()),
                kind: "other",
                message: "the Recycle Bin did not take the item".into(),
            });
        }
        Ok(TrashTicket { location: None, info: None, trashed_at: now })
    }

    /// Best-effort restore through the `$I` records (see the module docs).
    pub fn restore(ticket: &TrashTicket, original: &Path) -> Result<()> {
        if std::fs::symlink_metadata(original).is_ok() {
            return Err(Error::Conflict(format!("{} exists again; nothing was overwritten", original.display())));
        }
        let (r_item, i_file) = match (&ticket.location, &ticket.info) {
            (Some(l), Some(i)) => (l.clone(), i.clone()),
            _ => find_record(ticket, original)?,
        };
        if let Some(parent) = original.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        rename_no_replace(&r_item, original)?;
        let _ = std::fs::remove_file(&i_file);
        Ok(())
    }

    fn find_record(ticket: &TrashTicket, original: &Path) -> Result<(PathBuf, PathBuf)> {
        let root = drive_root(original)
            .ok_or_else(|| Error::unsupported("Recycle Bin restore", "the original is not on a local drive"))?;
        let bin = PathBuf::from(format!("{root}$Recycle.Bin"));
        let want: Vec<u16> = original.as_os_str().encode_wide().collect();
        let target_ft = timestamp_to_filetime(ticket.trashed_at);
        const WINDOW: u64 = 10 * 60 * 10_000_000; // ten minutes in FILETIME ticks
        let mut best: Option<(u64, PathBuf, PathBuf)> = None;
        let sids = std::fs::read_dir(&bin).map_err(|e| Error::io(&bin, e))?;
        for sid in sids.flatten() {
            let Ok(items) = std::fs::read_dir(sid.path()) else { continue };
            for it in items.flatten() {
                let name = it.file_name().to_string_lossy().into_owned();
                if !name.starts_with("$I") {
                    continue;
                }
                let Ok(data) = read_bounded(&it.path(), 70_000) else { continue };
                let Some(rec) = parse_recycle_record(&data) else { continue };
                if !same_windows_path(&rec.path, &want) {
                    continue;
                }
                let diff = rec.deleted_filetime.abs_diff(target_ft);
                if diff > WINDOW {
                    continue;
                }
                let r = it.path().with_file_name(format!("$R{}", &name[2..]));
                if std::fs::symlink_metadata(&r).is_err() {
                    continue;
                }
                if best.as_ref().is_none_or(|(d, _, _)| diff < *d) {
                    best = Some((diff, r, it.path()));
                }
            }
        }
        best.map(|(_, r, i)| (r, i)).ok_or_else(|| Error::NotFound(original.to_path_buf()))
    }

    fn read_bounded(p: &Path, max: u64) -> std::io::Result<Vec<u8>> {
        use std::io::Read;
        let mut v = Vec::new();
        std::fs::File::open(p)?.take(max).read_to_end(&mut v)?;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_round_trip() {
        let p = b"/home/me/My Docs/r\xc3\xa9sum\xc3\xa9 100%.txt";
        let e = percent_encode(p);
        assert_eq!(e, "/home/me/My%20Docs/r%C3%A9sum%C3%A9%20100%25.txt");
        assert_eq!(percent_decode(&e).unwrap(), p);
        assert!(percent_decode("%zz").is_none());
        assert!(percent_decode("%4").is_none());
    }

    #[test]
    fn trashinfo_round_trip() {
        let t = trashinfo_text(b"/a b/c", "2026-11-13T10:00:00");
        assert_eq!(t, "[Trash Info]\nPath=/a%20b/c\nDeletionDate=2026-11-13T10:00:00\n");
        let p = parse_trashinfo(&t).unwrap();
        assert_eq!(p.path, b"/a b/c");
        assert_eq!(p.deletion_date.as_deref(), Some("2026-11-13T10:00:00"));
        assert!(parse_trashinfo("Path=/x\n").is_none());
        assert!(parse_trashinfo("[Trash Info]\nDeletionDate=x\n").is_none());
        assert_eq!(format_deletion_date(Timestamp::from_ymd_hms(2026, 1, 2, 3, 4, 5), 3600), "2026-01-02T04:04:05");
    }

    #[test]
    fn collision_names() {
        use std::ffi::OsStr;
        assert_eq!(trash_name_candidate(OsStr::new("a.txt"), 1, false), "a.txt");
        assert_eq!(trash_name_candidate(OsStr::new("a.txt"), 2, false), "a.2.txt");
        assert_eq!(trash_name_candidate(OsStr::new("noext"), 3, false), "noext.3");
        assert_eq!(trash_name_candidate(OsStr::new("dir.d"), 2, true), "dir.d.2");
    }

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn shell_buffer_is_double_nul_without_long_prefix() {
        let b = shell_from_buffer(&w("\\\\?\\C:\\Users\\me\\a.txt")).unwrap();
        assert_eq!(b, [w("C:\\Users\\me\\a.txt"), vec![0, 0]].concat());
        let b = shell_from_buffer(&w("\\\\?\\UNC\\srv\\share\\x")).unwrap();
        assert_eq!(b, [w("\\\\srv\\share\\x"), vec![0, 0]].concat());
        assert!(shell_from_buffer(&w("relative\\x")).is_none());
        assert!(shell_from_buffer(&w("C:/x")).is_none());
        assert!(shell_from_buffer(&[w("C:\\a"), vec![0], w("b")].concat()).is_none(), "embedded NUL");
        assert!(shell_from_buffer(&w(&format!("C:\\{}", "x".repeat(300)))).is_none());
        // A trailing terminator is fine.
        assert!(shell_from_buffer(&[w("C:\\a"), vec![0]].concat()).is_some());
    }

    #[test]
    fn recycle_records_v1_and_v2() {
        let path = w("C:\\Users\\me\\doc.txt");
        let mut v2 = Vec::new();
        v2.extend_from_slice(&2u64.to_le_bytes());
        v2.extend_from_slice(&1234u64.to_le_bytes());
        v2.extend_from_slice(&5678u64.to_le_bytes());
        v2.extend_from_slice(&((path.len() + 1) as u32).to_le_bytes());
        for c in path.iter().chain(std::iter::once(&0)) {
            v2.extend_from_slice(&c.to_le_bytes());
        }
        let r = parse_recycle_record(&v2).unwrap();
        assert_eq!((r.size, r.deleted_filetime), (1234, 5678));
        assert_eq!(r.path, path);
        let mut v1 = vec![0u8; 24 + 520];
        v1[0] = 1;
        for (i, c) in path.iter().enumerate() {
            v1[24 + i * 2..26 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        assert_eq!(parse_recycle_record(&v1).unwrap().path, path);
        assert!(parse_recycle_record(&v2[..30]).is_none());
        assert!(parse_recycle_record(&[9u8; 600]).is_none());
        assert!(same_windows_path(&w("\\\\?\\C:\\USERS\\Me\\Doc.txt"), &path));
        assert!(!same_windows_path(&w("C:\\Users\\me\\doc2.txt"), &path));
        assert_eq!(timestamp_to_filetime(Timestamp(0)), 116_444_736_000_000_000);
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::super::*;
        use mj_core::testutil::TempDir;
        use std::os::unix::fs::PermissionsExt;

        fn cfg(t: &TempDir) -> TrashConfig {
            TrashConfig { home_trash: t.join("xdg/Trash"), uid: crate::ffi::linux::current_uid(), volume_trash: false }
        }

        #[test]
        fn trash_and_restore_in_home_trash() {
            let t = TempDir::new("trash").unwrap();
            let f = t.write("work/a b.txt", b"content").unwrap();
            let now = Timestamp::from_ymd_hms(2026, 11, 13, 12, 0, 0);
            let ticket = trash_with(&f, &cfg(&t), now).unwrap();
            assert!(!f.exists());
            let loc = ticket.location.clone().unwrap();
            assert_eq!(loc, t.join("xdg/Trash/files/a b.txt"));
            let info = std::fs::read_to_string(ticket.info.as_ref().unwrap()).unwrap();
            let parsed = parse_trashinfo(&info).unwrap();
            assert_eq!(parsed.path, std::os::unix::ffi::OsStrExt::as_bytes(f.as_os_str()));
            assert!(info.contains("Path=") && info.contains("a%20b.txt"));
            let date = parsed.deletion_date.unwrap();
            assert_eq!(date.len(), 19);
            assert!(date.starts_with("2026-11-1"), "{date}");
            let mode = std::fs::metadata(t.join("xdg/Trash")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);

            // Name collision: a second file with the same name gets `a b.2.txt`.
            let f2 = t.write("work/a b.txt", b"second").unwrap();
            let t2 = trash_with(&f2, &cfg(&t), now).unwrap();
            assert_eq!(t2.location.as_ref().unwrap(), &t.join("xdg/Trash/files/a b.2.txt"));
            assert!(t2.info.as_ref().unwrap().ends_with("a b.2.txt.trashinfo"));
            // A stale item in files/ without info is never overwritten.
            std::fs::write(t.join("xdg/Trash/files/a b.3.txt"), b"stale").unwrap();
            let f3 = t.write("work/a b.txt", b"third").unwrap();
            let t3 = trash_with(&f3, &cfg(&t), now).unwrap();
            assert_eq!(t3.location.as_ref().unwrap(), &t.join("xdg/Trash/files/a b.4.txt"));
            assert_eq!(std::fs::read(t.join("xdg/Trash/files/a b.3.txt")).unwrap(), b"stale");

            // Restore refuses an occupied original, then succeeds.
            let blocker = t.write("work/a b.txt", b"blocker").unwrap();
            assert_eq!(restore_freedesktop(&ticket, &f).unwrap_err().code(), "conflict");
            std::fs::remove_file(&blocker).unwrap();
            // Restore refuses a ticket for another path.
            assert_eq!(restore_freedesktop(&ticket, &t.join("work/other.txt")).unwrap_err().code(), "conflict");
            restore_freedesktop(&ticket, &f).unwrap();
            assert_eq!(std::fs::read(&f).unwrap(), b"content");
            assert!(!ticket.info.as_ref().unwrap().exists());
            assert!(!loc.exists());
        }

        #[test]
        fn trashes_folders_and_links_without_following() {
            let t = TempDir::new("trashdir").unwrap();
            t.write("work/d/x.txt", b"x").unwrap();
            t.write("outside/keep.txt", b"keep").unwrap();
            std::os::unix::fs::symlink(t.join("outside"), t.join("work/link")).unwrap();
            let now = Timestamp::now();
            let a = trash_with(&t.join("work/d"), &cfg(&t), now).unwrap();
            let b = trash_with(&t.join("work/link"), &cfg(&t), now).unwrap();
            assert!(std::fs::symlink_metadata(b.location.as_ref().unwrap()).unwrap().file_type().is_symlink());
            assert_eq!(std::fs::read(t.join("outside/keep.txt")).unwrap(), b"keep");
            restore_freedesktop(&a, &t.join("work/d")).unwrap();
            restore_freedesktop(&b, &t.join("work/link")).unwrap();
            assert_eq!(std::fs::read(t.join("work/d/x.txt")).unwrap(), b"x");
            assert!(std::fs::symlink_metadata(t.join("work/link")).unwrap().file_type().is_symlink());
        }

        #[test]
        fn volume_trash_checks() {
            let t = TempDir::new("voltrash").unwrap();
            let uid = crate::ffi::linux::current_uid();
            // No .Trash: falls back to .Trash-$uid with mode 0700.
            let d = volume_trash_dir(t.path(), uid).unwrap();
            assert_eq!(d, t.join(format!(".Trash-{uid}")));
            assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
            // .Trash without the sticky bit is ignored.
            let t2 = TempDir::new("voltrash2").unwrap();
            std::fs::create_dir(t2.join(".Trash")).unwrap();
            std::fs::set_permissions(t2.join(".Trash"), std::fs::Permissions::from_mode(0o777)).unwrap();
            assert_eq!(volume_trash_dir(t2.path(), uid).unwrap(), t2.join(format!(".Trash-{uid}")));
            // With the sticky bit, $topdir/.Trash/$uid is used.
            let t3 = TempDir::new("voltrash3").unwrap();
            std::fs::create_dir(t3.join(".Trash")).unwrap();
            std::fs::set_permissions(t3.join(".Trash"), std::fs::Permissions::from_mode(0o1777)).unwrap();
            assert_eq!(volume_trash_dir(t3.path(), uid).unwrap(), t3.join(".Trash").join(uid.to_string()));
            // A symlinked .Trash is never used.
            let t4 = TempDir::new("voltrash4").unwrap();
            std::fs::create_dir(t4.join("real")).unwrap();
            std::fs::set_permissions(t4.join("real"), std::fs::Permissions::from_mode(0o1777)).unwrap();
            std::os::unix::fs::symlink(t4.join("real"), t4.join(".Trash")).unwrap();
            assert_eq!(volume_trash_dir(t4.path(), uid).unwrap(), t4.join(format!(".Trash-{uid}")));
            // A .Trash-$uid that is a link is refused.
            let t5 = TempDir::new("voltrash5").unwrap();
            std::fs::create_dir(t5.join("elsewhere")).unwrap();
            std::os::unix::fs::symlink(t5.join("elsewhere"), t5.join(format!(".Trash-{uid}"))).unwrap();
            assert_eq!(volume_trash_dir(t5.path(), uid).unwrap_err().code(), "unsupported");
        }

        #[test]
        fn relative_paths_in_volume_trash_restore() {
            let t = TempDir::new("voltrash-restore").unwrap();
            let uid = crate::ffi::linux::current_uid();
            let trash_dir = volume_trash_dir(t.path(), uid).unwrap();
            std::fs::create_dir_all(trash_dir.join("files")).unwrap();
            std::fs::create_dir_all(trash_dir.join("info")).unwrap();
            std::fs::write(trash_dir.join("files/x.txt"), b"x").unwrap();
            let info = trash_dir.join("info/x.txt.trashinfo");
            std::fs::write(&info, trashinfo_text(b"sub/x.txt", "2026-01-01T00:00:00")).unwrap();
            let ticket = TrashTicket {
                location: Some(trash_dir.join("files/x.txt")),
                info: Some(info),
                trashed_at: Timestamp(0),
            };
            restore_freedesktop(&ticket, &t.join("sub/x.txt")).unwrap();
            assert_eq!(std::fs::read(t.join("sub/x.txt")).unwrap(), b"x");
        }
    }
}
