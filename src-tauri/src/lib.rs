//! MyJanitor GUI shell (Tauri 2). Owner: W3-D.
//!
//! The frontend is the plain `ui/` folder (no bundler, `withGlobalTauri`); all
//! file I/O happens in Rust. This crate wires commands and events to `mj-engine`
//! and answers the two shell commands that need the window: the native file
//! dialogs (`dialogs.rs`).

pub mod commands;
pub mod dialogs;
pub mod events;
pub mod launch;

use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;

/// How long shutdown waits for a running scan or job to stop between items.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// Start the app. The engine is built with the platform's app folders and an empty
/// scope: nothing is scanned or changed until the user asks.
pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    let launch = launch::parse(&args, &cwd);
    let (data_dir, cache_dir, portable) = match &launch.data_dir {
        Some(root) => {
            let d = mj_engine::paths::AppDirs::under(root);
            (d.data, d.cache, false)
        }
        None => mj_engine::paths::default_dirs(),
    };
    let intent = launch.intent;
    let app = tauri::Builder::default()
        .setup(move |app| {
            let mut cfg = mj_engine::EngineConfig::new(data_dir, cache_dir);
            cfg.portable = portable;
            cfg.launch = intent;
            cfg.events = Arc::new(events::TauriSink::new(app.handle().clone()));
            let engine = mj_engine::Engine::new(cfg).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
            // F-13: the engine holds the data folder's lock (shared with the CLI and
            // scheduled runs); a second window on the same folder does not open.
            if let Some(msg) = engine.instance_conflict() {
                eprintln!("MyJanitor is already running: {msg}");
                std::process::exit(1);
            }
            app.manage(commands::AppState { engine });
            app.manage(dialogs::DialogGate::default());
            Ok(())
        })
        .invoke_handler(mj_handlers!())
        .build(tauri::generate_context!())
        .expect("error while building MyJanitor");
    app.run(move |handle, event| {
        if let tauri::RunEvent::Exit = event {
            // Graceful shutdown: stop work between items, save the scan cache. The
            // journal is synced after every record already.
            if let Some(state) = handle.try_state::<commands::AppState>() {
                state.engine.shutdown(SHUTDOWN_WAIT);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_command_is_registered() {
        let api = mj_engine::api::COMMANDS.iter().map(|c| c.name);
        let shell = mj_engine::api::SHELL_COMMANDS.iter().map(|c| c.name);
        let expected: Vec<&str> = api.chain(shell).collect();
        assert_eq!(super::commands::REGISTERED, expected.as_slice());
    }

    /// `REGISTERED` is a hand-kept list; this ties it to what `mj_handlers!` really
    /// registers, so a command cannot be listed without a handler or the reverse.
    #[test]
    fn the_handler_list_matches_the_registered_names() {
        let src = include_str!("commands.rs");
        let body = src.split("tauri::generate_handler![").nth(1).expect("handler list").split(']').next().unwrap();
        let handlers: Vec<&str> = body
            .lines()
            .filter_map(|l| l.trim().trim_end_matches(',').rsplit("::").next())
            .filter(|n| !n.is_empty())
            .collect();
        assert_eq!(handlers, super::commands::REGISTERED);
    }
}
