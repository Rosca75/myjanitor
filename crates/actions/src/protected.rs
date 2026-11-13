//! Protected paths (SAF-01): a fixed built-in list per OS (shown locked) plus the
//! user's list from settings. Owner: W2-E.
//!
//! Three kinds of entries:
//!
//! - **folders**: the folder and everything below it (`/usr`, `C:\Windows`);
//! - **exact** paths: only that path itself, so files below stay actionable
//!   (`/`, `/home`, the user's home / profile root, `C:\Users`);
//! - **patterns**: globs matched anywhere (`**/.git/**`).
//!
//! The app's own folders (config, data, cache, quarantine, journal) are protected
//! as folders: built in from the environment and added explicitly by the engine
//! through [`ProtectedPaths::with_app_dirs`] for non-default locations.

use std::path::{Path, PathBuf};

use mj_core::glob::{Glob, GlobMode};
use mj_core::pathutil::{is_within, path_key};

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The user's home / profile folder, when known.
pub fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        env_path("USERPROFILE")
    } else {
        env_path("HOME")
    }
}

/// MyJanitor's own folders under the platform's default locations (config, data, cache).
pub fn default_app_dirs() -> Vec<PathBuf> {
    let app = mj_core::APP_DIR;
    let mut v = Vec::new();
    if cfg!(windows) {
        for k in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(p) = env_path(k) {
                v.push(p.join(mj_core::APP_NAME));
                v.push(p.join(app));
            }
        }
    } else {
        let home = home_dir();
        let xdg = |k: &str, rel: &str| env_path(k).or_else(|| home.as_ref().map(|h| h.join(rel)));
        for (k, rel) in
            [("XDG_CONFIG_HOME", ".config"), ("XDG_DATA_HOME", ".local/share"), ("XDG_CACHE_HOME", ".cache")]
        {
            if let Some(p) = xdg(k, rel) {
                v.push(p.join(app));
            }
        }
    }
    v
}

/// Built-in protected locations (exact folders; everything below is protected).
pub fn builtin_paths() -> Vec<String> {
    let mut v: Vec<String> = if cfg!(windows) {
        let mut v: Vec<String> = [
            "C:\\Windows",
            "C:\\Program Files",
            "C:\\Program Files (x86)",
            "C:\\ProgramData",
            "C:\\Recovery",
            "C:\\System Volume Information",
            "C:\\$Recycle.Bin",
            "C:\\$WinREAgent",
            "C:\\Boot",
            "C:\\EFI",
            "C:\\PerfLogs",
            "C:\\Config.Msi",
            "C:\\Users\\Default",
            "C:\\Users\\All Users",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for k in ["SystemRoot", "windir", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData"] {
            if let Some(p) = env_path(k) {
                v.push(p.to_string_lossy().into_owned());
            }
        }
        v
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
            "/lost+found",
            "/opt",
            "/proc",
            "/root",
            "/run",
            "/sbin",
            "/snap",
            "/srv",
            "/sys",
            "/usr",
            "/var",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    };
    v.extend(default_app_dirs().iter().map(|p| p.to_string_lossy().into_owned()));
    v.sort();
    v.dedup();
    v
}

/// Built-in paths protected only as themselves (their contents stay actionable).
pub fn builtin_exact() -> Vec<String> {
    let mut v: Vec<String> = if cfg!(windows) {
        ["C:\\", "C:\\Users", "C:\\Users\\Public"].iter().map(|s| s.to_string()).collect()
    } else {
        ["/", "/home", "/media", "/mnt", "/tmp", "/var/tmp"].iter().map(|s| s.to_string()).collect()
    };
    if let Some(h) = home_dir() {
        v.push(h.to_string_lossy().into_owned());
    }
    v
}

/// Built-in protected patterns (match anywhere).
pub fn builtin_patterns() -> Vec<String> {
    [
        "**/.git",
        "**/.git/**",
        "**/.ssh/**",
        "**/.gnupg/**",
        "**/pagefile.sys",
        "**/hiberfil.sys",
        "**/swapfile.sys",
        "**/System Volume Information/**",
        "**/$Recycle.Bin/**",
        "**/.Trash-*/**",
        "**/.local/share/Trash/**",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn is_glob(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// The effective protected set.
#[derive(Debug, Clone)]
pub struct ProtectedPaths {
    folders: Vec<PathBuf>,
    exact: Vec<PathBuf>,
    patterns: Vec<Glob>,
    user: Vec<PathBuf>,
    user_patterns: Vec<Glob>,
}

impl ProtectedPaths {
    /// Built-ins plus the user's protected paths. A user entry containing `*`, `?` or `[`
    /// is a pattern (`**/Clients/**`), any other entry a folder.
    pub fn new(user: &[PathBuf]) -> Self {
        let (pats, folders): (Vec<&PathBuf>, Vec<&PathBuf>) = user.iter().partition(|p| is_glob(&p.to_string_lossy()));
        ProtectedPaths {
            folders: builtin_paths().into_iter().map(PathBuf::from).collect(),
            exact: builtin_exact().into_iter().map(PathBuf::from).collect(),
            patterns: builtin_patterns().iter().map(|p| Glob::platform(p, GlobMode::Path)).collect(),
            user: folders.into_iter().cloned().collect(),
            user_patterns: pats.iter().map(|p| Glob::platform(&p.to_string_lossy(), GlobMode::Path)).collect(),
        }
    }

    /// Nothing protected (tests only; never used by the engine).
    pub fn empty() -> Self {
        ProtectedPaths {
            folders: Vec::new(),
            exact: Vec::new(),
            patterns: Vec::new(),
            user: Vec::new(),
            user_patterns: Vec::new(),
        }
    }

    /// Add folders (the app's config, data, cache, quarantine and journal folders when not
    /// at their default places).
    pub fn with_app_dirs(mut self, dirs: &[PathBuf]) -> Self {
        self.folders.extend(dirs.iter().cloned());
        self
    }

    /// Add user patterns.
    pub fn with_patterns(mut self, patterns: &[String]) -> Self {
        self.user_patterns.extend(patterns.iter().map(|p| Glob::platform(p, GlobMode::Path)));
        self
    }

    /// True when `path` is protected.
    pub fn is_protected(&self, path: &Path) -> bool {
        let key = path_key(path);
        self.folders.iter().chain(self.user.iter()).any(|f| is_within(f, path))
            || self.exact.iter().any(|e| path_key(e) == key)
            || self.patterns.iter().chain(self.user_patterns.iter()).any(|g| g.matches_path(path))
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
        let p = ProtectedPaths::new(&[PathBuf::from("/home/me/keep"), PathBuf::from("**/Clients/**")]);
        assert!(p.is_protected(Path::new("/home/me/keep/x")));
        assert!(p.is_protected(Path::new("/home/me/src/.git/config")));
        assert!(p.is_protected(Path::new("/data/Clients/a.pdf")));
        assert!(!p.is_protected(Path::new("/home/me/src/main.rs")));
        #[cfg(unix)]
        {
            assert!(p.is_protected(Path::new("/etc/passwd")));
            assert!(p.is_protected(Path::new("/var/log/syslog")));
            assert!(p.is_protected(Path::new("/")));
            assert!(p.is_protected(Path::new("/home")));
            assert!(!p.is_protected(Path::new("/home/someone/file.txt")), "exact entries protect only themselves");
            assert!(!p.is_protected(Path::new("/tmp/x")));
            if let Some(h) = home_dir() {
                assert!(p.is_protected(&h));
                assert!(p.is_protected(&h.join(".local/share/myjanitor/journal/entries-2026-11.jsonl")));
            }
        }
        let p = ProtectedPaths::empty().with_app_dirs(&[PathBuf::from("/q")]).with_patterns(&["*.key".into()]);
        assert!(p.is_protected(Path::new("/q/1/2/x")));
        assert!(p.is_protected(Path::new("/a/b.key")));
    }
}
