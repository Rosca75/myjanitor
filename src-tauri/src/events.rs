//! Forwards engine events to the webview (`app.emit(name, payload)`). Owner: W3-D.
//! Emit failures are ignored: they only happen while the window is closing.

use mj_engine::api::{Event, EventSink};
use tauri::{AppHandle, Emitter};

/// Event sink backed by the Tauri app handle.
pub struct TauriSink {
    app: AppHandle,
}

impl TauriSink {
    /// Sink for `app`.
    pub fn new(app: AppHandle) -> Self {
        TauriSink { app }
    }
}

impl EventSink for TauriSink {
    fn emit(&self, event: &Event) {
        let _ = self.app.emit(event.name(), event.payload());
    }
}
