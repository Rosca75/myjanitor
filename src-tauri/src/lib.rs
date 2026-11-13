//! MyJanitor GUI shell (Tauri 2). Owner: W3-D.
//!
//! The frontend is the plain `ui/` folder (no bundler, `withGlobalTauri`); all
//! file I/O happens in Rust. This crate only wires commands and events to
//! `mj-engine`.

pub mod commands;
pub mod events;

use std::sync::Arc;

use tauri::Manager;

/// Start the app.
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let (data_dir, cache_dir, portable) = mj_engine::paths::default_dirs();
            let mut cfg = mj_engine::EngineConfig::new(data_dir, cache_dir);
            cfg.portable = portable;
            cfg.events = Arc::new(events::TauriSink::new(app.handle().clone()));
            let engine = mj_engine::Engine::new(cfg).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
            app.manage(commands::AppState { engine });
            Ok(())
        })
        .invoke_handler(mj_handlers!())
        .run(tauri::generate_context!())
        .expect("error while running MyJanitor");
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_engine_command_is_registered() {
        let engine: Vec<&str> = mj_engine::api::COMMANDS.iter().map(|c| c.name).collect();
        assert_eq!(super::commands::REGISTERED, engine.as_slice());
    }
}
