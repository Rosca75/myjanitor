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

use mj_core::glob::{entry_glob, is_absolute_entry, is_pattern_entry, Glob, GlobMode};
use mj_core::pathutil::path_key;

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

/// Built-in protected patterns (match anywhere): version-control metadata
/// ([`builtin_vcs_patterns`]), credential stores and OS files.
pub fn builtin_patterns() -> Vec<String> {
    let mut v = builtin_vcs_patterns();
    v.extend(builtin_credential_patterns());
    v.extend(
        [
            "**/pagefile.sys",
            "**/hiberfil.sys",
            "**/swapfile.sys",
            "**/System Volume Information/**",
            "**/$Recycle.Bin/**",
            "**/.Trash-*/**",
            "**/.local/share/Trash/**",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    v
}

/// Version-control metadata folders (and everything in them). A regenerable artefact
/// folder a detector owns as a whole (`node_modules`, `target`, a package cache) may hold
/// such folders (packages installed from git) and may still be removed with them; any
/// other folder holding one is refused (F-01).
pub fn builtin_vcs_patterns() -> Vec<String> {
    let mut v = Vec::new();
    for d in [".git", ".hg", ".svn", ".bzr"] {
        v.push(format!("**/{d}"));
        v.push(format!("**/{d}/**"));
    }
    v
}

/// Credential stores: protected everywhere, even inside an artefact folder.
pub fn builtin_credential_patterns() -> Vec<String> {
    let mut v = Vec::new();
    for d in [".ssh", ".gnupg", ".aws", ".azure", ".kube", ".password-store", ".docker"] {
        v.push(format!("**/{d}"));
        v.push(format!("**/{d}/**"));
    }
    v
}

/// The canonical spelling of an existing absolute path when it differs from the given one
/// (G-01). `None` for a path that does not exist (yet) or already is canonical.
pub fn canonical_spelling(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let c = std::fs::canonicalize(path).ok()?;
    (path_key(&c) != path_key(path)).then_some(c)
}

/// The effective protected set.
#[derive(Debug, Clone)]
pub struct ProtectedPaths {
    folders: Vec<PathBuf>,
    exact: Vec<PathBuf>,
    patterns: Vec<Glob>,
    /// Version-control patterns (also in `patterns`'s role, kept apart so an owned artefact
    /// tree can be checked without them).
    vcs_patterns: Vec<Glob>,
    user: Vec<PathBuf>,
    user_patterns: Vec<Glob>,
    /// `path_key` of every folder and user folder (with a trailing `/`) and of every exact
    /// entry, computed once: `is_protected` runs for every candidate of every card, and
    /// re-normalising ~50 built-in paths per call made badging the scan's hot spot
    /// (W4-INT: 1.9 s of a 2.3 s exact-duplicates analysis at 48 000 candidates).
    folder_prefixes: Vec<String>,
    exact_keys: Vec<String>,
}

/// `path_key` of `root` as the prefix a path below it starts with.
fn prefix_key(root: &Path) -> String {
    let r = path_key(root);
    if r.ends_with('/') {
        r
    } else {
        format!("{r}/")
    }
}

impl ProtectedPaths {
    /// Recompute the cached keys after the lists changed. G-01: every folder and exact
    /// entry that exists is keyed in its given spelling **and** in its canonical one (links,
    /// junctions, mapped and `subst` drives resolved; short names expanded on Windows), so a
    /// path reached through another spelling of the same location is still recognised.
    fn rekey(mut self) -> Self {
        let mut prefixes = Vec::new();
        for f in self.folders.iter().chain(self.user.iter()) {
            prefixes.push(prefix_key(f));
            if let Some(c) = canonical_spelling(f) {
                prefixes.push(prefix_key(&c));
            }
        }
        let mut exact = Vec::new();
        for e in &self.exact {
            exact.push(path_key(e));
            if let Some(c) = canonical_spelling(e) {
                exact.push(path_key(&c));
            }
        }
        prefixes.sort();
        prefixes.dedup();
        exact.sort();
        exact.dedup();
        self.folder_prefixes = prefixes;
        self.exact_keys = exact;
        self
    }

    /// Built-ins plus the user's protected paths, classified by
    /// [`mj_core::glob::is_pattern_entry`] (RF-2, SCAN-2): a relative entry containing `*`,
    /// `?` or `[` is a pattern (`**/Clients/**`); an absolute entry is always the folder of
    /// that exact name (`D:\Clients [2024]`), and when it also holds `*` or `?` it is in
    /// addition a pattern whose brackets are literal. Treating an entry as both can only
    /// protect more, never less.
    pub fn new(user: &[PathBuf]) -> Self {
        let mut pats = Vec::new();
        let mut folders = Vec::new();
        for p in user {
            let s = p.to_string_lossy();
            if is_pattern_entry(&s) {
                pats.push(entry_glob(&s));
            }
            if !is_pattern_entry(&s) || is_absolute_entry(&s) {
                folders.push(p.clone());
            }
        }
        ProtectedPaths {
            folders: builtin_paths().into_iter().map(PathBuf::from).collect(),
            exact: builtin_exact().into_iter().map(PathBuf::from).collect(),
            patterns: builtin_patterns()
                .iter()
                .filter(|p| !builtin_vcs_patterns().contains(p))
                .map(|p| Glob::platform(p, GlobMode::Path))
                .collect(),
            vcs_patterns: builtin_vcs_patterns().iter().map(|p| Glob::platform(p, GlobMode::Path)).collect(),
            user: folders,
            user_patterns: pats,
            folder_prefixes: Vec::new(),
            exact_keys: Vec::new(),
        }
        .rekey()
    }

    /// Nothing protected (tests only; never used by the engine).
    pub fn empty() -> Self {
        ProtectedPaths {
            folders: Vec::new(),
            exact: Vec::new(),
            patterns: Vec::new(),
            vcs_patterns: Vec::new(),
            user: Vec::new(),
            user_patterns: Vec::new(),
            folder_prefixes: Vec::new(),
            exact_keys: Vec::new(),
        }
    }

    /// Add folders (the app's config, data, cache, quarantine and journal folders when not
    /// at their default places).
    pub fn with_app_dirs(mut self, dirs: &[PathBuf]) -> Self {
        self.folders.extend(dirs.iter().cloned());
        self.rekey()
    }

    /// Add user folders (everything below each is protected).
    pub fn with_user_folders(mut self, dirs: &[PathBuf]) -> Self {
        self.user.extend(dirs.iter().cloned());
        self.rekey()
    }

    /// Add user patterns.
    pub fn with_patterns(mut self, patterns: &[String]) -> Self {
        self.user_patterns.extend(patterns.iter().map(|p| Glob::platform(p, GlobMode::Path)));
        self
    }

    /// True when `path` is protected.
    pub fn is_protected(&self, path: &Path) -> bool {
        self.is_protected_as(path, false)
    }

    /// True when `path` is protected inside a folder a detector owns as a whole (a
    /// regenerable artefact such as `node_modules`): every rule applies except the
    /// version-control patterns ([`builtin_vcs_patterns`]).
    pub fn is_protected_in_owned_tree(&self, path: &Path) -> bool {
        self.is_protected_as(path, true)
    }

    /// True when a protected folder or exact entry lies strictly below `path` (lexically;
    /// patterns need the real tree, see `Guard::check_tree`).
    pub fn has_protected_below(&self, path: &Path) -> bool {
        let k = prefix_key(path);
        self.folder_prefixes.iter().any(|p| p.len() > k.len() && p.starts_with(k.as_str()))
            || self.exact_keys.iter().any(|e| e.len() > k.len() && e.starts_with(k.as_str()))
    }

    fn is_protected_as(&self, path: &Path, owned_tree: bool) -> bool {
        let key = path_key(path);
        // Same rule as `is_within`: the folder itself or anything below it.
        let below =
            |prefix: &String| key.starts_with(prefix.as_str()) || prefix.strip_suffix('/') == Some(key.as_str());
        self.folder_prefixes.iter().any(below)
            || self.exact_keys.contains(&key)
            || self.patterns.iter().chain(self.user_patterns.iter()).any(|g| g.matches_path(path))
            || (!owned_tree && self.vcs_patterns.iter().any(|g| g.matches_path(path)))
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

    /// RF-2 / SCAN-2: a protected folder whose name holds `[`, `*` or `?` is a folder, not a
    /// never-matching glob.
    #[test]
    fn a_literal_folder_with_glob_characters_is_protected() {
        let p = ProtectedPaths::new(&[PathBuf::from("/home/me/Clients [2024]"), PathBuf::from("**/Clients/**")]);
        assert!(p.is_protected(Path::new("/home/me/Clients [2024]")));
        assert!(p.is_protected(Path::new("/home/me/Clients [2024]/a.doc")));
        assert!(p.has_protected_below(Path::new("/home/me")));
        assert!(!p.is_protected(Path::new("/home/me/Clients 2/a.doc")), "brackets are not a class");
        assert!(p.is_protected(Path::new("/data/Clients/a.pdf")), "relative patterns stay patterns");
        // An absolute entry with a real wildcard is a pattern whose brackets are literal, and
        // also the folder of that exact name.
        let p = ProtectedPaths::new(&[PathBuf::from("/data/[raw] */keep/**"), PathBuf::from("/srv/what?")]);
        assert!(p.is_protected(Path::new("/data/[raw] 2019/keep/a.cr2")));
        assert!(!p.is_protected(Path::new("/data/r 2019/keep/a.cr2")));
        assert!(p.is_protected(Path::new("/srv/what?/x")), "a folder literally named `what?`");
        assert!(p.is_protected(Path::new("/srv/whatx")));
    }
}
