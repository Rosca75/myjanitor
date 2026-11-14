//! External tools (W3-C): the engine-side listing behind `tools_list` and
//! `tools_refresh` (IPC.md). Detection itself lives in `mj_optimise::tools`;
//! this module feeds it the manual paths from the settings and keeps the
//! result the optimisers' `availability()` checks read.

use mj_core::{lock, ExternalTool, Result};

use crate::Engine;

/// Detect the tools: `settings.tool_paths` (a file, or a folder to look in) win over `PATH`.
pub(crate) fn load(engine: &Engine) {
    let overrides = lock(&engine.settings).tool_paths.clone();
    *lock(&engine.tools) = mj_optimise::tools::detect(&overrides);
}

impl Engine {
    /// Detected tools, as of the last detection.
    pub fn tools_list(&self) -> Result<Vec<ExternalTool>> {
        Ok(lock(&self.tools).clone())
    }

    /// Detect again, forgetting cached version probes (after the user installed a tool or edited
    /// `settings.tool_paths`).
    pub fn tools_refresh(&self) -> Result<Vec<ExternalTool>> {
        mj_optimise::tools::clear_cache();
        load(self);
        self.tools_list()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mj_core::testutil::TempDir;

    use crate::api::*;
    use crate::{Engine, EngineConfig};

    fn engine() -> Arc<Engine> {
        let t = std::env::temp_dir();
        let mut cfg = EngineConfig::new(t.join("unused-data"), t.join("unused-cache"));
        cfg.persist = false;
        Engine::new(cfg).unwrap()
    }

    #[test]
    fn list_has_every_known_tool_with_a_consistent_found_flag() {
        let tools = engine().tools_list().unwrap();
        assert_eq!(tools.len(), mj_core::tools::KNOWN_TOOLS.len());
        assert!(tools.iter().all(|t| t.found == t.path.is_some() && !t.id.is_empty()));
    }

    #[cfg(unix)]
    #[test]
    fn refresh_picks_up_a_manual_path_from_settings() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("engine-tools").unwrap();
        let exe = dir.write("my-oxipng", b"#!/bin/sh\necho 'oxipng 9.1.1'\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = engine();
        let mut s = e.settings_get().unwrap();
        s.tool_paths.insert("oxipng".into(), exe.clone());
        e.settings_set(SettingsSetRequest { settings: s }).unwrap();
        let tools = e.tools_refresh().unwrap();
        let ox = tools.iter().find(|t| t.id == "oxipng").unwrap();
        assert!(ox.found);
        assert_eq!(ox.path.as_deref(), Some(exe.as_path()));
        assert_eq!(ox.version.as_deref(), Some("9.1.1"));
        assert_eq!(e.tools_list().unwrap(), tools);
    }
}
