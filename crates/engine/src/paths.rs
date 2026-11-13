//! Data and cache directories (owner W3-E for portable mode details).
//!
//! | | Windows | Linux |
//! |---|---|---|
//! | data | `%APPDATA%\MyJanitor` | `$XDG_DATA_HOME/myjanitor` or `~/.local/share/myjanitor` |
//! | cache | `%LOCALAPPDATA%\MyJanitor\cache` | `$XDG_CACHE_HOME/myjanitor` or `~/.cache/myjanitor` |
//! | portable (AUT-04) | `<exe dir>\MyJanitor-data\{data,cache}` | same |
//!
//! Portable mode is on when a file named `myjanitor.portable` sits next to the executable.

use std::path::{Path, PathBuf};

/// Marker file enabling portable mode.
pub const PORTABLE_MARKER: &str = "myjanitor.portable";

/// `(data_dir, cache_dir, portable)` for this machine.
pub fn default_dirs() -> (PathBuf, PathBuf, bool) {
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        if dir.join(PORTABLE_MARKER).is_file() {
            let base = dir.join("MyJanitor-data");
            return (base.join("data"), base.join("cache"), true);
        }
    }
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        let roaming = env("APPDATA").unwrap_or_else(std::env::temp_dir);
        let local = env("LOCALAPPDATA").unwrap_or_else(|| roaming.clone());
        (roaming.join(mj_core::APP_NAME), local.join(mj_core::APP_NAME).join("cache"), false)
    } else {
        let home = env("HOME").unwrap_or_else(std::env::temp_dir);
        let data = env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let cache = env("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"));
        (data.join(mj_core::APP_DIR), cache.join(mj_core::APP_DIR), false)
    }
}

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    let k = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from)
}
