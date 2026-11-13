//! Settings, protected paths and the scan cache (Settings drawer). Owner: W3-E.
//! Stored in `<data>/settings.json`. Side effects of a change (context-menu
//! registration AUT-03, watcher OPT-FSC-03) are W3-E's.

use mj_core::{lock, Error, Result, Settings};

use crate::api::{CacheInfo, ProtectedPathsInfo, ProtectedPathsSetRequest, SettingsSetRequest};
use crate::Engine;

const FILE: &str = "settings.json";

pub(crate) fn load(engine: &Engine) {
    if engine.cfg.persist {
        if let Some(s) = crate::store::read_json::<Settings>(&engine.cfg.data_dir.join(FILE)) {
            *lock(&engine.settings) = s;
        }
    }
}

fn validate(s: &Settings) -> Result<()> {
    if s.general.undo_days == 0 || s.general.undo_days > 3650 {
        return Err(Error::InvalidInput("undo_days must be between 1 and 3650".into()));
    }
    if s.general.quarantine_days == 0 || s.general.quarantine_days > 3650 {
        return Err(Error::InvalidInput("quarantine_days must be between 1 and 3650".into()));
    }
    if s.general.light_copy_suffix.trim().is_empty() || s.general.light_copy_suffix.contains(['/', '\\']) {
        return Err(Error::InvalidInput("the light-copy suffix must be a non-empty name fragment".into()));
    }
    if !s.optimise.keep_if_not_smaller {
        return Err(Error::InvalidInput("keep-if-not-smaller cannot be turned off".into()));
    }
    Ok(())
}

impl Engine {
    fn save_settings(&self, s: &Settings) -> Result<()> {
        if self.cfg.persist {
            crate::store::write_json(&self.cfg.data_dir.join(FILE), s)?;
        }
        Ok(())
    }

    /// All settings.
    pub fn settings_get(&self) -> Result<Settings> {
        Ok(lock(&self.settings).clone())
    }

    /// Replace settings.
    pub fn settings_set(&self, req: SettingsSetRequest) -> Result<Settings> {
        let mut s = req.settings;
        validate(&s)?;
        s.version = mj_core::settings::SETTINGS_VERSION;
        self.save_settings(&s)?;
        *lock(&self.settings) = s.clone();
        Ok(s)
    }

    /// Built-in and user protected paths.
    pub fn protected_paths_get(&self) -> Result<ProtectedPathsInfo> {
        Ok(ProtectedPathsInfo {
            builtin: mj_actions::protected::builtin_paths(),
            builtin_patterns: mj_actions::protected::builtin_patterns(),
            user: lock(&self.settings).safety.protected_paths.clone(),
        })
    }

    /// Replace the user's protected paths.
    pub fn protected_paths_set(&self, req: ProtectedPathsSetRequest) -> Result<ProtectedPathsInfo> {
        if let Some(p) = req.user.iter().find(|p| !p.is_absolute()) {
            return Err(Error::InvalidInput(format!("protected paths must be absolute: {}", p.display())));
        }
        let s = {
            let mut s = lock(&self.settings);
            s.safety.protected_paths = req.user;
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
