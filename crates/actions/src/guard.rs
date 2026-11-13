//! The root allowlist and path checks enforced inside the action engine (§10.3
//! of the brief). Owner: W2-E (Wave 1 ships the lexical checks for real).

use std::path::{Path, PathBuf};

use mj_core::pathutil::{is_within, normalize_lexical};
use mj_core::{EntryKind, Error, Result};
use mj_platform::ScanPlatform;

use crate::protected::ProtectedPaths;

/// Decides whether a path may be changed.
#[derive(Debug, Clone)]
pub struct Guard {
    allowed_roots: Vec<PathBuf>,
    protected: ProtectedPaths,
}

impl Guard {
    /// `allowed_roots` must be the scan roots of the current scope (never `/`, a drive root
    /// is allowed only when the user scanned it). An empty list allows nothing.
    pub fn new(allowed_roots: Vec<PathBuf>, protected: ProtectedPaths) -> Self {
        Guard { allowed_roots: allowed_roots.iter().map(|r| normalize_lexical(r)).collect(), protected }
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
        if !path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(Error::InvalidInput(format!("not a plain absolute path: {}", path.display())));
        }
        let root = self.root_of(path).ok_or_else(|| Error::OutsideAllowedRoots(path.to_path_buf()))?;
        if mj_core::pathutil::path_key(root) == mj_core::pathutil::path_key(path) {
            return Err(Error::Protected(path.to_path_buf()));
        }
        if self.protected.is_protected(path) {
            return Err(Error::Protected(path.to_path_buf()));
        }
        Ok(())
    }

    /// Lexical checks plus: no ancestor between the allowed root and `path` is a link or
    /// junction (so the action cannot be redirected outside the root). The item itself may
    /// be a link only when `allow_link_itself` (removing a broken link removes the link).
    pub fn check(&self, path: &Path, platform: &dyn ScanPlatform, allow_link_itself: bool) -> Result<()> {
        self.check_lexical(path)?;
        let root = self.root_of(path).map(Path::to_path_buf).unwrap_or_default();
        let mut cur = path.parent();
        while let Some(p) = cur {
            if p.components().count() <= root.components().count() {
                break;
            }
            if platform.metadata(p)?.kind == EntryKind::Link {
                return Err(Error::LinkNotFollowed(p.to_path_buf()));
            }
            cur = p.parent();
        }
        if !allow_link_itself && platform.metadata(path)?.kind == EntryKind::Link {
            return Err(Error::LinkNotFollowed(path.to_path_buf()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
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
}
