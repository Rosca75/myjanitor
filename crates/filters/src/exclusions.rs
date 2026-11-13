//! Global exclusions / whitelist (CLN-ENG-03). Owner: W2-C.
//!
//! Applied before any rule: an excluded entry, and everything below an
//! excluded folder, is never evaluated and never returned. The list mixes
//! absolute paths and glob patterns, as in `Settings::safety::exclusions`:
//! an entry that is absolute and has no wildcard is a path (the folder and its
//! whole subtree), anything else is a pattern (matched against the full path
//! when it contains a separator, against the file name otherwise; `*.sys`
//! therefore excludes by type).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use mj_core::glob::{Glob, GlobMode};
use mj_core::pathutil::path_key;
use mj_core::FileEntry;

/// A compiled exclusion list.
#[derive(Debug, Default)]
pub struct Exclusions {
    /// `path_key`s of excluded folders/files.
    path_keys: HashSet<String>,
    patterns: Vec<Glob>,
}

fn is_absolute_like(s: &str) -> bool {
    s.starts_with('/') || s.starts_with('\\') || s.as_bytes().get(1) == Some(&b':')
}

impl Exclusions {
    /// An empty list (excludes nothing).
    pub fn none() -> Exclusions {
        Exclusions::default()
    }

    /// Build from the settings strings (see the module docs for the path/pattern split).
    pub fn from_entries<S: AsRef<str>>(entries: &[S]) -> Exclusions {
        let mut ex = Exclusions::default();
        for e in entries {
            let e = e.as_ref().trim();
            if e.is_empty() {
                continue;
            }
            if is_absolute_like(e) && !e.contains(['*', '?']) {
                ex.add_path(Path::new(e));
            } else {
                ex.add_pattern(e);
            }
        }
        ex
    }

    /// Exclude a path and everything below it.
    pub fn add_path(&mut self, path: &Path) {
        self.path_keys.insert(path_key(path));
    }

    /// Exclude entries matching a glob (case-insensitive on Windows).
    pub fn add_pattern(&mut self, pattern: &str) {
        self.patterns.push(Glob::platform(pattern, GlobMode::Path));
    }

    /// Exclude all paths in a list.
    pub fn with_paths(mut self, paths: &[PathBuf]) -> Exclusions {
        for p in paths {
            self.add_path(p);
        }
        self
    }

    /// True when the list excludes nothing.
    pub fn is_empty(&self) -> bool {
        self.path_keys.is_empty() && self.patterns.is_empty()
    }

    /// Is this entry excluded (itself, by path ancestry or by pattern)?
    pub fn is_excluded(&self, entry: &FileEntry) -> bool {
        if self.is_empty() {
            return false;
        }
        if !self.path_keys.is_empty() {
            let mut key = path_key(&entry.path);
            loop {
                if self.path_keys.contains(&key) {
                    return true;
                }
                match key.rfind('/') {
                    Some(0) | None => break,
                    Some(i) => key.truncate(i),
                }
            }
            if self.path_keys.contains("/") {
                return true;
            }
        }
        self.patterns.iter().any(|g| g.matches_path(&entry.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::EntryKind;

    fn e(p: &str) -> FileEntry {
        FileEntry::new(p, EntryKind::File, 1)
    }

    #[test]
    fn paths_cover_their_subtree_but_not_siblings() {
        let ex = Exclusions::from_entries(&["/data/keep", "/data/also/"]);
        assert!(ex.is_excluded(&e("/data/keep")));
        assert!(ex.is_excluded(&e("/data/keep/a/b.txt")));
        assert!(ex.is_excluded(&e("/data/also/x")));
        assert!(!ex.is_excluded(&e("/data/keeper/a.txt")));
        assert!(!ex.is_excluded(&e("/data/other.txt")));
    }

    #[test]
    fn patterns_by_name_and_by_path() {
        let ex = Exclusions::from_entries(&["*.sys", "**/node_modules/**", "  ", ""]);
        assert!(ex.is_excluded(&e("/a/b/driver.sys")));
        assert!(ex.is_excluded(&e("/a/node_modules/x/y.js")));
        assert!(!ex.is_excluded(&e("/a/b/readme.txt")));
        assert!(!Exclusions::none().is_excluded(&e("/a")));
        assert!(Exclusions::from_entries::<&str>(&[]).is_empty());
    }
}
