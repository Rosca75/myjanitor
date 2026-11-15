//! GUI start-up (W6-A, IPC-2 and IPC-6/XC-2). Decision record: `docs/decisions/W6-A.md`.
//!
//! The engine is built on a worker thread, never on the main thread: opening the
//! journal, loading the scan cache and probing the external tools take time, and the
//! window must paint and answer while they run. Commands that arrive before the
//! engine is ready wait for it on Tauri's blocking pool ([`EngineSlot::wait`]).
//! Expired quarantine items are then purged as an ordinary queued job
//! (`Engine::quarantine_purge_if_due`), never before the window opens.
//!
//! A second start on a data folder another MyJanitor holds (the open app, a CLI
//! command, a scheduled run) does not open a second window: it says so in a native
//! message box and exits. Nothing is printed only to a console, which a Windows GUI
//! process does not have.

use std::sync::{Arc, Condvar, Mutex};

use mj_core::ErrorInfo;
use mj_engine::api::{LaunchIntent, LaunchMode};
use mj_engine::Engine;

/// What the engine slot holds.
enum Slot {
    Pending,
    Ready(Arc<Engine>),
    Failed(ErrorInfo),
}

/// The engine, once the start-up thread has built it.
pub struct EngineSlot {
    slot: Mutex<Slot>,
    ready: Condvar,
}

impl Default for EngineSlot {
    fn default() -> Self {
        EngineSlot { slot: Mutex::new(Slot::Pending), ready: Condvar::new() }
    }
}

impl EngineSlot {
    /// Store the engine (or why there is none) and wake every waiting command.
    pub fn set(&self, engine: Result<Arc<Engine>, ErrorInfo>) {
        let mut s = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        *s = match engine {
            Ok(e) => Slot::Ready(e),
            Err(e) => Slot::Failed(e),
        };
        self.ready.notify_all();
    }

    /// The engine, blocking until the start-up thread has stored it. Call it on a
    /// blocking-pool thread, never on the main thread.
    pub fn wait(&self) -> Result<Arc<Engine>, ErrorInfo> {
        let mut s = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match &*s {
                Slot::Pending => s = self.ready.wait(s).unwrap_or_else(|e| e.into_inner()),
                Slot::Ready(e) => return Ok(Arc::clone(e)),
                Slot::Failed(e) => return Err(e.clone()),
            }
        }
    }

    /// The engine if it is ready (shutdown), without waiting.
    pub fn get(&self) -> Option<Arc<Engine>> {
        match &*self.slot.lock().unwrap_or_else(|e| e.into_inner()) {
            Slot::Ready(e) => Some(Arc::clone(e)),
            _ => None,
        }
    }
}

/// What the shell does once the engine is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    /// Open normally.
    Start,
    /// Tell the user why MyJanitor cannot open, then exit.
    Refuse { title: String, message: String },
}

/// The start-up decision for an engine that does (`Some(why)`) or does not hold the data
/// folder's lock. `intent` is the context-menu request of this launch, if any: the message
/// says it was not carried out, so a click on "Find duplicates here" is never lost silently.
pub fn decide(conflict: Option<&str>, intent: Option<&LaunchIntent>) -> Startup {
    let Some(why) = conflict else { return Startup::Start };
    let mut message = format!(
        "Another MyJanitor is using its data folder ({why}).\n\nOnly one MyJanitor may work on the same files at a \
         time. Switch to the MyJanitor window that is already open, or wait for the command or scheduled run to \
         finish, then start MyJanitor again."
    );
    if let Some(i) = intent {
        let what = match i.mode {
            LaunchMode::Optimise => "Optimise",
            LaunchMode::Duplicates => "Find duplicates",
        };
        message.push_str(&format!("\n\n\"{what}\" for {} was not started.", i.path.display()));
    }
    Startup::Refuse { title: "MyJanitor is already running".into(), message }
}

/// The message when the engine cannot be built at all (an app folder cannot be created).
pub fn failed(e: &mj_core::Error) -> Startup {
    let info = e.info();
    let mut message = format!("MyJanitor could not start: {}", info.message);
    if !info.hint.is_empty() {
        message.push_str(&format!("\n\n{}", info.hint));
    }
    Startup::Refuse { title: "MyJanitor could not start".into(), message }
}

/// The error commands get while the shell is refusing to start (they are never visible:
/// the window is hidden and the app exits after the message box).
pub fn refused_info(message: &str) -> ErrorInfo {
    ErrorInfo { code: "conflict".into(), message: message.to_string(), hint: String::new(), path: None }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn a_free_data_folder_starts_and_a_held_one_is_refused_with_a_message() {
        assert_eq!(decide(None, None), Startup::Start);
        let Startup::Refuse { title, message } = decide(Some("held by process 42"), None) else { panic!("refused") };
        assert!(title.contains("already running"));
        assert!(message.contains("held by process 42"), "{message}");
        assert!(!message.contains("was not started"));
    }

    #[test]
    fn a_refused_context_menu_launch_names_what_was_not_done() {
        let intent = LaunchIntent { mode: LaunchMode::Duplicates, path: PathBuf::from("/home/me/Photos") };
        let Startup::Refuse { message, .. } = decide(Some("held"), Some(&intent)) else { panic!("refused") };
        assert!(message.contains("\"Find duplicates\" for /home/me/Photos was not started"), "{message}");
        // No conflict: the intent is carried out as before.
        assert_eq!(decide(None, Some(&intent)), Startup::Start);
    }

    #[test]
    fn an_engine_that_cannot_be_built_is_reported() {
        let Startup::Refuse { title, message } = failed(&mj_core::Error::Internal("disk full".into())) else {
            panic!("refused")
        };
        assert!(title.contains("could not start") && message.contains("disk full"), "{message}");
    }

    #[test]
    fn commands_wait_for_the_engine_and_then_get_it_or_the_reason() {
        let slot = Arc::new(EngineSlot::default());
        assert!(slot.get().is_none());
        let waiter = {
            let slot = Arc::clone(&slot);
            std::thread::spawn(move || slot.wait().map(|_| ()).unwrap_err())
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        slot.set(Err(refused_info("held")));
        assert_eq!(waiter.join().unwrap().message, "held");
        assert!(slot.get().is_none());
        // A ready engine is handed to every caller.
        let mut cfg = mj_engine::EngineConfig::new(PathBuf::from("unused-data"), PathBuf::from("unused-cache"));
        cfg.persist = false;
        let engine = Engine::new(cfg).unwrap();
        let slot = EngineSlot::default();
        slot.set(Ok(Arc::clone(&engine)));
        assert!(Arc::ptr_eq(&slot.wait().unwrap(), &engine));
        assert!(slot.get().is_some());
    }
}
