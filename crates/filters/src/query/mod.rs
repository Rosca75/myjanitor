//! Quick-query syntax (CLN-ENG-05). Owner: W2-C. **Grammar (frozen, docs/IPC.md §6):**
//!
//! ```text
//! query   := or
//! or      := and ("OR" and)*
//! and     := unary (["AND"] unary)*          // juxtaposition = AND
//! unary   := "-" unary | "NOT" unary | "(" or ")" | term
//! term    := key ":" value | bare
//! key     := ext | name | path | size | modified | created | accessed | older | newer
//!          | type | attr | owner | depth | pathlen | kind | content | is | regex | top
//! value   := [op] item ("," item)*           // a list means "any of"
//! op      := "<" | "<=" | ">" | ">=" | "="
//! item    := word | "quoted text" | 'quoted text' | /regex/
//! bare    := item                             // file name contains / wildcard / regex
//! ```
//!
//! Keywords `OR`, `AND` and `NOT` are upper case only (`or` is a word). A `-`
//! directly in front of a term negates it (`-ext:tmp`). Parentheses group; put a
//! space before `(` and quote values that contain parentheses.
//!
//! | Term | Meaning |
//! |---|---|
//! | `ext:mp4`, `ext:jpg,png` | extension in the list (`.` and `*.` prefixes ignored) |
//! | `name:*.tmp`, `name:final`, `name:=exact`, `name:/^img_\d+/` | name wildcard (`*`/`?`), contains, exact (`=`), regex |
//! | `path:*/Photos/*`, `path:Photos` | same on the full path; a wildcard without a separator matches anywhere in the path |
//! | `size:>1GB`, `size:<=500k`, `size:1MB..10MB`, `size:=0` | bytes (binary units B/KB/MB/GB/TB, decimals ok); `=0` is the zero-byte condition |
//! | `modified:<2024`, `>=2024-03`, `2024-03-15`, `2023..2024` | absolute dates compare with the period named (`<2024` before 2024-01-01, `>2024` after 2024, `2024` within 2024) |
//! | `modified:>2y`, `<30d`, `30d`, `6m` | relative values are *ages* (`>2y` older than two years, `<30d` newer than 30 days, bare `30d` within the last 30 days; `m` = months) |
//! | `created:`, `accessed:` | as `modified:` |
//! | `older:90d`, `newer:7d` | shorthands for `modified:>90d` / `modified:<7d` (also take absolute dates) |
//! | `type:video` | file-type group (case-insensitive; custom groups checked when parsed with groups) |
//! | `attr:hidden`, `attr:!hidden` | attribute set / clear (hidden, system, readonly, compressed, sparse, encrypted, offline...) |
//! | `is:empty`, `is:hidden`, `is:invalid`, `is:mismatch`, `is:dir` | zero-byte, attribute, bad Windows name, extension mismatch, kind |
//! | `kind:file`, `owner:alice`, `depth:>3`, `pathlen:>240`, `content:invoice`, `top:50` | kind, owner, depth, path length, text inside the file, keep the N largest |
//! | `regex:/…/` or `regex:^img_\d+` | file-name regex (`pathregex:` for paths) |
//!
//! Free words are file-name conditions. Case is ignored everywhere in queries.
//! `top:N` limits the whole result, so it is only valid under top-level ANDs.
//! Every error carries the byte `offset` and `len` of the offending text.

mod lexer;
mod parser;
mod suggest;
mod units;
mod values;

use std::collections::BTreeMap;

use mj_core::Rule;
use serde::{Deserialize, Serialize};

pub use lexer::{Op, MAX_QUERY_BYTES};
pub use parser::MAX_NESTING;
pub use suggest::suggest;
pub use units::{parse_datespec, parse_size, parse_span, DateSpec};

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

/// Parse a query into a rule (`type:` group names are checked later, by `compile`).
pub fn parse(query: &str) -> Result<Rule, Vec<QueryError>> {
    parser::parse_query(query, &values::Env { groups: None })
}

/// Like [`parse`], but `type:` values are validated (and canonicalised) against `groups`.
pub fn parse_with_groups(query: &str, groups: &BTreeMap<String, Vec<String>>) -> Result<Rule, Vec<QueryError>> {
    parser::parse_query(query, &values::Env { groups: Some(groups) })
}

#[cfg(test)]
mod tests;
