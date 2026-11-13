//! # mj-filters
//!
//! Metadata filters, the AND/OR/NOT rule engine, saved presets and the
//! quick-query parser (W2-C; CLN-FLT-*, CLN-ENG-01/02/04/05, CLN-CNT-01/02).
//!
//! One evaluator serves the rule builder, the search box and presets: the
//! query parser produces a [`mj_core::Rule`], [`eval::compile`] turns any rule
//! into a [`eval::CompiledRule`], and [`eval::select`] applies it to an index.
#![forbid(unsafe_code)]

pub mod content;
pub mod eval;
pub mod groups;
pub mod presets;
pub mod query;

pub use eval::{compile, select, CompiledRule, FilterContext};
pub use query::{parse, suggest, QueryError, Suggestion};
