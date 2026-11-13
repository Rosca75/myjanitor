//! Protected paths (SAF-01): a fixed built-in list per OS (shown locked) plus the
//! user's list from settings. Owner: W2-E (extend the built-in list; keep the API).

use std::path::{Path, PathBuf};

use mj_core::glob::{Glob, GlobMode};
use mj_core::pathutil::is_within;

/// Built-in protected locations (exact folders; everything below is protected).
pub fn builtin_paths() -> Vec<String> {
    if cfg!(windows) {
        [
            "C:\\Windows",
            "C:\\Program Files",
            "C:\\Program Files (x86)",
            "C:\\ProgramData\\Microsoft",
            "C:\\Recovery",
            "C:\\System Volume Information",
            "C:\\$Recycle.Bin",
            "C:\\Boot",
            "C:\\EFI",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    } else {
        [
            "/bin",
            "/boot",
            "/dev",
            "/etc",
            "/lib",
            "/lib32",
            "/lib64",
            "/libx32",
            "/proc",
            "/run",
            "/sbin",
            "/snap",
            "/sys",
            "/usr",
            "/var/lib",
            "/var/log/journal",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }
}

/// Built-in protected patterns (match anywhere).
pub fn builtin_patterns() -> Vec<String> {
    ["**/.git", "**/.git/**", "**/.ssh/**", "**/.gnupg/**", "**/pagefile.sys", "**/hiberfil.sys", "**/swapfile.sys"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// The effective protected set.
#[derive(Debug, Clone)]
pub struct ProtectedPaths {
    folders: Vec<PathBuf>,
    patterns: Vec<Glob>,
    user: Vec<PathBuf>,
}

impl ProtectedPaths {
    /// Built-ins plus the user's protected paths.
    pub fn new(user: &[PathBuf]) -> Self {
        ProtectedPaths {
            folders: builtin_paths().into_iter().map(PathBuf::from).collect(),
            patterns: builtin_patterns().iter().map(|p| Glob::platform(p, GlobMode::Path)).collect(),
            user: user.to_vec(),
        }
    }

    /// Nothing protected (tests only; never used by the engine).
    pub fn empty() -> Self {
        ProtectedPaths { folders: Vec::new(), patterns: Vec::new(), user: Vec::new() }
    }

    /// True when `path` is protected.
    pub fn is_protected(&self, path: &Path) -> bool {
        self.folders.iter().chain(self.user.iter()).any(|f| is_within(f, path))
            || self.patterns.iter().any(|g| g.matches_path(path))
    }

    /// The user's list.
    pub fn user(&self) -> &[PathBuf] {
        &self.user
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_and_user() {
        let p = ProtectedPaths::new(&[PathBuf::from("/home/me/keep")]);
        assert!(p.is_protected(Path::new("/home/me/keep/x")));
        assert!(p.is_protected(Path::new("/home/me/src/.git/config")));
        assert!(!p.is_protected(Path::new("/home/me/src/main.rs")));
        #[cfg(unix)]
        assert!(p.is_protected(Path::new("/etc/passwd")));
    }
}
