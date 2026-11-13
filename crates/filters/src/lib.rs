//! # mj-filters
//!
//! Metadata filters, the AND/OR/NOT rule engine, saved presets and the
//! quick-query parser (W2-C; CLN-FLT-*, CLN-ENG-01..05, CLN-CNT-01..03).
//!
//! One evaluator serves the rule builder, the search box and presets: the
//! query parser produces a [`mj_core::Rule`], [`eval::compile`] turns any rule
//! into a [`eval::CompiledRule`], and [`eval::select`] (or the richer
//! [`eval::select_with`]) applies it to an index.
//!
//! Module map: [`eval`] (engine, topmost, LargestN), [`matcher`] (name/path
//! text matching), [`groups`] (file-type groups), [`content`] (full-text and
//! mismatch reads, extractor trait), [`media`] (media-metadata conditions),
//! [`exclusions`] (global exclusions), [`presets`], [`query`] (quick search
//! syntax) and [`timeutil`] (overflow-safe date helpers).
#![forbid(unsafe_code)]

pub mod content;
pub mod eval;
pub mod exclusions;
pub mod groups;
pub mod matcher;
pub mod media;
pub mod presets;
pub mod query;
pub mod timeutil;

pub use eval::{
    compile, compile_with, select, select_with, CompiledRule, FilterContext, FilterDiagnostics, SelectOptions,
    Selection,
};
pub use exclusions::Exclusions;
pub use query::{parse, suggest, QueryError, Suggestion};
