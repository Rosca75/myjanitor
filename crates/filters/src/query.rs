//! Quick-query syntax (CLN-ENG-05). Owner: W2-C. **Grammar (frozen, docs/IPC.md §8):**
//!
//! ```text
//! query   := or
//! or      := and ("OR" and)*
//! and     := unary (["AND"] unary)*          // juxtaposition = AND
//! unary   := "-" unary | "NOT" unary | "(" or ")" | term
//! term    := key ":" value | bare
//! key     := ext | name | path | size | modified | created | accessed | type
//!          | attr | owner | depth | pathlen | kind | content | is
//! value   := [op] text        op := "<" | "<=" | ">" | ">=" | "="
//! bare    := text  (name contains, case-insensitive)
//! ```
//! Examples: `ext:mp4 size:>1GB modified:<2024`, `ext:jpg,png -path:*/Photos/*`,
//! `modified:>2y` (older than 2 years: `>` on a *relative age* means older),
//! `is:empty`, `is:hidden`, `type:installers`, `name:"final v2*"`.

use mj_core::{Error, Rule};
use serde::{Deserialize, Serialize};

/// A parse problem at a byte range of the query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryError {
    pub offset: usize,
    pub len: usize,
    pub message: String,
}

/// An autocomplete suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// Text that replaces `replace_len` bytes ending at the cursor.
    pub insert: String,
    pub replace_len: usize,
    /// Shown in the list ("size:>1GB  bigger than 1 GB").
    pub label: String,
}

/// Parse a query into a rule.
pub fn parse(query: &str) -> Result<Rule, Vec<QueryError>> {
    let _ = query;
    Err(vec![QueryError { offset: 0, len: 0, message: Error::not_implemented("query parser (W2-C)").to_string() }])
}

/// Suggestions for the text before `cursor` (byte offset).
pub fn suggest(query: &str, cursor: usize) -> Vec<Suggestion> {
    let _ = (query, cursor);
    Vec::new()
}
