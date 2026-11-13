//! Application directories (AUT-04, AUT-07). Owner: W3-E.
//!
//! The resolution is a **pure function** of the operating system, an
//! environment map, the executable's folder and whether a portable marker
//! exists, so both conventions are unit-tested on any host. [`detect`] feeds it
//! the real process environment.
//!
//! | | Windows | Linux |
//! |---|---|---|
//! | config (reserved) | `%APPDATA%\MyJanitor` | `$XDG_CONFIG_HOME/myjanitor` or `~/.config/myjanitor` |
//! | data (settings, schedules, presets, journal, quarantine) | `%LOCALAPPDATA%\MyJanitor` | `$XDG_DATA_HOME/myjanitor` or `~/.local/share/myjanitor` |
//! | cache (scan cache) | `%LOCALAPPDATA%\MyJanitor\cache` | `$XDG_CACHE_HOME/myjanitor` or `~/.cache/myjanitor` |
//! | portable | everything under `<exe dir>/data` | same |
//!
//! Portable mode is on when a file named `portable` (or `myjanitor.portable`)
//! sits next to the executable. An explicit override ([`AppDirs::under`], used
//! by `--data-dir` and by tests) puts every directory below one root.
//!
//! The engine keeps all its state files in the *data* directory (its config
//! takes one `data_dir`), so the *config* directory is reported for the OS
//! integration files and future use but holds nothing yet.
//!
//! The OS integration code (context menu, schedule registration) lives in
//! `settings/` and `schedules/` and is re-exported here as the public surface
//! of this crate for the CLI: see [`integration`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Marker files enabling portable mode (either name).
pub const PORTABLE_MARKERS: &[&str] = &["portable", "myjanitor.portable"];

/// Public surface of the OS integration code (context menu, scheduler registration).
pub mod integration {
    pub use crate::schedules::next::{local_offset_minutes, next_run};
    pub use crate::schedules::os as scheduler;
    pub use crate::settings::context_menu;
    pub use crate::settings::os::{Cmd, CmdOutput, Runner, StdRunner};
}

/// Operating system family for path conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
}

impl Os {
    /// The OS this binary was compiled for.
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Environment variables by name (only the ones the resolution reads matter).
pub type EnvMap = BTreeMap<String, String>;

/// The process environment as an [`EnvMap`] (non-empty values only).
pub fn current_env() -> EnvMap {
    std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

/// Every directory the application uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    /// Reserved for user configuration (see the module docs).
    pub config: PathBuf,
    /// Settings, schedules, presets, journal and quarantine live below it.
    pub data: PathBuf,
    /// Scan cache.
    pub cache: PathBuf,
    /// `<data>/journal` (undo journal, ARCHITECTURE.md 7.3).
    pub journal: PathBuf,
    /// `<data>/quarantine` (default; the setting may override it).
    pub quarantine: PathBuf,
    /// Scratch space when no temp folder is set (`<cache>/tmp`).
    pub temp: PathBuf,
    /// Portable mode: nothing is written outside the executable's folder.
    pub portable: bool,
}

impl AppDirs {
    fn from_parts(config: PathBuf, data: PathBuf, cache: PathBuf, portable: bool) -> AppDirs {
        AppDirs {
            journal: data.join("journal"),
            quarantine: data.join("quarantine"),
            temp: cache.join("tmp"),
            config,
            data,
            cache,
            portable,
        }
    }

    /// Every directory below one root: the `--data-dir` override and the tests.
    pub fn under(root: &Path) -> AppDirs {
        AppDirs::from_parts(root.to_path_buf(), root.to_path_buf(), root.join("cache"), false)
    }
}

/// Resolve the directories for `os` from `env`.
///
/// `exe_dir` is the folder of the running executable; with `portable_marker`
/// true (a marker file sits there) everything goes below `<exe_dir>/data`.
/// Missing variables fall back as the platform convention says; with no usable
/// home at all the system temp folder stands in so start-up never fails.
pub fn resolve(os: Os, env: &EnvMap, exe_dir: Option<&Path>, portable_marker: bool) -> AppDirs {
    if let (true, Some(dir)) = (portable_marker, exe_dir) {
        let base = dir.join("data");
        return AppDirs::from_parts(base.clone(), base.clone(), base.join("cache"), true);
    }
    let var = |k: &str| env.get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    match os {
        Os::Windows => {
            let profile = var("USERPROFILE");
            let roaming = var("APPDATA")
                .or_else(|| profile.as_ref().map(|p| p.join("AppData").join("Roaming")))
                .unwrap_or_else(std::env::temp_dir);
            let local = var("LOCALAPPDATA")
                .or_else(|| profile.as_ref().map(|p| p.join("AppData").join("Local")))
                .unwrap_or_else(|| roaming.clone());
            let data = local.join(mj_core::APP_NAME);
            let cache = data.join("cache");
            AppDirs::from_parts(roaming.join(mj_core::APP_NAME), data, cache, false)
        }
        Os::Linux => {
            let home = var("HOME").unwrap_or_else(std::env::temp_dir);
            // The XDG spec says a relative value must be ignored.
            let xdg =
                |k: &str, fallback: &str| var(k).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(fallback));
            let config = xdg("XDG_CONFIG_HOME", ".config");
            let data = xdg("XDG_DATA_HOME", ".local/share");
            let cache = xdg("XDG_CACHE_HOME", ".cache");
            AppDirs::from_parts(
                config.join(mj_core::APP_DIR),
                data.join(mj_core::APP_DIR),
                cache.join(mj_core::APP_DIR),
                false,
            )
        }
    }
}

/// True when a portable marker file sits in `dir`.
pub fn has_portable_marker(dir: &Path) -> bool {
    PORTABLE_MARKERS.iter().any(|m| dir.join(m).is_file())
}

/// The directories for this process: real OS, real environment, real executable folder.
pub fn detect() -> AppDirs {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let marker = exe_dir.as_deref().is_some_and(has_portable_marker);
    resolve(Os::current(), &current_env(), exe_dir.as_deref(), marker)
}

/// `(data_dir, cache_dir, portable)` for this machine (the Tauri shell and the CLI call this).
pub fn default_dirs() -> (PathBuf, PathBuf, bool) {
    let d = detect();
    (d.data, d.cache, d.portable)
}

/// The scratch folder for optimisation work (AUT-07): the user's setting when
/// given, otherwise `dirs.temp`.
pub fn effective_temp_dir(setting: Option<&Path>, dirs: &AppDirs) -> PathBuf {
    match setting {
        Some(p) if p.is_absolute() => p.to_path_buf(),
        _ => dirs.temp.clone(),
    }
}

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    let k = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> EnvMap {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn windows_layout_uses_appdata_and_localappdata() {
        let e = env(&[("APPDATA", "C:\\Users\\m\\AppData\\Roaming"), ("LOCALAPPDATA", "C:\\Users\\m\\AppData\\Local")]);
        let d = resolve(Os::Windows, &e, None, false);
        assert_eq!(d.config, PathBuf::from("C:\\Users\\m\\AppData\\Roaming").join("MyJanitor"));
        assert_eq!(d.data, PathBuf::from("C:\\Users\\m\\AppData\\Local").join("MyJanitor"));
        assert_eq!(d.cache, d.data.join("cache"));
        assert_eq!(d.journal, d.data.join("journal"));
        assert_eq!(d.quarantine, d.data.join("quarantine"));
        assert!(d.temp.starts_with(&d.cache));
        assert!(!d.portable);
    }

    #[test]
    fn windows_falls_back_to_userprofile() {
        let e = env(&[("USERPROFILE", "C:\\Users\\m")]);
        let d = resolve(Os::Windows, &e, None, false);
        assert_eq!(d.config, PathBuf::from("C:\\Users\\m").join("AppData").join("Roaming").join("MyJanitor"));
        assert_eq!(d.data, PathBuf::from("C:\\Users\\m").join("AppData").join("Local").join("MyJanitor"));
    }

    #[test]
    fn linux_layout_follows_xdg_with_fallbacks() {
        let e = env(&[("HOME", "/home/m")]);
        let d = resolve(Os::Linux, &e, None, false);
        assert_eq!(d.config, PathBuf::from("/home/m/.config/myjanitor"));
        assert_eq!(d.data, PathBuf::from("/home/m/.local/share/myjanitor"));
        assert_eq!(d.cache, PathBuf::from("/home/m/.cache/myjanitor"));
        assert_eq!(d.journal, PathBuf::from("/home/m/.local/share/myjanitor/journal"));

        let e = env(&[
            ("HOME", "/home/m"),
            ("XDG_CONFIG_HOME", "/x/c"),
            ("XDG_DATA_HOME", "/x/d"),
            ("XDG_CACHE_HOME", "/x/k"),
        ]);
        let d = resolve(Os::Linux, &e, None, false);
        assert_eq!(d.config, PathBuf::from("/x/c/myjanitor"));
        assert_eq!(d.data, PathBuf::from("/x/d/myjanitor"));
        assert_eq!(d.cache, PathBuf::from("/x/k/myjanitor"));
    }

    #[test]
    fn linux_ignores_relative_and_empty_xdg_values() {
        let e = env(&[("HOME", "/home/m"), ("XDG_DATA_HOME", "relative/dir"), ("XDG_CACHE_HOME", "")]);
        let d = resolve(Os::Linux, &e, None, false);
        assert_eq!(d.data, PathBuf::from("/home/m/.local/share/myjanitor"));
        assert_eq!(d.cache, PathBuf::from("/home/m/.cache/myjanitor"));
    }

    #[test]
    fn portable_marker_puts_everything_beside_the_executable() {
        let e = env(&[("HOME", "/home/m")]);
        for os in [Os::Windows, Os::Linux] {
            let d = resolve(os, &e, Some(Path::new("/opt/mj")), true);
            assert!(d.portable);
            assert_eq!(d.data, PathBuf::from("/opt/mj/data"));
            assert_eq!(d.cache, PathBuf::from("/opt/mj/data/cache"));
            assert_eq!(d.journal, PathBuf::from("/opt/mj/data/journal"));
            assert_eq!(d.quarantine, PathBuf::from("/opt/mj/data/quarantine"));
            assert!(d.temp.starts_with("/opt/mj/data"));
        }
        // A marker without a known executable folder cannot be honoured.
        assert!(!resolve(Os::Linux, &e, None, true).portable);
        // No marker: normal layout even when the folder is known.
        assert!(!resolve(Os::Linux, &e, Some(Path::new("/opt/mj")), false).portable);
    }

    #[test]
    fn explicit_override_roots_every_directory() {
        let d = AppDirs::under(Path::new("/tmp/t"));
        assert_eq!(d.data, PathBuf::from("/tmp/t"));
        assert_eq!(d.cache, PathBuf::from("/tmp/t/cache"));
        assert_eq!(d.journal, PathBuf::from("/tmp/t/journal"));
        assert_eq!(d.quarantine, PathBuf::from("/tmp/t/quarantine"));
        assert!(d.temp.starts_with("/tmp/t"));
        assert!(!d.portable);
    }

    #[test]
    fn temp_setting_wins_when_absolute() {
        let d = AppDirs::under(Path::new("/tmp/t"));
        assert_eq!(effective_temp_dir(Some(Path::new("/mnt/ram")), &d), PathBuf::from("/mnt/ram"));
        assert_eq!(effective_temp_dir(Some(Path::new("relative")), &d), d.temp);
        assert_eq!(effective_temp_dir(None, &d), d.temp);
    }

    #[test]
    fn marker_detection_reads_the_folder() {
        let t = mj_core::testutil::TempDir::new("paths-marker").unwrap();
        assert!(!has_portable_marker(t.path()));
        t.write("portable", b"").unwrap();
        assert!(has_portable_marker(t.path()));
        let t2 = mj_core::testutil::TempDir::new("paths-marker2").unwrap();
        t2.write("myjanitor.portable", b"").unwrap();
        assert!(has_portable_marker(t2.path()));
    }
}
