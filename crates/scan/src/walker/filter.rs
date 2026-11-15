//! Scope rules applied while walking (SCAN-02): exclusions, includes, hidden.
//!
//! Exclusion patterns (scope `exclude` plus the global settings exclusions):
//! - an **absolute path without wildcards** (`/home/me/VMs`, `C:\Temp`) excludes
//!   that entry and everything below it;
//! - a **pattern without a separator** (`node_modules`, `*.tmp`, `.git`) is
//!   matched against the entry's name, gitignore-style, at any depth;
//! - a **relative pattern with a separator** (`build/out`, `target/**/*.o`)
//!   is matched against the end of the path (it is anchored as `**/pattern`);
//! - an **absolute pattern with wildcards** (`/home/*/cache`) is matched
//!   against the whole path.
//!
//! A matching directory is never entered, so its contents are neither listed
//! nor counted (one `excluded` count for the directory itself).
//! Includes (scope `include`) only filter which *files* are indexed;
//! directories are always walked so includes can match below them.

use std::ffi::OsStr;
use std::path::Path;

use mj_core::glob::{Glob, GlobMode};
use mj_core::pathutil::{is_within, path_key, CASE_INSENSITIVE};
use mj_core::{Attributes, FileEntry, ScanScope};

/// Compiled scope rules.
#[derive(Debug, Clone)]
pub struct ScopeFilter {
    globs: Vec<Glob>,
    /// Absolute literal exclusions: (path key, folded last component).
    paths: Vec<(String, String)>,
    include: Vec<Glob>,
    include_hidden: bool,
}

fn fold(s: &str) -> String {
    if CASE_INSENSITIVE {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

impl ScopeFilter {
    /// Compile the scope's rules plus the global `extra` exclusions.
    pub fn new(scope: &ScanScope, extra: &[String]) -> ScopeFilter {
        let mut f = ScopeFilter {
            globs: Vec::new(),
            paths: Vec::new(),
            include: Vec::new(),
            include_hidden: scope.options.include_hidden,
        };
        for raw in scope.exclude.iter().chain(extra) {
            let p = raw.trim();
            if p.is_empty() {
                continue;
            }
            let g = Glob::platform(p, GlobMode::Path);
            let absolute = Path::new(p).is_absolute() || p.starts_with('/');
            if !g.is_wildcard() && absolute {
                let name = Path::new(p).file_name().map(|n| fold(&n.to_string_lossy())).unwrap_or_default();
                f.paths.push((path_key(Path::new(p)), name));
            } else if !absolute && (p.contains('/') || p.contains('\\')) && !p.starts_with("**") {
                f.globs.push(Glob::platform(&format!("**/{p}"), GlobMode::Path));
            } else {
                f.globs.push(g);
            }
        }
        f.include = scope
            .include
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(|p| Glob::platform(p, GlobMode::Path))
            .collect();
        f
    }

    /// True when the scan root itself is excluded (inside an absolute exclusion or matched by a pattern).
    pub fn excludes_root(&self, root: &Path) -> bool {
        self.paths.iter().any(|(k, _)| is_within(Path::new(k), root)) || self.globs.iter().any(|g| g.matches_path(root))
    }

    /// True when `path` (named `name`) is excluded. The walk never enters an
    /// excluded directory, so an absolute exclusion only has to match exactly.
    pub fn excludes(&self, path: &Path, name: &OsStr) -> bool {
        if !self.paths.is_empty() {
            let n = fold(&name.to_string_lossy());
            if self.paths.iter().any(|(_, last)| *last == n) {
                let key = path_key(path);
                if self.paths.iter().any(|(k, _)| *k == key) {
                    return true;
                }
            }
        }
        self.globs.iter().any(|g| g.matches_path(path))
    }

    /// True when hidden entries are skipped and `name` is a dot name.
    pub fn skips_hidden_name(&self, name: &OsStr) -> bool {
        !self.include_hidden && name.as_encoded_bytes().first() == Some(&b'.')
    }

    /// True when hidden entries are skipped and `e` is hidden (dot name or attribute).
    pub fn skips_hidden(&self, e: &FileEntry) -> bool {
        !self.include_hidden && (e.attrs.contains(Attributes::HIDDEN) || e.name().starts_with('.'))
    }

    /// True when a file passes the include patterns (always, when there are none).
    pub fn includes_file(&self, path: &Path) -> bool {
        self.include.is_empty() || self.include.iter().any(|g| g.matches_path(path))
    }

    /// True when include patterns restrict the indexed files.
    pub fn has_includes(&self) -> bool {
        !self.include.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::EntryKind;

    fn filter(excl: &[&str], incl: &[&str], hidden: bool) -> ScopeFilter {
        let mut s = ScanScope::default();
        s.exclude = excl.iter().map(|s| s.to_string()).collect();
        s.include = incl.iter().map(|s| s.to_string()).collect();
        s.options.include_hidden = hidden;
        ScopeFilter::new(&s, &["  ".into(), "*.tmp".into()])
    }

    fn ex(f: &ScopeFilter, p: &str) -> bool {
        let path = Path::new(p);
        f.excludes(path, path.file_name().unwrap())
    }

    #[test]
    fn absolute_paths_names_and_relative_patterns() {
        let f = filter(&["/home/me/VMs", "node_modules", "build/out", "/srv/*/cache", "/x/y/"], &[], true);
        assert!(ex(&f, "/home/me/VMs"));
        assert!(!ex(&f, "/home/me/VMs2"));
        assert!(!ex(&f, "/home/other/VMs"));
        assert!(ex(&f, "/a/b/node_modules"));
        assert!(!ex(&f, "/a/b/node_modules_x"));
        assert!(ex(&f, "/p/build/out"));
        assert!(!ex(&f, "/p/build/output"));
        assert!(ex(&f, "/srv/web/cache"));
        assert!(!ex(&f, "/srv/web/x/cache"));
        assert!(ex(&f, "/x/y"));
        // Global extra exclusion; blank patterns ignored.
        assert!(ex(&f, "/q/z.tmp"));
        assert!(!ex(&f, "/q/z.txt"));
    }

    #[test]
    fn root_exclusion_and_includes() {
        let f = filter(&["/data"], &["*.jpg", "/keep/**"], true);
        assert!(f.excludes_root(Path::new("/data/photos")));
        assert!(f.excludes_root(Path::new("/data")));
        assert!(!f.excludes_root(Path::new("/database")));
        assert!(f.includes_file(Path::new("/a/b/C.jpg")) || CASE_INSENSITIVE);
        assert!(f.includes_file(Path::new("/a/b/c.jpg")));
        assert!(f.includes_file(Path::new("/keep/x/y.txt")));
        assert!(!f.includes_file(Path::new("/a/b/c.png")));
        assert!(f.has_includes());
        assert!(filter(&[], &[], true).includes_file(Path::new("/z")));
    }

    #[test]
    fn hidden_rules() {
        let f = filter(&[], &[], false);
        assert!(f.skips_hidden_name(OsStr::new(".cache")));
        assert!(!f.skips_hidden_name(OsStr::new("cache")));
        let mut e = FileEntry::new("/a/b", EntryKind::File, 1);
        assert!(!f.skips_hidden(&e));
        e.attrs |= Attributes::HIDDEN;
        assert!(f.skips_hidden(&e));
        assert!(!filter(&[], &[], true).skips_hidden(&e));
    }
}
