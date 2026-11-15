//! The command and event API shared by the Tauri shell, the CLI and the mock
//! backend. **Frozen contract**: `docs/IPC.md` documents every entry of
//! [`COMMANDS`], [`SHELL_COMMANDS`] and [`EVENTS`]; a test keeps them in sync.

pub mod commands;
pub mod events;
pub mod types;

pub use commands::{CommandSpec, EventSpec, ShellCommandSpec, COMMANDS, EVENTS, SHELL_COMMANDS};
pub use events::{CollectSink, Event, EventSink, NullSink};
pub use types::*;
