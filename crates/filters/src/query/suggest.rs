//! Autocomplete for the quick-query box. Owner: W2-C.

use super::values::{value_hints, KEYS};
use super::Suggestion;

/// Most suggestions returned.
pub const MAX_SUGGESTIONS: usize = 20;

/// Suggestions for the text before `cursor` (byte offset; clamped to the query
/// and moved back to a character boundary, so any cursor value is safe).
///
/// The token under the cursor starts after the last whitespace or `(` that is
/// not inside an open quote. Without a `:` it completes keys (and `OR`/`AND`);
/// after `key:` it offers example values for that key, e.g. `size:>1GB`.
pub fn suggest(query: &str, cursor: usize) -> Vec<Suggestion> {
    let mut end = cursor.min(query.len());
    while !query.is_char_boundary(end) {
        end -= 1;
    }
    let before = &query[..end];
    let mut start = 0;
    let mut quote: Option<char> = None;
    for (i, c) in before.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, c) if c.is_whitespace() || c == '(' => start = i + c.len_utf8(),
            _ => {}
        }
    }
    if quote.is_some() {
        return Vec::new();
    }
    let token = &before[start..];
    let neg = usize::from(token.starts_with('-'));
    let token = &token[neg..];
    let mut out = Vec::new();
    match token.split_once(':') {
        None => {
            let t = token.to_lowercase();
            for (key, insert, label) in KEYS {
                if key.starts_with(&t) {
                    out.push(Suggestion {
                        insert: insert.to_string(),
                        replace_len: token.len(),
                        label: label.to_string(),
                    });
                }
            }
            if !token.is_empty() {
                for kw in ["OR", "AND", "NOT"] {
                    if kw.starts_with(&token.to_uppercase()) && token.chars().all(|c| c.is_ascii_alphabetic()) {
                        out.push(Suggestion {
                            insert: kw.to_string(),
                            replace_len: token.len(),
                            label: format!("{kw}  combine terms"),
                        });
                    }
                }
            }
        }
        Some((key, value)) => {
            let key = key.to_lowercase();
            // Keep a typed operator; complete only the text after it.
            let op_len = value.chars().take_while(|c| matches!(c, '<' | '>' | '=')).count();
            let typed = value[op_len..].to_lowercase();
            let typed_op = &value[..op_len];
            for (v, what) in value_hints(&key) {
                let bare = v.trim_start_matches(['<', '>', '=']);
                let hint_op = &v[..v.len() - bare.len()];
                let wrong_op = op_len > 0 && ((!hint_op.is_empty() && hint_op != typed_op) || bare.contains(".."));
                if !bare.to_lowercase().starts_with(&typed) || wrong_op {
                    continue;
                }
                let insert = if op_len > 0 { format!("{typed_op}{bare}") } else { v.clone() };
                out.push(Suggestion { insert, replace_len: value.len(), label: format!("{key}:{v}  {what}") });
            }
        }
    }
    out.truncate(MAX_SUGGESTIONS);
    out
}
