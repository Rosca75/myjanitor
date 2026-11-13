//! Rule compilation and evaluation. Owner: W2-C.
//!
//! Semantics (frozen): `all([])` matches everything, `any([])` nothing; a
//! missing timestamp never satisfies a date condition; `TextMode::Wildcard`
//! uses `mj_core::glob` (name mode for `Name`, path mode for `Path`); regexes
//! are compiled once (`regex` crate) and invalid ones are `Error::InvalidInput`;
//! `LargestN` is applied after the rest of the tree; `topmost_only` keeps a
//! matching folder and drops matches below it (CLN-ENG-04).

use std::collections::BTreeMap;

use mj_core::{Error, FileEntry, FileIndex, Result, Rule, Timestamp};

/// Evaluation context.
pub struct FilterContext<'a> {
    /// "Now" for relative dates.
    pub now: Timestamp,
    /// File-type groups (`Settings::type_groups`).
    pub type_groups: &'a BTreeMap<String, Vec<String>>,
}

/// A rule ready for fast evaluation.
#[derive(Debug)]
pub struct CompiledRule {
    rule: Rule,
}

impl CompiledRule {
    /// The source rule.
    pub fn rule(&self) -> &Rule {
        &self.rule
    }

    /// Per-entry test (ignores `LargestN`, which needs the whole result).
    pub fn matches(&self, entry: &FileEntry) -> bool {
        let _ = entry;
        false
    }
}

/// Compile a rule (validates regexes, groups and `LargestN` placement).
pub fn compile(rule: &Rule, ctx: &FilterContext<'_>) -> Result<CompiledRule> {
    let _ = (rule, ctx);
    Err(Error::not_implemented("rule engine (W2-C)"))
}

/// Positions of the index entries matching `rule`, in index order.
pub fn select(index: &FileIndex, rule: &Rule, ctx: &FilterContext<'_>, topmost_only: bool) -> Result<Vec<usize>> {
    let _ = (index, rule, ctx, topmost_only);
    Err(Error::not_implemented("rule engine (W2-C)"))
}
