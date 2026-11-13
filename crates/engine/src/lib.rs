//! # mj-engine
//!
//! The facade both front ends use (GUI through Tauri, `myjanitor-cli`
//! directly): session state, scan orchestration, opportunity building,
//! plan, dry run, job queue, history, settings, schedules, reports.
//!
//! The command and event API is plain Rust in [`api`] (serde types); the
//! Tauri shell and the CLI are thin adapters over [`Engine`] methods named
//! exactly like the commands in [`api::COMMANDS`] (see `docs/IPC.md`).
#![forbid(unsafe_code)]

pub mod api;
mod app;
mod apply;
mod disk;
mod engine;
mod fsbrowse;
mod history;
mod jobs;
mod opportunities;
pub mod paths;
mod plan;
mod presets;
mod reports;
mod results;
mod scan_ops;
mod schedules;
mod scope;
mod search;
mod settings;
mod steps;
mod store;
mod tools;

pub use engine::{Engine, EngineConfig};

#[cfg(test)]
mod tests;
