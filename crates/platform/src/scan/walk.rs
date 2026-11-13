//! Directory listing and a simple sequential walk. Owner: W2-A.
//! The parallel production walker lives in `mj-scan`; this is the primitive.

use std::path::Path;

use mj_core::{CancelToken, EntryKind, Error, Result};

use crate::traits::DirItem;

/// List one directory without following links.
pub fn read_dir(dir: &Path) -> Result<Vec<DirItem>> {
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
