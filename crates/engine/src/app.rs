//! `app_info`. Owner: W1 (W3-E may extend `AppInfo` through the orchestrator).

use mj_core::Result;

use crate::api::AppInfo;
use crate::Engine;

impl Engine {
    /// Version, platform, folders, portable and elevation state.
    pub fn app_info(&self) -> Result<AppInfo> {
        Ok(AppInfo {
            name: mj_core::APP_NAME.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            platform: if cfg!(windows) { "windows" } else { "linux" }.into(),
            bundle_id: mj_core::BUNDLE_ID.into(),
            portable: self.cfg.portable,
            data_dir: self.cfg.data_dir.clone(),
            cache_dir: self.cfg.cache_dir.clone(),
            elevated: self.platform().is_elevated(),
            journal_persistent: self.journal.is_persistent(),
        })
    }
}
