//! The command and event API shared by the Tauri shell, the CLI and the mock
//! backend. **Frozen contract**: `docs/IPC.md` documents every entry of
//! [`COMMANDS`] and [`EVENTS`]; a test keeps the three in sync.

pub mod commands;
pub mod events;
pub mod types;

pub use commands::{CommandSpec, EventSpec, COMMANDS, EVENTS};
pub use events::{CollectSink, Event, EventSink, NullSink};
pub use types::*;
