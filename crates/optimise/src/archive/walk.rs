//! Enumerate what goes into an archive, never following links.
//!
//! Each source is walked with `symlink_metadata`; a symlink (or Windows
//! junction) is recorded as [`Kind::Link`] and never entered, so it is listed
//! in the index as skipped. Names are stored relative to `base` with `/`
//! separators. Anything that cannot be stored faithfully (non-UTF-8 names,
//! backslashes, sockets/devices, a source outside `base`) is an error: an
//! archive that silently leaves out data must never be produced, because the
//! action engine removes the sources once the archive verifies.

use std::fs;
use std::path::{Component, Path, PathBuf};

use mj_core::{Error, Result, Timestamp};

/// Most entries in one archive.
pub const MAX_ITEMS: usize = 1_000_000;

/// What an item is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    File {
        size: u64,
    },
    Dir,
    /// Not archived; the target text is kept in the index.
    Link {
        target: String,
    },
}

/// One thing found under the sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Relative name with `/` separators (directories without the trailing `/`).
    pub rel: String,
    pub abs: PathBuf,
    pub kind: Kind,
    pub mtime: Timestamp,
    /// Unix permission bits when known.
    pub mode: Option<u32>,
}

impl Item {
    /// Size of a file, 0 otherwise.
    pub fn size(&self) -> u64 {
        match self.kind {
            Kind::File { size } => size,
            _ => 0,
        }
    }
}

/// `rel` of `path` under `base`, `/`-separated, validated.
pub fn relative_name(base: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(base)
        .map_err(|_| Error::InvalidInput(format!("{} is not under {}", path.display(), base.display())))?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => {
                let s = s.to_str().ok_or_else(|| {
                    Error::unsupported("Archiving", format!("{}: name is not valid Unicode", path.display()))
                })?;
                if s.contains('\\') {
                    return Err(Error::unsupported(
                        "Archiving",
                        format!("{}: a backslash in a name would become a folder on Windows", path.display()),
                    ));
                }
                parts.push(s.to_string());
            }
            Component::CurDir => {}
            _ => return Err(Error::InvalidInput(format!("{}: unexpected path component", path.display()))),
        }
    }
    if parts.is_empty() {
        return Err(Error::InvalidInput("the archive base itself cannot be archived".into()));
    }
    Ok(parts.join("/"))
}

fn mode_of(md: &fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(md.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = md;
        None
    }
}

fn item(base: &Path, abs: &Path) -> Result<Item> {
    let md = fs::symlink_metadata(abs).map_err(|e| Error::io(abs, e))?;
    let mtime = md.modified().map(Timestamp::from_system_time).unwrap_or_default();
    let ft = md.file_type();
    let kind = if ft.is_symlink() || is_reparse_dir(&md) {
        let target = fs::read_link(abs).map(|t| t.to_string_lossy().into_owned()).unwrap_or_default();
        Kind::Link { target }
    } else if ft.is_dir() {
        Kind::Dir
    } else if ft.is_file() {
        Kind::File { size: md.len() }
    } else {
        return Err(Error::unsupported(
            "Archiving",
            format!("{}: special file (socket, pipe or device)", abs.display()),
        ));
    };
    Ok(Item { rel: relative_name(base, abs)?, abs: abs.to_path_buf(), kind, mtime, mode: mode_of(&md) })
}

/// Windows junctions and other reparse-point directories are treated as links.
fn is_reparse_dir(md: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        md.is_dir() && md.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = md;
        false
    }
}

/// Every item under `sources` (each source included), sorted by name.
/// Iterative (no recursion depth limit to hit), cancel-aware through `check`.
pub fn collect(sources: &[PathBuf], base: &Path, check: &dyn Fn() -> Result<()>) -> Result<Vec<Item>> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = sources.iter().rev().cloned().collect();
    while let Some(p) = stack.pop() {
        check()?;
        let it = item(base, &p)?;
        if it.kind == Kind::Dir {
            let mut kids: Vec<PathBuf> = fs::read_dir(&p)
                .map_err(|e| Error::io(&p, e))?
                .map(|d| d.map(|d| d.path()).map_err(|e| Error::io(&p, e)))
                .collect::<Result<_>>()?;
            kids.sort();
            stack.extend(kids.into_iter().rev());
        }
        out.push(it);
        if out.len() > MAX_ITEMS {
            return Err(Error::unsupported("Archiving", "more than one million entries"));
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    if out.windows(2).any(|w| w[0].rel == w[1].rel) {
        return Err(Error::InvalidInput("two sources map to the same archive name".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_names() {
        let b = Path::new("/a/b");
        assert_eq!(relative_name(b, Path::new("/a/b/c/d.txt")).unwrap(), "c/d.txt");
        assert!(relative_name(b, Path::new("/a/x")).is_err());
        assert!(relative_name(b, b).is_err());
        #[cfg(unix)]
        assert!(relative_name(b, Path::new("/a/b/c\\d")).is_err());
    }
}
