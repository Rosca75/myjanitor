//! Settings, protected paths and the scan cache (Settings drawer). Owner: W3-E.
//!
//! Stored in `<data>/settings.json` (pretty JSON, atomic write). Rules:
//!
//! * **Loading never fails.** A missing file gives defaults; a file that cannot
//!   be parsed is moved aside as `settings.json.bad-<secs>` and defaults are used;
//!   a file that parses but holds out-of-range values is *repaired* ([`sanitize`])
//!   rather than rejected. Unknown fields are ignored and missing ones default
//!   (the core type is `#[serde(default)]`), so settings files survive upgrades.
//! * **Saving is strict.** [`validate`] rejects bad input with the field name in
//!   the message (`invalid_input`), after [`normalise`] tidied it.
//! * **Side effects** of a change (context menu, AUT-03) run before the new
//!   settings are stored; when they fail the old settings stay.
//!
//! Not stored here: "kept languages" (no such field in the frozen `Settings`)
//! and anything the OPT-FSC-03 watcher would need beyond the stored toggle and
//! folder list (the watcher is stored but unavailable, see `docs/decisions/W3-E.md`).

pub mod context_menu;
pub mod os;
pub(crate) mod persist;

use std::path::{Path, PathBuf};

use mj_core::{lock, Error, OriginalsPolicy, Result, Settings};

use crate::api::{CacheInfo, ProtectedPathsInfo, ProtectedPathsSetRequest, SettingsSetRequest};
use crate::Engine;

const FILE: &str = "settings.json";

/// Upper bound of the worker thread setting (0 = all logical CPUs).
pub const MAX_THREADS: u16 = 256;
/// Upper bound of parallel optimisation jobs (0 = automatic).
pub const MAX_PARALLEL_JOBS: u16 = 64;
/// Upper bound of undo / quarantine retention in days (ten years).
pub const MAX_DAYS: u32 = 3650;

/// Load settings from `path`, repairing what can be repaired. Never fails.
pub fn load_from(path: &Path) -> Settings {
    match persist::read_or_backup::<Settings>(path) {
        persist::Loaded::Ok(mut s) => {
            sanitize(&mut s);
            s
        }
        persist::Loaded::Missing | persist::Loaded::Corrupt { .. } => Settings::default(),
    }
}

pub(crate) fn load(engine: &Engine) {
    if engine.cfg.persist {
        *lock(&engine.settings) = load_from(&engine.cfg.data_dir.join(FILE));
    }
}

/// Repair a loaded (possibly hand-edited) settings value so the rest of the app
/// can rely on its invariants. The strict counterpart is [`validate`].
pub fn sanitize(s: &mut Settings) {
    let d = Settings::default();
    s.version = mj_core::settings::SETTINGS_VERSION;
    if !(1..=MAX_DAYS).contains(&s.general.undo_days) {
        s.general.undo_days = d.general.undo_days;
    }
    if !(1..=MAX_DAYS).contains(&s.general.quarantine_days) {
        s.general.quarantine_days = d.general.quarantine_days;
    }
    if suffix_error(&s.general.light_copy_suffix).is_some() {
        s.general.light_copy_suffix = d.general.light_copy_suffix;
    }
    s.optimise.keep_if_not_smaller = true;
    s.performance.threads = s.performance.threads.min(MAX_THREADS);
    s.performance.parallel_jobs = s.performance.parallel_jobs.min(MAX_PARALLEL_JOBS);
    s.scan.threads = s.scan.threads.min(MAX_THREADS);
    for list in [&mut s.safety.protected_paths, &mut s.integration.watched_folders] {
        list.retain(|p| p.is_absolute());
    }
    s.tool_paths.retain(|k, v| !k.trim().is_empty() && v.is_absolute());
    s.general.quarantine_dir = s.general.quarantine_dir.take().filter(|p| p.is_absolute());
    s.performance.temp_dir = s.performance.temp_dir.take().filter(|p| p.is_absolute());
    if let OriginalsPolicy::BackupFolder { folder } = &s.general.default_originals {
        if !folder.is_absolute() {
            s.general.default_originals = OriginalsPolicy::RecycleBin;
        }
    }
    normalise(s);
}

/// Tidy a settings value without changing its meaning: trimmed, de-duplicated
/// exclusions and protected paths, canonical extension spelling.
pub fn normalise(s: &mut Settings) {
    let mut seen = std::collections::BTreeSet::new();
    s.safety.exclusions = s
        .safety
        .exclusions
        .iter()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty() && seen.insert(e.clone()))
        .collect();
    dedupe_paths(&mut s.safety.protected_paths);
    dedupe_paths(&mut s.integration.watched_folders);
    for exts in s.type_groups.values_mut() {
        for e in exts.iter_mut() {
            *e = e.trim().trim_start_matches('.').to_lowercase();
        }
        let mut seen = std::collections::BTreeSet::new();
        exts.retain(|e| !e.is_empty() && seen.insert(e.clone()));
    }
}

fn dedupe_paths(list: &mut Vec<PathBuf>) {
    let mut seen = std::collections::BTreeSet::new();
    list.retain(|p| seen.insert(mj_core::pathutil::path_key(p)));
}

fn suffix_error(s: &str) -> Option<&'static str> {
    if s.trim().is_empty() {
        Some("the light-copy suffix must not be empty")
    } else if s.chars().count() > 32 {
        Some("the light-copy suffix must be at most 32 characters")
    } else if s.contains(['/', '\\', '<', '>', ':', '"', '|', '?', '*', '\0']) {
        Some("the light-copy suffix must be a plain name fragment (no path separators or reserved characters)")
    } else {
        None
    }
}

fn require_absolute(field: &str, p: &Path) -> Result<()> {
    if p.is_absolute() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!("{field} must be an absolute path: {}", p.display())))
    }
}

fn bad(msg: impl Into<String>) -> Error {
    Error::InvalidInput(msg.into())
}

/// Strict validation: the first problem is reported with the field it belongs to.
pub fn validate(s: &Settings) -> Result<()> {
    let g = &s.general;
    if !(1..=MAX_DAYS).contains(&g.undo_days) {
        return Err(bad(format!("general.undo_days must be between 1 and {MAX_DAYS}")));
    }
    if !(1..=MAX_DAYS).contains(&g.quarantine_days) {
        return Err(bad(format!("general.quarantine_days must be between 1 and {MAX_DAYS}")));
    }
    if let Some(m) = suffix_error(&g.light_copy_suffix) {
        return Err(bad(format!("general.light_copy_suffix: {m}")));
    }
    if let OriginalsPolicy::BackupFolder { folder } = &g.default_originals {
        require_absolute("general.default_originals.folder", folder)?;
    }
    if let Some(q) = &g.quarantine_dir {
        require_absolute("general.quarantine_dir", q)?;
    }
    if s.scan.threads > MAX_THREADS {
        return Err(bad(format!("scan.threads must be between 0 and {MAX_THREADS} (0 = all CPUs)")));
    }
    for p in &s.safety.protected_paths {
        require_absolute("safety.protected_paths", p)?;
    }
    for e in &s.safety.exclusions {
        if e.trim().is_empty() || e.contains('\0') || e.len() > 1024 {
            return Err(bad(
                "safety.exclusions: every entry must be a non-empty pattern or path (at most 1024 characters)",
            ));
        }
    }
    if !s.optimise.keep_if_not_smaller {
        return Err(bad("optimise.keep_if_not_smaller cannot be turned off"));
    }
    let p = &s.performance;
    if p.threads > MAX_THREADS {
        return Err(bad(format!("performance.threads must be between 0 and {MAX_THREADS} (0 = all CPUs)")));
    }
    if p.parallel_jobs > MAX_PARALLEL_JOBS {
        return Err(bad(format!(
            "performance.parallel_jobs must be between 0 and {MAX_PARALLEL_JOBS} (0 = automatic)"
        )));
    }
    if let Some(t) = &p.temp_dir {
        require_absolute("performance.temp_dir", t)?;
    }
    for (id, path) in &s.tool_paths {
        if id.trim().is_empty() {
            return Err(bad("tool_paths: a tool id must not be empty"));
        }
        require_absolute(&format!("tool_paths.{id}"), path)?;
    }
    for f in &s.integration.watched_folders {
        require_absolute("integration.watched_folders", f)?;
    }
    for (group, exts) in &s.type_groups {
        if group.trim().is_empty() {
            return Err(bad("type_groups: a group name must not be empty"));
        }
        if exts.iter().any(|e| e.is_empty() || e.contains(['.', '/', '\\', ' ', '*'])) {
            return Err(bad(format!("type_groups.{group}: extensions are written without a dot, e.g. \"mp3\"")));
        }
    }
    Ok(())
}

/// Order the protected-paths list and refuse relative entries.
fn clean_protected(list: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    for p in &list {
        require_absolute("protected paths", p)?;
    }
    let mut v = list;
    dedupe_paths(&mut v);
    Ok(v)
}

impl Engine {
    fn save_settings(&self, s: &Settings) -> Result<()> {
        if self.cfg.persist {
            crate::store::write_json(&self.cfg.data_dir.join(FILE), s)?;
        }
        Ok(())
    }

    /// Apply the OS-level side effects of a change (context menu). Skipped when
    /// the engine does not persist (unit tests) or OS integration is disabled.
    fn apply_side_effects(&self, old: &Settings, new: &Settings) -> Result<()> {
        if old.integration.context_menu == new.integration.context_menu
            || !self.cfg.persist
            || os::os_integration_disabled()
        {
            return Ok(());
        }
        if new.integration.context_menu {
            if self.cfg.portable {
                return Err(Error::unsupported(
                    "context menu",
                    "portable mode keeps everything in its own folder and does not register with the OS",
                ));
            }
            context_menu::install(&os::StdRunner).map(|_| ())
        } else {
            context_menu::remove(&os::StdRunner)
        }
    }

    /// All settings.
    pub fn settings_get(&self) -> Result<Settings> {
        Ok(lock(&self.settings).clone())
    }

    /// Replace settings (normalised, validated, side effects applied, persisted atomically).
    pub fn settings_set(&self, req: SettingsSetRequest) -> Result<Settings> {
        let mut s = req.settings;
        normalise(&mut s);
        validate(&s)?;
        s.version = mj_core::settings::SETTINGS_VERSION;
        let old = lock(&self.settings).clone();
        self.apply_side_effects(&old, &s)?;
        self.save_settings(&s)?;
        *lock(&self.settings) = s.clone();
        Ok(s)
    }

    /// The scratch folder for optimisation work (AUT-07): the setting, else `<cache>/tmp`.
    pub fn temp_dir(&self) -> PathBuf {
        let dirs = crate::paths::AppDirs::under(&self.cfg.data_dir);
        let fallback = crate::paths::AppDirs { temp: self.cfg.cache_dir.join("tmp"), ..dirs };
        crate::paths::effective_temp_dir(lock(&self.settings).performance.temp_dir.as_deref(), &fallback)
    }

    /// Built-in and user protected paths.
    pub fn protected_paths_get(&self) -> Result<ProtectedPathsInfo> {
        Ok(ProtectedPathsInfo {
            builtin: mj_actions::protected::builtin_paths(),
            builtin_patterns: mj_actions::protected::builtin_patterns(),
            user: lock(&self.settings).safety.protected_paths.clone(),
        })
    }

    /// Replace the user's protected paths (absolute only, de-duplicated).
    pub fn protected_paths_set(&self, req: ProtectedPathsSetRequest) -> Result<ProtectedPathsInfo> {
        let user = clean_protected(req.user)?;
        let s = {
            let mut s = lock(&self.settings);
            s.safety.protected_paths = user;
            s.clone()
        };
        self.save_settings(&s)?;
        self.protected_paths_get()
    }

    /// Scan cache size.
    pub fn cache_info(&self) -> Result<CacheInfo> {
        let path = self.cache.file().map(|p| p.to_path_buf());
        let bytes = path.as_ref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len()).unwrap_or(0);
        Ok(CacheInfo { entries: self.cache.len() as u64, bytes, path })
    }

    /// Clear the scan cache.
    pub fn cache_clear(&self) -> Result<CacheInfo> {
        self.cache.clear()?;
        self.cache_info()
    }
}

#[cfg(test)]
mod tests {
    use mj_core::testutil::TempDir;

    use super::*;
    use crate::{Engine, EngineConfig};

    fn engine_in(t: &TempDir) -> std::sync::Arc<Engine> {
        Engine::new(EngineConfig::new(t.join("data"), t.join("cache"))).unwrap()
    }

    fn abs(s: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:\\{}", s.replace('/', "\\")))
        } else {
            PathBuf::from(format!("/{s}"))
        }
    }

    #[test]
    fn round_trip_through_the_engine_and_a_restart() {
        let t = TempDir::new("settings-rt").unwrap();
        let e = engine_in(&t);
        let mut s = e.settings_get().unwrap();
        s.general.undo_days = 14;
        s.performance.threads = 6;
        s.performance.low_memory = true;
        s.safety.exclusions = vec!["**/node_modules".into()];
        s.safety.protected_paths = vec![abs("clients/archive")];
        s.performance.temp_dir = Some(abs("mnt/ram"));
        let saved = e.settings_set(SettingsSetRequest { settings: s.clone() }).unwrap();
        assert_eq!(saved, s);
        assert!(t.join("data/settings.json").is_file());
        assert!(!t.join("data/settings.json.tmp").exists(), "the temp file is renamed away");
        let again = engine_in(&t);
        assert_eq!(again.settings_get().unwrap(), s);
        assert_eq!(again.temp_dir(), abs("mnt/ram"));
    }

    #[test]
    fn corrupt_file_gives_defaults_and_a_backup_not_an_error() {
        let t = TempDir::new("settings-bad").unwrap();
        std::fs::create_dir_all(t.join("data")).unwrap();
        std::fs::write(t.join("data/settings.json"), b"{\"general\": [oops").unwrap();
        let e = engine_in(&t);
        assert_eq!(e.settings_get().unwrap(), Settings::default());
        let backups: Vec<_> = std::fs::read_dir(t.join("data"))
            .unwrap()
            .filter_map(|d| d.ok())
            .filter(|d| d.file_name().to_string_lossy().starts_with("settings.json.bad-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(backups[0].path()).unwrap(), b"{\"general\": [oops");
        // The engine works and saves normally afterwards.
        e.settings_set(SettingsSetRequest { settings: Settings::default() }).unwrap();
        assert!(t.join("data/settings.json").is_file());
    }

    #[test]
    fn unknown_fields_are_ignored_and_missing_ones_default() {
        let t = TempDir::new("settings-unknown").unwrap();
        std::fs::create_dir_all(t.join("data")).unwrap();
        std::fs::write(
            t.join("data/settings.json"),
            br#"{"version":0,"general":{"undo_days":7,"future":true},"later":{}}"#,
        )
        .unwrap();
        let s = engine_in(&t).settings_get().unwrap();
        assert_eq!(s.general.undo_days, 7);
        assert_eq!(s.general.quarantine_days, 30);
        assert_eq!(s.version, mj_core::settings::SETTINGS_VERSION);
    }

    #[test]
    fn hand_edited_out_of_range_values_are_repaired_on_load() {
        let t = TempDir::new("settings-repair").unwrap();
        let p = t.join("settings.json");
        std::fs::write(&p, br#"{"general":{"undo_days":0,"light_copy_suffix":"a/b"},"performance":{"threads":60000,"temp_dir":"relative"},"optimise":{"keep_if_not_smaller":false},"safety":{"protected_paths":["rel","/ok"]}}"#).unwrap();
        let s = load_from(&p);
        assert_eq!(s.general.undo_days, 30);
        assert_eq!(s.general.light_copy_suffix, "_light");
        assert_eq!(s.performance.threads, MAX_THREADS);
        assert_eq!(s.performance.temp_dir, None);
        assert!(s.optimise.keep_if_not_smaller);
        assert!(s.safety.protected_paths.iter().all(|p| p.is_absolute()));
        assert!(validate(&s).is_ok(), "a repaired value always validates");
    }

    #[test]
    fn validation_names_the_field() {
        let check = |f: &dyn Fn(&mut Settings), needle: &str| {
            let mut s = Settings::default();
            f(&mut s);
            let msg = validate(&s).unwrap_err().to_string();
            assert!(msg.contains(needle), "{msg} should mention {needle}");
        };
        check(&|s| s.general.undo_days = 0, "general.undo_days");
        check(&|s| s.general.quarantine_days = 4000, "general.quarantine_days");
        check(&|s| s.general.light_copy_suffix = "  ".into(), "light_copy_suffix");
        check(&|s| s.general.light_copy_suffix = "x/y".into(), "light_copy_suffix");
        check(&|s| s.performance.threads = 999, "performance.threads");
        check(&|s| s.performance.parallel_jobs = 999, "performance.parallel_jobs");
        check(&|s| s.performance.temp_dir = Some(PathBuf::from("tmp")), "performance.temp_dir");
        check(&|s| s.general.quarantine_dir = Some(PathBuf::from("q")), "general.quarantine_dir");
        check(&|s| s.safety.protected_paths = vec![PathBuf::from("rel")], "absolute");
        check(&|s| s.safety.exclusions = vec!["".into()], "safety.exclusions");
        check(&|s| s.optimise.keep_if_not_smaller = false, "keep_if_not_smaller");
        check(
            &|s| {
                s.tool_paths.insert("ffmpeg".into(), PathBuf::from("ffmpeg"));
            },
            "tool_paths.ffmpeg",
        );
        check(&|s| s.integration.watched_folders = vec![PathBuf::from("x")], "watched_folders");
        check(
            &|s| {
                s.type_groups.insert("a".into(), vec![".mp3".into()]);
            },
            "type_groups.a",
        );
        check(
            &|s| s.general.default_originals = OriginalsPolicy::BackupFolder { folder: PathBuf::from("b") },
            "default_originals",
        );
        assert!(validate(&Settings::default()).is_ok());
    }

    #[test]
    fn settings_set_rejects_without_changing_anything() {
        let t = TempDir::new("settings-rej").unwrap();
        let e = engine_in(&t);
        let mut s = Settings::default();
        s.performance.threads = 999;
        let err = e.settings_set(SettingsSetRequest { settings: s }).unwrap_err();
        assert_eq!(err.code(), "invalid_input");
        assert_eq!(e.settings_get().unwrap(), Settings::default());
        assert!(!t.join("data/settings.json").exists());
    }

    #[test]
    fn normalise_cleans_exclusions_paths_and_extensions() {
        let mut s = Settings::default();
        s.safety.exclusions = vec![" **/.cache ".into(), "".into(), "**/.cache".into()];
        s.safety.protected_paths = vec![abs("a/b"), abs("a/./b"), abs("c")];
        s.type_groups.insert("audio".into(), vec![".MP3".into(), "mp3".into(), " flac".into()]);
        normalise(&mut s);
        assert_eq!(s.safety.exclusions, vec!["**/.cache"]);
        assert_eq!(s.safety.protected_paths, vec![abs("a/b"), abs("c")]);
        assert_eq!(s.type_groups["audio"], vec!["mp3", "flac"]);
    }

    #[test]
    fn protected_paths_are_absolute_deduplicated_and_persisted() {
        let t = TempDir::new("settings-prot").unwrap();
        let e = engine_in(&t);
        let err = e.protected_paths_set(ProtectedPathsSetRequest { user: vec![PathBuf::from("rel")] }).unwrap_err();
        assert_eq!(err.code(), "invalid_input");
        let info = e.protected_paths_set(ProtectedPathsSetRequest { user: vec![abs("x/y"), abs("x/y")] }).unwrap();
        assert_eq!(info.user, vec![abs("x/y")]);
        assert!(!info.builtin.is_empty(), "the locked built-in list is always reported");
        assert_eq!(engine_in(&t).protected_paths_get().unwrap().user, vec![abs("x/y")]);
    }

    #[test]
    fn enabling_the_context_menu_in_portable_mode_is_refused_and_not_stored() {
        let t = TempDir::new("settings-portable").unwrap();
        let mut cfg = EngineConfig::new(t.join("data"), t.join("cache"));
        cfg.portable = true;
        let e = Engine::new(cfg).unwrap();
        let mut s = Settings::default();
        s.integration.context_menu = true;
        let err = e.settings_set(SettingsSetRequest { settings: s }).unwrap_err();
        assert_eq!(err.code(), "unsupported");
        assert!(!e.settings_get().unwrap().integration.context_menu);
    }

    #[test]
    fn context_menu_side_effect_is_skipped_without_persistence() {
        let t = TempDir::new("settings-nosfx").unwrap();
        let mut cfg = EngineConfig::new(t.join("data"), t.join("cache"));
        cfg.persist = false;
        let e = Engine::new(cfg).unwrap();
        let mut s = Settings::default();
        s.integration.context_menu = true;
        assert!(e.settings_set(SettingsSetRequest { settings: s }).unwrap().integration.context_menu);
    }

    #[test]
    fn cache_info_and_clear_answer() {
        let t = TempDir::new("settings-cache").unwrap();
        let e = engine_in(&t);
        assert_eq!(e.cache_info().unwrap().entries, 0);
        assert_eq!(e.cache_clear().unwrap().entries, 0);
    }
}
