//! # mj-actions
//!
//! The only crate that changes user files (W2-E). Every destructive path goes
//! through [`engine::execute`], which for each [`mj_core::ActionItem`]:
//!
//! 1. checks the [`guard::Guard`]: inside a caller-supplied allowed root,
//!    not protected, no link/junction on the path below the root;
//! 2. re-stats the item and compares it with the scan stamp (TOCTOU guard);
//! 3. writes a journal `intent` record and syncs it (**journal before act**);
//! 4. acts through `mj_platform::ActionPlatform` / `std::fs`;
//! 5. writes the `done` / `failed` record.
//!
//! Undo, quarantine purge, crash recovery, safety inspection (signed,
//! labelled, encrypted, synced), keep-rule selection and report/audit export
//! live beside it.
#![forbid(unsafe_code)]

pub mod engine;
pub mod fsops;
pub mod guard;
pub mod journal;
pub mod protected;
pub mod quarantine;
pub mod report;
pub mod retain;
pub mod safety;
pub mod select;
pub mod undo;

pub use engine::{dry_run, execute, execute_with, ExecContext, ExecOptions, Services, TrashRoute};
pub use guard::Guard;
pub use journal::Journal;
pub use protected::ProtectedPaths;
pub use safety::SafetyInspector;
