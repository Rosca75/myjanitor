//! The root allowlist and path checks enforced inside the action engine (§10.3
//! of the brief). Owner: W2-E.
//!
//! [`Guard::check`] refuses, in this order:
//!
//! 1. a path that is not absolute or has a `..` component (`invalid_input`);
//! 2. a path outside every allowed root (`outside_allowed_roots`), or an allowed
//!    root itself (`protected`): a root is never acted on;
//! 3. a protected path (`protected`), lexically and after canonicalisation;
//! 4. a link, junction or mount point on any ancestor between the root and the
//!    item (`link_not_followed`); the item itself may be a link only when the
//!    caller removes the link itself;
//! 5. a canonical parent that escapes the canonical root (defence in depth
//!    against links the lstat walk could not see, e.g. a race).
//!
//! The final component is never resolved: a link is judged as a link.
//! [`Guard::check_item`] adds the TOCTOU stamp check against the scan.

use std::path::{Component, Path, PathBuf};

use mj_core::pathutil::{is_within, normalize_lexical, path_key};
use mj_core::{ActionItem, EntryKind, Error, FileEntry, FileStamp, Result, TargetKind};
use mj_platform::ScanPlatform;

use crate::protected::ProtectedPaths;

/// Decides whether a path may be changed.
#[derive(Debug, Clone)]
pub struct Guard {
    allowed_roots: Vec<PathBuf>,
    protected: ProtectedPaths,
}

fn plain_absolute(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(Error::InvalidInput(format!("not a plain absolute path: {}", path.display())));
    }
    Ok(())
}

impl Guard {
    /// `allowed_roots` must be the scan roots of the current scope (never `/`, a drive root
    /// is allowed only when the user scanned it). An empty list allows nothing.
    pub fn new(allowed_roots: Vec<PathBuf>, protected: ProtectedPaths) -> Self {
        Guard {
            allowed_roots: allowed_roots.iter().filter(|r| r.is_absolute()).map(|r| normalize_lexical(r)).collect(),
            protected,
        }
    }

    /// The allowed roots.
    pub fn allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }

    /// The protected-path set.
    pub fn protected(&self) -> &ProtectedPaths {
        &self.protected
    }

    /// The allowed root containing `path`.
    pub fn root_of(&self, path: &Path) -> Option<&Path> {
        self.allowed_roots
            .iter()
            .filter(|r| is_within(r, path))
            .max_by_key(|r| r.components().count())
            .map(|r| r.as_path())
    }

    /// Lexical checks only: absolute, no `..`, inside an allowed root, not the root
    /// itself, not protected.
    pub fn check_lexical(&self, path: &Path) -> Result<()> {
        plain_absolute(path)?;
        let root = self.root_of(path).ok_or_else(|| Error::OutsideAllowedRoots(path.to_path_buf()))?;
        if path_key(root) == path_key(path) {
            return Err(Error::Protected(path.to_path_buf()));
        }
        if self.protected.is_protected(path) {
            return Err(Error::Protected(path.to_path_buf()));
        }
        Ok(())
    }

    /// Lexical checks plus: no ancestor between the allowed root and `path` is a link,
    /// junction or mount point (so the action cannot be redirected outside the root), and
    /// the canonical parent stays inside the canonical root. The item itself may be a link
    /// only when `allow_link_itself` (removing a broken link removes the link).
    pub fn check(&self, path: &Path, platform: &dyn ScanPlatform, allow_link_itself: bool) -> Result<()> {
        self.check_lexical(path)?;
        let root = self.root_of(path).map(Path::to_path_buf).unwrap_or_default();
        // A root that is itself a link is resolved by the canonical check below; links
        // above the root are the user's choice (they picked the location).
        let root_md = platform.metadata(&root)?;
        let root_volume = if root_md.kind == EntryKind::Link { None } else { root_md.file_id.map(|f| f.volume) };
        let mut cur = path.parent();
        while let Some(p) = cur {
            if p.components().count() <= root.components().count() {
                break;
            }
            let md = platform.metadata(p)?;
            if md.kind == EntryKind::Link {
                return Err(Error::LinkNotFollowed(p.to_path_buf()));
            }
            if md.kind != EntryKind::Dir {
                return Err(Error::InvalidInput(format!("not a folder: {}", p.display())));
            }
            if let (Some(rv), Some(v)) = (root_volume, md.file_id.map(|f| f.volume)) {
                if rv != v {
                    // A mount point below the root: another volume was mounted into the tree.
                    return Err(Error::LinkNotFollowed(p.to_path_buf()));
                }
            }
            cur = p.parent();
        }
        let md = platform.metadata(path)?;
        if !allow_link_itself && md.kind == EntryKind::Link {
            return Err(Error::LinkNotFollowed(path.to_path_buf()));
        }
        self.check_canonical(path, &root)?;
        Ok(())
    }

    /// The canonical parent (links resolved) must stay below the canonical root, and the
    /// canonical form must not be protected either. The final component is not resolved.
    fn check_canonical(&self, path: &Path, root: &Path) -> Result<()> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(Error::InvalidInput(format!("no parent: {}", path.display())));
        };
        let croot = std::fs::canonicalize(root).map_err(|e| Error::io(root, e))?;
        let cparent = std::fs::canonicalize(parent).map_err(|e| Error::io(parent, e))?;
        let cpath = cparent.join(name);
        if !is_within(&croot, &cpath) {
            return Err(Error::LinkNotFollowed(path.to_path_buf()));
        }
        if path_key(&croot) == path_key(&cpath) {
            return Err(Error::Protected(path.to_path_buf()));
        }
        if self.protected.is_protected(&cpath) {
            return Err(Error::Protected(path.to_path_buf()));
        }
        Ok(())
    }

    /// Every check for one action item, plus the TOCTOU stamp check. Returns the fresh
    /// metadata. `allow_link_itself` is for removals of the link itself.
    pub fn check_item(
        &self,
        item: &ActionItem,
        platform: &dyn ScanPlatform,
        allow_link_itself: bool,
    ) -> Result<FileEntry> {
        self.check(&item.path, platform, allow_link_itself)?;
        if !item.root.as_os_str().is_empty() && !is_within(&item.root, &item.path) {
            return Err(Error::InvalidInput(format!(
                "{} is not below its scan root {}",
                item.path.display(),
                item.root.display()
            )));
        }
        let md = platform.metadata(&item.path)?;
        match (item.target, md.kind) {
            (TargetKind::File, EntryKind::File) | (TargetKind::Folder, EntryKind::Dir) => {}
            (_, EntryKind::Link) if allow_link_itself => {}
            _ => return Err(Error::Changed(item.path.clone())),
        }
        Self::compare_stamp(item, &md)?;
        Ok(md)
    }

    /// Re-stat `item.path` right before acting and compare with the scan stamp.
    pub fn restat(&self, item: &ActionItem, platform: &dyn ScanPlatform) -> Result<FileEntry> {
        let md = platform.metadata(&item.path)?;
        Self::compare_stamp(item, &md)?;
        Ok(md)
    }

    fn compare_stamp(item: &ActionItem, md: &FileEntry) -> Result<()> {
        if item.target == TargetKind::File && md.kind != EntryKind::Dir && !stamp_matches(item.expected, md) {
            return Err(Error::Changed(item.path.clone()));
        }
        Ok(())
    }

    /// A destination folder for move / copy / backup: absolute, no `..`, not protected
    /// (lexically and in its canonical form, as far as it exists), and not inside the item.
    pub fn check_destination(&self, dest: &Path, item: &Path) -> Result<()> {
        plain_absolute(dest)?;
        if self.protected.is_protected(dest) {
            return Err(Error::Protected(dest.to_path_buf()));
        }
        if is_within(item, dest) {
            return Err(Error::InvalidInput(format!("{} is inside {}", dest.display(), item.display())));
        }
        let mut existing = Some(dest);
        while let Some(p) = existing {
            if let Ok(c) = std::fs::canonicalize(p) {
                let rest = dest.strip_prefix(p).unwrap_or(Path::new(""));
                let full = c.join(rest);
                if self.protected.is_protected(&full) {
                    return Err(Error::Protected(dest.to_path_buf()));
                }
                if let Ok(ci) = std::fs::canonicalize(item) {
                    if is_within(&ci, &full) {
                        return Err(Error::InvalidInput(format!("{} is inside {}", dest.display(), item.display())));
                    }
                }
                break;
            }
            existing = p.parent();
        }
        Ok(())
    }
}

/// True when the fresh metadata still has the scanned size and mtime.
pub fn stamp_matches(expected: FileStamp, md: &FileEntry) -> bool {
    md.size == expected.size && md.modified.unwrap_or_default() == expected.mtime
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::Action;
    use mj_platform::NativePlatform;

    #[test]
    fn allowlist_and_links() {
        let t = TempDir::new("guard").unwrap();
        let f = t.write("in/a.txt", b"x").unwrap();
        let g = Guard::new(vec![t.join("in")], ProtectedPaths::empty());
        let p = NativePlatform::new();
        assert!(g.check(&f, &p, false).is_ok());
        assert_eq!(g.check(&t.join("in"), &p, false).unwrap_err().code(), "protected");
        assert_eq!(g.check_lexical(&t.join("out.txt")).unwrap_err().code(), "outside_allowed_roots");
        assert_eq!(g.check_lexical(&t.join("in/../out.txt")).unwrap_err().code(), "invalid_input");
        assert_eq!(g.check_lexical(Path::new("in/a.txt")).unwrap_err().code(), "invalid_input");
        assert_eq!(g.check_lexical(&t.join("in2/a.txt")).unwrap_err().code(), "outside_allowed_roots");
        assert_eq!(
            Guard::new(vec![], ProtectedPaths::empty()).check_lexical(&f).unwrap_err().code(),
            "outside_allowed_roots"
        );
        #[cfg(unix)]
        {
            t.write("elsewhere/b.txt", b"y").unwrap();
            std::os::unix::fs::symlink(t.join("elsewhere"), t.join("in/link")).unwrap();
            assert_eq!(g.check(&t.join("in/link/b.txt"), &p, false).unwrap_err().code(), "link_not_followed");
            assert_eq!(g.check(&t.join("in/link"), &p, false).unwrap_err().code(), "link_not_followed");
            assert!(g.check(&t.join("in/link"), &p, true).is_ok());
        }
    }

    #[test]
    fn protected_and_stamp() {
        let t = TempDir::new("guard2").unwrap();
        let f = t.write("in/keep/a.txt", b"x").unwrap();
        let g = Guard::new(vec![t.join("in")], ProtectedPaths::new(&[t.join("in/keep")]));
        let p = NativePlatform::new();
        assert_eq!(g.check(&f, &p, false).unwrap_err().code(), "protected");
        let f2 = t.write("in/b.txt", b"abc").unwrap();
        let md = p.metadata(&f2).unwrap();
        let mut item = ActionItem {
            path: f2.clone(),
            target: TargetKind::File,
            action: Action::Trash,
            link_source: None,
            root: t.join("in"),
            expected: md.stamp(),
        };
        assert!(g.check_item(&item, &p, false).is_ok());
        std::fs::write(&f2, b"abcd").unwrap();
        assert_eq!(g.check_item(&item, &p, false).unwrap_err().code(), "changed");
        item.expected = p.metadata(&f2).unwrap().stamp();
        item.target = TargetKind::Folder;
        assert_eq!(g.check_item(&item, &p, false).unwrap_err().code(), "changed", "kind mismatch");
        item.target = TargetKind::File;
        item.root = t.join("other");
        assert_eq!(g.check_item(&item, &p, false).unwrap_err().code(), "invalid_input");
    }

    #[test]
    fn destinations() {
        let t = TempDir::new("guard3").unwrap();
        let g = Guard::new(vec![t.join("in")], ProtectedPaths::empty().with_app_dirs(&[t.join("q")]));
        assert!(g.check_destination(&t.join("dest/x"), &t.join("in/a")).is_ok());
        assert_eq!(g.check_destination(&t.join("q/x"), &t.join("in/a")).unwrap_err().code(), "protected");
        assert_eq!(g.check_destination(&t.join("in/a/sub"), &t.join("in/a")).unwrap_err().code(), "invalid_input");
        assert_eq!(g.check_destination(Path::new("rel"), &t.join("in/a")).unwrap_err().code(), "invalid_input");
    }
}
