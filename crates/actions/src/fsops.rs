//! File operations every action is built from. Owner: W2-E.
//!
//! - Moves are a same-volume rename **without replace** when possible; across
//!   volumes they become copy + verify (size and xxh64 of what was written,
//!   re-read from disk) + fsync + delete of the source, and the source is only
//!   deleted after the copy verified.
//! - Nothing here ever replaces an existing path: new files are created with
//!   `create_new`, renames use `rename_no_replace`.
//! - Links inside folders are never followed: they are recreated as links
//!   (Unix) or the copy is refused (Windows), and removal uses
//!   `remove_dir_all`, which does not follow links.
//! - Recursion over folders is iterative with a depth limit.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use mj_core::hash::{to_hex, Sha256, Xxh64};
use mj_core::{Error, FileStamp, Result, Timestamp};
use mj_platform::action::{is_cross_device, rename_no_replace_io, sync_dir};

const BUF: usize = 1 << 20;
/// Folder depth limit for copies (hostile trees).
const MAX_DEPTH: usize = 4096;

/// Hashes of one file's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentHash {
    pub size: u64,
    pub xxh64: u64,
    pub sha256: String,
}

/// Stream a file once, computing xxh64 and SHA-256.
pub fn hash_file(path: &Path) -> Result<ContentHash> {
    let mut f = File::open(path).map_err(|e| Error::io(path, e))?;
    let mut x = Xxh64::new(0);
    let mut s = Sha256::new();
    let mut buf = vec![0u8; BUF];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| Error::io(path, e))?;
        if n == 0 {
            break;
        }
        x.update(&buf[..n]);
        s.update(&buf[..n]);
        size += n as u64;
    }
    Ok(ContentHash { size, xxh64: x.finish(), sha256: s.finish_hex() })
}

/// Byte-compare two files; returns `Some(sha256 of a)` when equal, `None` when not.
pub fn compare_files(a: &Path, b: &Path) -> Result<Option<String>> {
    let mut fa = File::open(a).map_err(|e| Error::io(a, e))?;
    let mut fb = File::open(b).map_err(|e| Error::io(b, e))?;
    let la = fa.metadata().map_err(|e| Error::io(a, e))?.len();
    let lb = fb.metadata().map_err(|e| Error::io(b, e))?.len();
    if la != lb {
        return Ok(None);
    }
    let mut s = Sha256::new();
    let mut ba = vec![0u8; BUF];
    let mut bb = vec![0u8; BUF];
    loop {
        let n = read_full(&mut fa, &mut ba).map_err(|e| Error::io(a, e))?;
        let m = read_full(&mut fb, &mut bb).map_err(|e| Error::io(b, e))?;
        if n != m || ba[..n] != bb[..m] {
            return Ok(None);
        }
        if n == 0 {
            return Ok(Some(s.finish_hex()));
        }
        s.update(&ba[..n]);
    }
}

fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = f.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

/// G-02: the identity of one source entry at the moment it was copied: kind, size,
/// mtime, and on Unix the device, inode and change time (any write, truncate, rename over
/// or permission change moves the change time), plus a link's target. A cross-volume move
/// deletes a source entry only while its fresh signature still equals this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySig {
    kind: char,
    size: u64,
    mtime: Option<std::time::SystemTime>,
    extra: (u64, u64, i64, i64),
    link: Option<PathBuf>,
}

/// The signature of `path` (never follows a link).
pub fn entry_sig(path: &Path) -> std::io::Result<EntrySig> {
    let md = fs::symlink_metadata(path)?;
    let ft = md.file_type();
    let kind = if ft.is_symlink() {
        'l'
    } else if ft.is_dir() {
        'd'
    } else if ft.is_file() {
        'f'
    } else {
        'o'
    };
    #[cfg(unix)]
    let extra = {
        use std::os::unix::fs::MetadataExt;
        (md.dev(), md.ino(), md.ctime(), md.ctime_nsec())
    };
    #[cfg(windows)]
    let extra = {
        use std::os::windows::fs::MetadataExt;
        (md.creation_time(), u64::from(md.file_attributes()), 0, 0)
    };
    #[cfg(not(any(unix, windows)))]
    let extra = (0, 0, 0, 0);
    Ok(EntrySig {
        kind,
        size: if kind == 'f' { md.len() } else { 0 },
        mtime: if kind == 'd' { None } else { md.modified().ok() },
        extra: if kind == 'd' { (0, 0, 0, 0) } else { extra },
        link: if kind == 'l' { fs::read_link(path).ok() } else { None },
    })
}

/// G-02: what a verified copy holds, keyed by the path relative to the copied item (`""`
/// for a single file), with each entry's signature as it was when it was copied.
pub type CopyManifest = std::collections::BTreeMap<PathBuf, EntrySig>;

/// Size and mtime of `path` (lstat).
pub fn stamp_of(path: &Path) -> Result<FileStamp> {
    let md = fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    Ok(FileStamp {
        size: if md.is_file() { md.len() } else { 0 },
        mtime: md.modified().map(Timestamp::from_system_time).unwrap_or_default(),
    })
}

/// True when something (file, folder or link, even broken) exists at `path`.
pub fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Permissions of `path` without following links (Unix mode bits; Windows 1 = read-only).
pub fn file_mode(path: &Path) -> Option<u32> {
    let md = std::fs::symlink_metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(md.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        Some(u32::from(md.permissions().readonly()))
    }
}

/// Restore permissions recorded by [`file_mode`] (best effort).
pub fn set_file_mode(path: &Path, mode: u32) -> Result<()> {
    let mut perms = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(mode & 0o7777);
    }
    #[cfg(not(unix))]
    {
        perms.set_readonly(mode & 1 == 1);
    }
    std::fs::set_permissions(path, perms).map_err(|e| Error::io(path, e))
}

/// Set a file's modification time.
pub fn set_mtime(path: &Path, t: Timestamp) -> Result<()> {
    let f = OpenOptions::new().write(true).open(path).map_err(|e| Error::io(path, e))?;
    f.set_modified(t.to_system_time()).map_err(|e| Error::io(path, e))
}

/// A free sibling name: `name.ext`, `name (1).ext`, ... (lstat existence; Windows reserved
/// stems get a trailing `_`, on every platform).
pub fn unique_path(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    if let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) {
        if mj_core::pathutil::has_invalid_windows_name(&name) {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            let clean: String =
                stem.chars()
                    .map(|c| {
                        if mj_core::pathutil::WINDOWS_INVALID_CHARS.contains(&c) || (c as u32) < 0x20 {
                            '_'
                        } else {
                            c
                        }
                    })
                    .collect();
            let clean = clean.trim_end_matches(['.', ' ']).to_string();
            p = path.with_file_name(format!("{clean}_{ext}"));
        }
    }
    mj_core::pathutil::unique_name(&p, exists)
}

/// Create the missing ancestors of `dir` (and `dir`), returning those created, top-down.
pub fn create_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut cur = Some(dir);
    while let Some(p) = cur {
        if exists(p) {
            break;
        }
        missing.push(p.to_path_buf());
        cur = p.parent();
    }
    missing.reverse();
    for p in &missing {
        match fs::create_dir(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::io(p, e)),
        }
    }
    if let Some(parent) = missing.first().and_then(|p| p.parent()) {
        let _ = sync_dir(parent);
    }
    Ok(missing)
}

/// Remove folders that are empty, deepest first (errors ignored: non-empty stays).
pub fn remove_empty_dirs(dirs: &[PathBuf]) {
    for d in dirs.iter().rev() {
        let _ = fs::remove_dir(d);
    }
}

/// Copy one regular file to `dst` (must not exist), preserving mtime and permissions,
/// fsync it, and verify what is on disk (size + xxh64). Returns the source's hashes.
pub fn copy_file_verified(src: &Path, dst: &Path) -> Result<ContentHash> {
    let smd = fs::symlink_metadata(src).map_err(|e| Error::io(src, e))?;
    if !smd.is_file() {
        return Err(Error::InvalidInput(format!("not a regular file: {}", src.display())));
    }
    let mut input = File::open(src).map_err(|e| Error::io(src, e))?;
    // Sibling of F-16: `dst` is removed on failure only once this call created it; an
    // existing file there is someone else's and is never touched.
    let out = OpenOptions::new().write(true).create_new(true).open(dst).map_err(|e| Error::io(dst, e))?;
    let before = entry_sig(src).map_err(|e| Error::io(src, e))?;
    let result = copy_file_inner(src, &smd, &mut input, out, dst).and_then(|h| {
        // G-02: a file written while it was being read gives a copy that is neither the
        // old nor the new content: refuse, the source stays as it is.
        match entry_sig(src) {
            Ok(after) if after == before && after.size == h.size => Ok(h),
            Ok(_) => Err(Error::Changed(src.to_path_buf())),
            Err(e) => Err(Error::io(src, e)),
        }
    });
    if result.is_err() {
        let _ = fs::remove_file(dst);
    }
    result
}

/// [`copy_file_verified`] that also returns the source's signature at copy time (G-02).
fn copy_file_recorded(src: &Path, dst: &Path) -> Result<EntrySig> {
    let sig = entry_sig(src).map_err(|e| Error::io(src, e))?;
    copy_file_verified(src, dst)?;
    // `copy_file_verified` already compared the signature before and after the read.
    Ok(sig)
}

fn copy_file_inner(src: &Path, smd: &fs::Metadata, input: &mut File, mut out: File, dst: &Path) -> Result<ContentHash> {
    let mut x = Xxh64::new(0);
    let mut s = Sha256::new();
    let mut buf = vec![0u8; BUF];
    let mut size = 0u64;
    loop {
        let n = input.read(&mut buf).map_err(|e| Error::io(src, e))?;
        if n == 0 {
            break;
        }
        x.update(&buf[..n]);
        s.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| Error::io(dst, e))?;
        size += n as u64;
    }
    if let Ok(m) = smd.modified() {
        out.set_modified(m).map_err(|e| Error::io(dst, e))?;
    }
    out.sync_all().map_err(|e| Error::io(dst, e))?;
    drop(out);
    fs::set_permissions(dst, smd.permissions()).map_err(|e| Error::io(dst, e))?;
    let src_hash = ContentHash { size, xxh64: x.finish(), sha256: s.finish_hex() };
    let back = hash_file(dst)?;
    if back.size != src_hash.size || back.xxh64 != src_hash.xxh64 {
        return Err(Error::Io {
            path: Some(dst.to_path_buf()),
            kind: "invalid_data",
            message: "the copy does not match the original (verification failed)".into(),
        });
    }
    if let Some(parent) = dst.parent() {
        let _ = sync_dir(parent);
    }
    Ok(src_hash)
}

/// Copy a file, folder or link to `dst` (must not exist), verifying every file. Links are
/// recreated as links, never followed. On failure everything written is removed.
pub fn copy_item_verified(src: &Path, dst: &Path) -> Result<()> {
    copy_item_recorded(src, dst).map(|_| ())
}

/// [`copy_item_verified`] returning the manifest of what was copied (G-02).
pub fn copy_item_recorded(src: &Path, dst: &Path) -> Result<CopyManifest> {
    let md = fs::symlink_metadata(src).map_err(|e| Error::io(src, e))?;
    let mut manifest = CopyManifest::new();
    if md.file_type().is_symlink() {
        let sig = entry_sig(src).map_err(|e| Error::io(src, e))?;
        copy_link(src, dst)?;
        manifest.insert(PathBuf::new(), sig);
        return Ok(manifest);
    }
    if md.is_file() {
        manifest.insert(PathBuf::new(), copy_file_recorded(src, dst)?);
        return Ok(manifest);
    }
    // Sibling of F-16: the top folder is created here; only once it is ours is a failed
    // copy removed again (a folder that already existed is never touched).
    fs::create_dir(dst).map_err(|e| Error::io(dst, e))?;
    let r = copy_tree(src, dst, &mut manifest);
    if let Err(e) = r {
        let _ = fs::remove_dir_all(dst);
        return Err(e);
    }
    Ok(manifest)
}

fn copy_link(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let target = fs::read_link(src).map_err(|e| Error::io(src, e))?;
        std::os::unix::fs::symlink(target, dst).map_err(|e| Error::io(dst, e))
    }
    #[cfg(not(unix))]
    {
        let _ = dst;
        Err(Error::unsupported("Copy", format!("{} is a link; links are not copied across volumes", src.display())))
    }
}

fn copy_tree(src: &Path, dst: &Path, manifest: &mut CopyManifest) -> Result<()> {
    // (source dir, destination dir, depth); folder mtimes/permissions applied afterwards.
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf(), 0usize)];
    let mut dirs = Vec::new();
    while let Some((s, d, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(Error::InvalidInput(format!("folder nesting too deep at {}", s.display())));
        }
        let smd = fs::symlink_metadata(&s).map_err(|e| Error::io(&s, e))?;
        dirs.push((d.clone(), smd));
        for e in fs::read_dir(&s).map_err(|e| Error::io(&s, e))? {
            let e = e.map_err(|e| Error::io(&s, e))?;
            let sp = e.path();
            let dp = d.join(e.file_name());
            let ft = e.file_type().map_err(|e| Error::io(&sp, e))?;
            let rel = sp.strip_prefix(src).map(Path::to_path_buf).unwrap_or_else(|_| sp.clone());
            if ft.is_symlink() {
                let sig = entry_sig(&sp).map_err(|e| Error::io(&sp, e))?;
                copy_link(&sp, &dp)?;
                manifest.insert(rel, sig);
            } else if ft.is_dir() {
                fs::create_dir(&dp).map_err(|e| Error::io(&dp, e))?;
                manifest.insert(rel, entry_sig(&sp).map_err(|e| Error::io(&sp, e))?);
                stack.push((sp, dp, depth + 1));
            } else if ft.is_file() {
                manifest.insert(rel, copy_file_recorded(&sp, &dp)?);
            } else {
                return Err(Error::unsupported("Copy", format!("{} is a special file", sp.display())));
            }
        }
    }
    for (d, smd) in dirs.iter().rev() {
        if let Ok(m) = smd.modified() {
            if let Ok(f) = File::open(d) {
                let _ = f.set_modified(m);
            }
        }
        let _ = fs::set_permissions(d, smd.permissions());
        let _ = sync_dir(d);
    }
    Ok(())
}

/// Remove a file, link or folder tree (links are removed, never followed).
pub fn remove_item(path: &Path) -> Result<()> {
    let md = fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    let r = if md.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
    r.map_err(|e| Error::io(path, e))?;
    if let Some(parent) = path.parent() {
        let _ = sync_dir(parent);
    }
    Ok(())
}

/// How a move was carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveHow {
    Renamed,
    Copied,
    /// F-09: across volumes, the complete copy at the destination verified, but part of
    /// the source could not be deleted afterwards: `left` entries remain at the source.
    /// The destination holds everything; the caller records the move as done (the copy
    /// is what undo restores) and names what was left behind.
    CopiedSourceLeft {
        left: u64,
    },
}

/// Move `src` to `dst` (must not exist): rename on one volume, else copy + verify + fsync +
/// delete. The source is only removed after the copy verified.
pub fn move_item(src: &Path, dst: &Path) -> Result<MoveHow> {
    move_item_with(src, dst, false, &|p, dir| if dir { fs::remove_dir(p) } else { fs::remove_file(p) })
}

/// [`move_item`] with the cross-volume path forced (`force_copy`) and the removal of each
/// source entry injectable (tests).
pub fn move_item_with(
    src: &Path,
    dst: &Path,
    force_copy: bool,
    remove: &dyn Fn(&Path, bool) -> std::io::Result<()>,
) -> Result<MoveHow> {
    move_item_hooked(src, dst, force_copy, &|| {}, remove)
}

/// [`move_item_with`] with a hook that runs after the copy verified and before the source
/// is compared with it (tests: something writes into the source at that moment).
pub fn move_item_hooked(
    src: &Path,
    dst: &Path,
    force_copy: bool,
    after_copy: &dyn Fn(),
    remove: &dyn Fn(&Path, bool) -> std::io::Result<()>,
) -> Result<MoveHow> {
    if force_copy {
        if exists(dst) {
            return Err(Error::Conflict(format!("{} already exists; nothing was overwritten", dst.display())));
        }
        return copy_then_remove(src, dst, after_copy, remove);
    }
    match rename_no_replace_io(src, dst) {
        Ok(()) => {
            if let Some(p) = dst.parent() {
                let _ = sync_dir(p);
            }
            if let Some(p) = src.parent() {
                let _ = sync_dir(p);
            }
            Ok(MoveHow::Renamed)
        }
        Err(e) if is_cross_device(&e) => copy_then_remove(src, dst, after_copy, remove),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(Error::Conflict(format!("{} already exists; nothing was overwritten", dst.display())))
        }
        Err(e) => Err(Error::io(src, e)),
    }
}

/// The cross-volume half of a move: copy, verify (size + xxh64 re-read) and fsync
/// everything first (a failure there removes the partial copy and leaves the source
/// untouched); then (G-02) compare the whole source with what was copied, and when anything
/// changed, appeared or disappeared meanwhile remove our own copy again and refuse
/// (`changed`: the source is complete and untouched); else delete the source entry by entry,
/// each only while it is still exactly what was copied.
fn copy_then_remove(
    src: &Path,
    dst: &Path,
    after_copy: &dyn Fn(),
    remove: &dyn Fn(&Path, bool) -> std::io::Result<()>,
) -> Result<MoveHow> {
    let manifest = copy_item_recorded(src, dst)?;
    after_copy();
    if !source_matches(src, &manifest) {
        // The copy is ours (created by this call, verified) and the source is complete:
        // removing the copy loses nothing, keeping it would leave a stale duplicate.
        let _ = remove_item(dst);
        return Err(Error::Changed(src.to_path_buf()));
    }
    Ok(remove_source(src, &manifest, remove))
}

/// G-02: true when every entry below `src` (and `src` itself) is in `manifest` with the
/// same signature, and nothing in `manifest` is missing.
fn source_matches(src: &Path, manifest: &CopyManifest) -> bool {
    let Ok(top) = fs::symlink_metadata(src) else { return false };
    if !top.is_dir() {
        return manifest.len() == 1 && manifest.get(Path::new("")) == entry_sig(src).ok().as_ref();
    }
    let mut seen = 0usize;
    let mut stack = vec![(src.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return false;
        }
        let Ok(rd) = fs::read_dir(&d) else { return false };
        for e in rd {
            let Ok(e) = e else { return false };
            let p = e.path();
            let Ok(rel) = p.strip_prefix(src) else { return false };
            let Ok(sig) = entry_sig(&p) else { return false };
            if manifest.get(rel) != Some(&sig) {
                return false;
            }
            seen += 1;
            if sig.kind == 'd' {
                stack.push((p, depth + 1));
            }
        }
    }
    seen == manifest.len()
}

/// F-09: delete the source of a verified cross-volume copy entry by entry, deepest first,
/// carrying on past failures so as little as possible is left in two places; links are
/// removed, never followed. G-02: an entry is deleted only while its fresh signature is
/// still the one recorded when it was copied; anything else (changed, or not in the copy
/// at all) is left and counted, and a folder is removed only when empty.
fn remove_source(src: &Path, manifest: &CopyManifest, remove: &dyn Fn(&Path, bool) -> std::io::Result<()>) -> MoveHow {
    let mut left = 0u64;
    let unchanged = |p: &Path| {
        let rel = p.strip_prefix(src).unwrap_or(p);
        match (entry_sig(p), manifest.get(rel)) {
            (Ok(now), Some(was)) => now == *was,
            _ => false,
        }
    };
    let is_dir = fs::symlink_metadata(src).map(|m| m.is_dir()).unwrap_or(false);
    if !is_dir {
        if !unchanged(src) || remove(src, false).is_err() {
            left += 1;
        }
    } else {
        // Post-order: (path, children pushed?)
        let mut stack = vec![(src.to_path_buf(), false, 0usize)];
        while let Some((p, expanded, depth)) = stack.pop() {
            if expanded {
                let rel = p.strip_prefix(src).unwrap_or(&p);
                let ours = rel.as_os_str().is_empty() || manifest.contains_key(rel);
                if !ours || remove(&p, true).is_err() {
                    left += 1;
                }
                continue;
            }
            stack.push((p.clone(), true, depth));
            let Ok(rd) = fs::read_dir(&p) else { continue };
            for e in rd.flatten() {
                let cp = e.path();
                let dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if dir && depth < MAX_DEPTH {
                    stack.push((cp, false, depth + 1));
                } else if !unchanged(&cp) || remove(&cp, false).is_err() {
                    left += 1;
                }
            }
        }
    }
    if let Some(parent) = src.parent() {
        let _ = sync_dir(parent);
    }
    if left == 0 {
        MoveHow::Copied
    } else {
        MoveHow::CopiedSourceLeft { left }
    }
}

/// The note journalled when a cross-volume move left part of its source behind.
pub fn left_behind_note(src: &Path, dst: &Path, left: u64) -> Error {
    Error::Conflict(format!(
        "moved by copying: the complete copy is at {}, but {left} item(s) could not be deleted at {}; \
         remove them there by hand (Restore needs that place to be free)",
        dst.display(),
        src.display()
    ))
}

/// Number of outputs made durable by [`sync_output`] in this process (diagnostics and
/// tests: proves the engine synced an output before it moved an original away).
pub static SYNCED_OUTPUTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// F-11: flush a new output file to disk (`fsync`), so that a power cut after the original
/// was moved away or replaced cannot leave an empty or truncated file under a `done`
/// journal record. Failure is an error: the original then stays where it is.
pub fn sync_output(path: &Path) -> Result<()> {
    // Opened for writing (never truncated): Windows flushes only a writable handle.
    let f = OpenOptions::new().write(true).open(path).map_err(|e| Error::io(path, e))?;
    f.sync_all().map_err(|e| Error::io(path, e))?;
    SYNCED_OUTPUTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Total size of the regular files of a file or folder (links count 0; bounded depth).
pub fn tree_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![(path.to_path_buf(), 0usize)];
    while let Some((p, depth)) = stack.pop() {
        let Ok(md) = fs::symlink_metadata(&p) else { continue };
        if md.is_file() {
            total = total.saturating_add(md.len());
        } else if md.is_dir() && depth < MAX_DEPTH {
            if let Ok(rd) = fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| (e.path(), depth + 1)));
            }
        }
    }
    total
}

/// Hex of an xxh64.
pub fn hex64(v: u64) -> String {
    to_hex(&v.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn copy_verify_and_compare() {
        let t = TempDir::new("fsops").unwrap();
        let a = t.write("a.bin", &vec![7u8; 3 * BUF + 5]).unwrap();
        let b = t.join("b.bin");
        let h = copy_file_verified(&a, &b).unwrap();
        assert_eq!(h, hash_file(&b).unwrap());
        assert_eq!(stamp_of(&a).unwrap(), stamp_of(&b).unwrap(), "mtime preserved");
        assert!(compare_files(&a, &b).unwrap().is_some());
        assert!(copy_file_verified(&a, &b).is_err(), "never overwrites");
        let c = t.write("c.bin", &vec![7u8; 3 * BUF + 4]).unwrap();
        assert!(compare_files(&a, &c).unwrap().is_none());
        let mut d = vec![7u8; 3 * BUF + 5];
        d[BUF + 3] = 8;
        let d = t.write("d.bin", &d).unwrap();
        assert!(compare_files(&a, &d).unwrap().is_none());
    }

    #[test]
    fn tree_copy_keeps_links_as_links() {
        let t = TempDir::new("fsops-tree").unwrap();
        t.write("src/a/b.txt", b"b").unwrap();
        t.write("src/c.txt", b"c").unwrap();
        t.write("outside/secret", b"s").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(t.join("outside"), t.join("src/a/link")).unwrap();
        copy_item_verified(&t.join("src"), &t.join("dst")).unwrap();
        assert_eq!(
            mj_core::testutil::snapshot(&t.join("src")).unwrap(),
            mj_core::testutil::snapshot(&t.join("dst")).unwrap()
        );
        remove_item(&t.join("dst")).unwrap();
        assert!(t.join("outside/secret").exists(), "removal never follows links");
        assert_eq!(tree_size(&t.join("src")), 2);
    }

    #[test]
    fn unique_and_dirs() {
        let t = TempDir::new("fsops-uniq").unwrap();
        t.write("x.txt", b"").unwrap();
        assert_eq!(unique_path(&t.join("x.txt")), t.join("x (1).txt"));
        assert_eq!(unique_path(&t.join("con.txt")), t.join("con_.txt"));
        let made = create_dirs(&t.join("p/q/r")).unwrap();
        assert_eq!(made, [t.join("p"), t.join("p/q"), t.join("p/q/r")]);
        remove_empty_dirs(&made);
        assert!(!t.join("p").exists());
    }
}
