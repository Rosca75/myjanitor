//! Directory listing and a simple sequential walk. Owner: W2-A.
//! The parallel production walker lives in `mj-scan`; these are the primitives.

use std::path::Path;

use mj_core::{CancelToken, EntryKind, Error, Result};

use crate::traits::DirItem;

/// List one directory without following links.
///
/// Linux and other Unix: `std::fs::read_dir` (`getdents64`, entry type from
/// `d_type`, no stat). Windows: `FindFirstFileExW` with `FindExInfoBasic` and
/// `FIND_FIRST_EX_LARGE_FETCH` through a `\\?\` long path; a reparse point is
/// classified by its tag, so junctions and symlinks are `Link` while cloud
/// placeholders stay files or directories.
pub fn read_dir(dir: &Path) -> Result<Vec<DirItem>> {
    native_read_dir(dir)
}

#[cfg(not(windows))]
fn native_read_dir(dir: &Path) -> Result<Vec<DirItem>> {
    let rd = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    let mut out = Vec::new();
    for item in rd {
        let item = item.map_err(|e| Error::io(dir, e))?;
        let kind = match item.file_type() {
            Ok(ft) if ft.is_symlink() => EntryKind::Link,
            Ok(ft) if ft.is_dir() => EntryKind::Dir,
            Ok(ft) if ft.is_file() => EntryKind::File,
            Ok(_) => EntryKind::Other,
            Err(_) => EntryKind::Other,
        };
        out.push(DirItem { name: item.file_name(), path: item.path(), kind });
    }
    Ok(out)
}

#[cfg(windows)]
fn native_read_dir(dir: &Path) -> Result<Vec<DirItem>> {
    use super::metadata::entry_from_win32;
    use super::win::*;
    use crate::ffi::windows::{wide_path, INVALID_HANDLE_VALUE};

    let pattern = dir.join("*");
    let w = wide_path(&pattern);
    let mut fd = WIN32_FIND_DATAW::zeroed();
    // SAFETY: `w` is NUL-terminated and `fd` is a writable WIN32_FIND_DATAW; both outlive the call.
    let h = unsafe {
        FindFirstFileExW(
            w.as_ptr(),
            FIND_EX_INFO_BASIC,
            &mut fd,
            FIND_EX_SEARCH_NAME_MATCH,
            std::ptr::null_mut(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let mut out = Vec::new();
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        let err = last_error();
        // An empty drive root has no "." entry and answers "file not found".
        if err.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) && std::fs::symlink_metadata(dir).is_ok() {
            return Ok(out);
        }
        return Err(Error::io(dir, err));
    }
    let _close = FindHandle(h);
    loop {
        let name = fd.name();
        if name != "." && name != ".." {
            let path = dir.join(&name);
            let e = entry_from_win32(&path, fd.dwFileAttributes, Some(fd.dwReserved0), 0, 0, 0, 0);
            out.push(DirItem { name, path, kind: e.kind });
        }
        // SAFETY: `h` is a valid find handle and `fd` a writable WIN32_FIND_DATAW.
        let ok = unsafe { FindNextFileW(h, &mut fd) };
        if ok == 0 {
            let err = last_error();
            if err.raw_os_error() == Some(ERROR_NO_MORE_FILES) {
                break;
            }
            return Err(Error::io(dir, err));
        }
    }
    Ok(out)
}

/// Depth-first walk below `root` (root excluded), sorted per directory, never
/// descending into links. `visit` returns false to skip descending into a directory.
/// Unreadable directories are passed to `on_error` and skipped.
pub fn walk(
    root: &Path,
    cancel: &CancelToken,
    visit: &mut dyn FnMut(&DirItem, u32) -> bool,
    on_error: &mut dyn FnMut(&Path, Error),
) -> Result<()> {
    let mut stack = vec![(root.to_path_buf(), 1u32)];
    while let Some((dir, depth)) = stack.pop() {
        cancel.checkpoint()?;
        let mut items = match read_dir(&dir) {
            Ok(items) => items,
            Err(e) => {
                on_error(&dir, e);
                continue;
            }
        };
        items.sort_by(|a, b| b.name.cmp(&a.name));
        for item in items {
            let descend = visit(&item, depth);
            if descend && item.kind == EntryKind::Dir {
                stack.push((item.path.clone(), depth + 1));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn walks_everything_but_not_through_links() {
        let t = TempDir::new("walk").unwrap();
        t.write("a/b/c.txt", b"1").unwrap();
        t.write("a/d.txt", b"2").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(t.join("a"), t.join("loop")).unwrap();
        let mut seen = Vec::new();
        walk(
            t.path(),
            &CancelToken::new(),
            &mut |i, d| {
                seen.push((i.name.to_string_lossy().into_owned(), d, i.kind));
                true
            },
            &mut |_, _| {},
        )
        .unwrap();
        assert!(seen.contains(&("c.txt".into(), 3, EntryKind::File)));
        assert!(seen.contains(&("d.txt".into(), 2, EntryKind::File)));
        #[cfg(unix)]
        assert!(seen.contains(&("loop".into(), 1, EntryKind::Link)));
        assert_eq!(seen.iter().filter(|s| s.0 == "c.txt").count(), 1);
    }
}
