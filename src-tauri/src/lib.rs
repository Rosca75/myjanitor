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
pub mod startup;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

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
            cfg.launch = intent.clone();
            cfg.events = Arc::new(events::TauriSink::new(app.handle().clone()));
            // IPC-2: expired quarantine items are purged by a queued job once the engine
            // is ready, not while the window waits.
            cfg.purge_at_start = false;
            let slot = Arc::new(startup::EngineSlot::default());
            app.manage(commands::AppState { engine: Arc::clone(&slot) });
            app.manage(dialogs::DialogGate::default());
            let handle = app.handle().clone();
            // IPC-2: the engine (journal, scan cache, tool probe) is built off the main
            // thread; commands wait for it on the blocking pool.
            mj_core::pool::spawn_large("mj-start", move || start_engine(handle, cfg, intent, &slot))?;
            Ok(())
        })
        .invoke_handler(mj_handlers!())
        .build(tauri::generate_context!())
        .expect("error while building MyJanitor");
    app.run(move |handle, event| {
        if let tauri::RunEvent::Exit = event {
            // Graceful shutdown: stop work between items, save the scan cache. The
            // journal is synced after every record already.
            if let Some(engine) = handle.try_state::<commands::AppState>().and_then(|s| s.engine.get()) {
                engine.shutdown(SHUTDOWN_WAIT);
            }
        }
    });
}

/// Body of the start-up thread: build the engine, then open normally or refuse with a
/// visible message (F-13, IPC-6).
fn start_engine(
    handle: AppHandle,
    cfg: mj_engine::EngineConfig,
    intent: Option<mj_engine::api::LaunchIntent>,
    slot: &startup::EngineSlot,
) {
    let decision = match mj_engine::Engine::new(cfg) {
        Ok(engine) => {
            // F-13: the engine holds the data folder's lock (shared with the CLI and
            // scheduled runs); a second window on the same folder does not open.
            let decision = startup::decide(engine.instance_conflict(), intent.as_ref());
            if decision == startup::Startup::Start {
                slot.set(Ok(Arc::clone(&engine)));
                // CLN-ACT-04: expired items go through the job queue (the error, for
                // example a journal that cannot be written, is already in app_info).
                let _ = engine.quarantine_purge_if_due();
            }
            decision
        }
        Err(e) => startup::failed(&e),
    };
    if let startup::Startup::Refuse { title, message } = decision {
        slot.set(Err(startup::refused_info(&message)));
        refuse(&handle, &title, &message);
    }
}

/// The answer of the refusal message box.
type Shown = Pin<Box<dyn Future<Output = rfd::MessageDialogResult> + Send>>;

/// Hide the window, show `message` in a native message box and exit once it is closed.
/// Runs on the start-up thread; the box is built on the main thread (GTK requires it).
fn refuse(handle: &AppHandle, title: &str, message: &str) {
    // Terminals still get it (the CLI user who started the GUI from one).
    eprintln!("{title}: {message}");
    if let Some(w) = handle.get_webview_window("main") {
        let _ = w.hide();
    }
    let (tx, rx) = std::sync::mpsc::channel::<Shown>();
    let (title, message) = (title.to_string(), message.to_string());
    let posted = handle.run_on_main_thread(move || {
        let shown = rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&title)
            .set_description(&message)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        let _ = tx.send(Box::pin(shown));
    });
    if posted.is_ok() {
        if let Ok(shown) = rx.recv() {
            tauri::async_runtime::block_on(shown);
        }
    }
    handle.exit(1);
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
