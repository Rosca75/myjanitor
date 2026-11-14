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
            capabilities: capabilities(self.cfg.portable),
            launch: self.cfg.launch.clone(),
        })
    }
}

/// Platform features of this build (`AppInfo::capabilities`). Portable mode never writes
/// outside its folder, so it offers no context menu and no OS schedule.
fn capabilities(portable: bool) -> Vec<String> {
    let mut c: Vec<&str> = if cfg!(windows) {
        vec!["recycle_bin", "wof_compression", "cloud_dehydrate", "hardlinks"]
    } else {
        vec!["freedesktop_trash", "btrfs_compression", "reflinks", "hardlinks"]
    };
    if !portable {
        c.extend(["context_menu", "os_schedules"]);
    }
    c.into_iter().map(String::from).collect()
}
