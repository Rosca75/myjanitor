//! Turns one parsed term (`key:value`) into a `Rule`. Owner: W2-C.
//!
//! The key table below is also what `suggest` offers. See the grammar in
//! `query/mod.rs` for the meaning of each key.

use std::collections::BTreeMap;

use mj_core::time::DateRange;
use mj_core::{Attributes, Cmp, Condition, DateField, DateValue, EntryKind, Rule, TextMode, Timestamp};

use super::lexer::{Item, Op, Span, Word};
use super::units::{parse_datespec, parse_size, parse_u32, span_days, DateSpec};
use super::QueryError;
use crate::groups;
use crate::matcher;

/// `(key, text inserted by autocomplete, label)`.
pub const KEYS: &[(&str, &str, &str)] = &[
    ("ext", "ext:", "ext:mp4,mkv  file extension"),
    ("name", "name:", "name:*.tmp  file name (wildcard, text or /regex/)"),
    ("path", "path:", "path:*/Downloads/*  full path"),
    ("size", "size:>", "size:>1GB  bigger than 1 GB"),
    ("modified", "modified:<", "modified:<2024  last modified"),
    ("created", "created:<", "created:<2024  created"),
    ("accessed", "accessed:<", "accessed:>1y  last accessed"),
    ("older", "older:", "older:90d  not modified for 90 days"),
    ("newer", "newer:", "newer:7d  modified in the last 7 days"),
    ("type", "type:", "type:video  file-type group"),
    ("attr", "attr:", "attr:hidden  attribute (!name = not set)"),
    ("is", "is:", "is:empty  empty, hidden, invalid, mismatch..."),
    ("kind", "kind:", "kind:dir  file, dir or link"),
    ("owner", "owner:", "owner:alice  file owner"),
    ("depth", "depth:>", "depth:>3  folder depth below the scan root"),
    ("pathlen", "pathlen:>", "pathlen:>240  full path length"),
    ("content", "content:", "content:invoice  text inside the file"),
    ("regex", "regex:", "regex:/^img_\\d+/  file name regex"),
    ("top", "top:", "top:50  only the 50 largest results"),
];

/// Values offered after `key:`.
pub fn value_hints(key: &str) -> Vec<(String, String)> {
    let v = |a: &str, b: &str| (a.to_string(), b.to_string());
    match key {
        "size" => vec![
            v(">1GB", "bigger than 1 GB"),
            v(">100MB", "bigger than 100 MB"),
            v("<1MB", "smaller than 1 MB"),
            v("1MB..10MB", "between"),
            v("=0", "empty files"),
        ],
        "modified" | "created" | "accessed" => vec![
            v("<2024", "before 2024"),
            v(">2y", "older than 2 years"),
            v(">90d", "older than 90 days"),
            v("<30d", "within 30 days"),
            v("2024-03", "in March 2024"),
        ],
        "older" | "newer" => {
            vec![v("30d", "30 days"), v("90d", "90 days"), v("6m", "6 months"), v("1y", "1 year"), v("2y", "2 years")]
        }
        "attr" => Attributes::ALL.iter().map(|(_, n)| v(n, "attribute")).collect(),
        "is" => vec![
            v("empty", "zero-byte file"),
            v("hidden", "hidden"),
            v("invalid", "name not portable to Windows"),
            v("mismatch", "content differs from extension"),
            v("file", "files only"),
            v("dir", "folders only"),
        ],
        "kind" => vec![v("file", "files"), v("dir", "folders"), v("link", "links")],
        "type" => groups::builtin().keys().map(|k| (k.clone(), "file-type group".to_string())).collect(),
        "ext" => ["jpg", "png", "mp4", "mkv", "pdf", "docx", "zip", "iso", "tmp", "log"]
            .iter()
            .map(|e| v(e, "extension"))
            .collect(),
        "depth" | "pathlen" => vec![v(">3", "greater than 3"), v("<=2", "at most 2")],
        "top" => vec![v("10", "10 largest"), v("50", "50 largest"), v("100", "100 largest")],
        _ => Vec::new(),
    }
}

/// Parse context.
pub struct Env<'a> {
    /// When set, `type:` values are validated against these groups.
    pub groups: Option<&'a BTreeMap<String, Vec<String>>>,
}

fn err(span: Span, msg: impl Into<String>) -> QueryError {
    span.err(msg)
}

fn cond(c: Condition) -> Rule {
    Rule::Cond(c)
}

fn any_of(mut rules: Vec<Rule>) -> Rule {
    if rules.len() == 1 {
        rules.remove(0)
    } else {
        Rule::Any(rules)
    }
}

fn all_of(rules: Vec<Rule>) -> Rule {
    Rule::All(rules)
}

fn non_empty(it: &Item, what: &str) -> Result<(), QueryError> {
    if it.text.trim().is_empty() {
        Err(err(it.span, format!("{what} is empty")))
    } else {
        Ok(())
    }
}

fn no_op(w: &Word) -> Result<(), QueryError> {
    match w.op {
        Some((op, span)) => Err(err(span, format!("'{}' does not take the operator '{}'", key_name(w), op.text()))),
        None => Ok(()),
    }
}

fn key_name(w: &Word) -> &str {
    w.key.as_ref().map(|k| k.0.as_str()).unwrap_or("this term")
}

fn cmp_of(op: Option<Op>) -> Cmp {
    match op {
        Some(Op::Lt) => Cmp::Lt,
        Some(Op::Le) => Cmp::Le,
        Some(Op::Gt) => Cmp::Gt,
        Some(Op::Ge) => Cmp::Ge,
        Some(Op::Eq) | None => Cmp::Eq,
    }
}

/// Distance-bounded "did you mean" for unknown keys.
fn closest_key(k: &str) -> Option<&'static str> {
    fn dist(a: &str, b: &str) -> usize {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut cur = vec![i];
            for j in 1..=b.len() {
                let cost = usize::from(a[i - 1] != b[j - 1]);
                cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
            }
            prev = cur;
        }
        prev[b.len()]
    }
    KEYS.iter().map(|(n, _, _)| (*n, dist(k, n))).filter(|(_, d)| *d <= 2).min_by_key(|(_, d)| *d).map(|(n, _)| n)
}

/// Build the rule of one term. The second value is the span of a `top:N` term
/// (which the parser must keep at the top-level AND).
pub fn build_term(w: &Word, env: &Env<'_>) -> Result<(Rule, Option<Span>), QueryError> {
    let Some((key, key_span)) = &w.key else {
        // Free word: file name contains (or wildcard / regex).
        return Ok((any_of(w.items.iter().map(|i| text_cond(i, w.op, false, false)).collect::<Result<_, _>>()?), None));
    };
    for it in &w.items {
        if it.text.is_empty() && !it.quoted {
            let what = if w.items.len() > 1 { "a value in this list is missing" } else { "this key needs a value" };
            return Err(err(
                if w.items.len() > 1 { it.span } else { *key_span },
                format!("{what} (e.g. {})", example(key)),
            ));
        }
    }
    let rule = match key.as_str() {
        "ext" | "extension" => ext(w)?,
        "name" => each(w, |i| text_cond(i, w.op, false, false))?,
        "path" => each(w, |i| text_cond(i, w.op, true, false))?,
        "regex" => each(w, |i| text_cond(i, w.op, false, true))?,
        "pathregex" => each(w, |i| text_cond(i, w.op, true, true))?,
        "size" => each(w, |i| size(w, i))?,
        "modified" | "mod" => each(w, |i| date(w, i, DateField::Modified))?,
        "created" => each(w, |i| date(w, i, DateField::Created))?,
        "accessed" => each(w, |i| date(w, i, DateField::Accessed))?,
        "older" => each(w, |i| age_key(w, i, true))?,
        "newer" => each(w, |i| age_key(w, i, false))?,
        "type" | "group" => each(w, |i| type_group(w, i, env))?,
        "attr" | "attribute" => each(w, |i| attr(w, i))?,
        "owner" | "user" => each(w, |i| owner(w, i))?,
        "depth" => each(w, |i| number(w, i, true))?,
        "pathlen" | "path_length" => each(w, |i| number(w, i, false))?,
        "kind" => each(w, |i| kind(w, i))?,
        "content" | "text" => each(w, |i| content(w, i))?,
        "is" => each(w, |i| is(w, i))?,
        "top" | "largest" => return top(w).map(|(r, s)| (r, Some(s))),
        other => {
            let hint = match closest_key(other) {
                Some(k) => format!("did you mean '{k}:'?"),
                None => "keys: ext, name, path, size, modified, created, accessed, older, newer, type, attr, is, kind, owner, depth, pathlen, content, regex, top".into(),
            };
            return Err(err(
                *key_span,
                format!(
                    "unknown key '{other}'; {hint} (quote the text to search for it literally, e.g. \"{other}:x\")"
                ),
            ));
        }
    };
    Ok((rule, None))
}

fn example(key: &str) -> &'static str {
    KEYS.iter().find(|(k, _, _)| *k == key).map(|(_, _, l)| l.split("  ").next().unwrap_or("")).unwrap_or("key:value")
}

/// Apply `f` to each value; several values mean "any of".
fn each(w: &Word, f: impl Fn(&Item) -> Result<Rule, QueryError>) -> Result<Rule, QueryError> {
    Ok(any_of(w.items.iter().map(f).collect::<Result<Vec<_>, _>>()?))
}

fn ext(w: &Word) -> Result<Rule, QueryError> {
    no_op(w)?;
    let mut values = Vec::new();
    for it in &w.items {
        let e = groups::normalise_ext(&it.text);
        if e.is_empty() || e.contains(['/', '\\', ' ', '*', '?']) {
            return Err(err(it.span, format!("'{}' is not a file extension; write ext:mp4 or ext:jpg,png", it.text)));
        }
        values.push(e);
    }
    Ok(cond(Condition::Extension { values }))
}

/// A name / path condition. `force_regex` is the `regex:` key.
fn text_cond(it: &Item, op: Option<(Op, Span)>, path: bool, force_regex: bool) -> Result<Rule, QueryError> {
    let op = op.map(|(o, _)| o);
    non_empty(it, "this value")?;
    let field = |mode: TextMode, pattern: String| {
        if path {
            cond(Condition::Path { mode, pattern, case_sensitive: false })
        } else {
            cond(Condition::Name { mode, pattern, case_sensitive: false })
        }
    };
    if !matches!(op, None | Some(Op::Eq)) {
        let o = op.map(Op::text).unwrap_or("");
        return Err(err(it.span, format!("the operator '{o}' only works with size, dates, depth and path length")));
    }
    // Regex: `/…/` for names, `/…/` with a regex metacharacter for paths, always for `regex:`.
    let looks_regex =
        |inner: &str| !path || (!it.quoted && inner.contains(['\\', '^', '$', '|', '(', ')', '{', '}', '+']));
    let pattern = match (&it.regex, force_regex) {
        (Some(inner), true) => Some(inner.clone()),
        (Some(inner), false) if looks_regex(inner) => Some(inner.clone()),
        (None, true) => Some(it.text.clone()),
        _ => None,
    };
    if let Some(p) = pattern {
        matcher::build_regex(&p, false).map_err(|e| err(it.span, e))?;
        return Ok(field(TextMode::Regex, p));
    }
    let text = it.text.clone();
    if op == Some(Op::Eq) {
        return Ok(field(TextMode::Exact, text));
    }
    if text.contains(['*', '?']) {
        // A separator-free path wildcard means "anywhere in the path".
        let p = if path && !text.contains(['/', '\\']) {
            let tail = if text.ends_with('*') { "**" } else { "" };
            format!("**{}{tail}", text.trim_matches('*'))
        } else {
            text
        };
        return Ok(field(TextMode::Wildcard, p));
    }
    Ok(field(TextMode::Contains, text))
}

/// Split `a..b` (both sides non-empty).
fn range(s: &str) -> Option<(&str, &str)> {
    let (a, b) = s.split_once("..")?;
    (!a.is_empty() && !b.is_empty()).then_some((a, b))
}

fn size(w: &Word, it: &Item) -> Result<Rule, QueryError> {
    let sz = |s: &str| parse_size(s).map_err(|m| err(it.span, m));
    if let Some((a, b)) = range(&it.text) {
        if let Some((_, span)) = w.op {
            return Err(err(span, "a range like 1MB..10MB cannot be combined with an operator"));
        }
        let (lo, hi) = (sz(a)?, sz(b)?);
        if lo > hi {
            return Err(err(it.span, "the range starts above its end; write the smaller size first"));
        }
        return Ok(all_of(vec![
            cond(Condition::Size { cmp: Cmp::Ge, bytes: lo }),
            cond(Condition::Size { cmp: Cmp::Le, bytes: hi }),
        ]));
    }
    let bytes = sz(&it.text)?;
    let cmp = cmp_of(w.op.map(|o| o.0));
    if cmp == Cmp::Eq && bytes == 0 {
        return Ok(cond(Condition::ZeroByte));
    }
    Ok(cond(Condition::Size { cmp, bytes }))
}

fn number(w: &Word, it: &Item, depth: bool) -> Result<Rule, QueryError> {
    let n = |s: &str| parse_u32(s).map_err(|m| err(it.span, m));
    let make = |cmp: Cmp, value: u32| {
        cond(if depth { Condition::Depth { cmp, value } } else { Condition::PathLength { cmp, value } })
    };
    if let Some((a, b)) = range(&it.text) {
        if let Some((_, span)) = w.op {
            return Err(err(span, "a range like 2..5 cannot be combined with an operator"));
        }
        let (lo, hi) = (n(a)?, n(b)?);
        if lo > hi {
            return Err(err(it.span, "the range starts above its end"));
        }
        return Ok(all_of(vec![make(Cmp::Ge, lo), make(Cmp::Le, hi)]));
    }
    Ok(make(cmp_of(w.op.map(|o| o.0)), n(&it.text)?))
}

fn at(t: Timestamp) -> DateValue {
    DateValue::At(t)
}

fn dcond(which: DateField, cmp: Cmp, value: DateValue) -> Rule {
    cond(Condition::Date { which, cmp, value })
}

/// One date bound for an operator (see the table in `query/mod.rs`).
fn date_rule(which: DateField, op: Option<Op>, spec: DateSpec) -> Rule {
    match spec {
        DateSpec::Abs(DateRange { start, end }) => match op {
            Some(Op::Lt) => dcond(which, Cmp::Lt, at(start)),
            Some(Op::Le) => dcond(which, Cmp::Lt, at(end)),
            Some(Op::Gt) => dcond(which, Cmp::Ge, at(end)),
            Some(Op::Ge) => dcond(which, Cmp::Ge, at(start)),
            Some(Op::Eq) | None => all_of(vec![dcond(which, Cmp::Ge, at(start)), dcond(which, Cmp::Lt, at(end))]),
        },
        // Relative values are ages: `>2y` = older than two years.
        DateSpec::Rel(span) => {
            let ago = DateValue::Ago(span);
            match op {
                Some(Op::Gt) => dcond(which, Cmp::Lt, ago),
                Some(Op::Ge) => dcond(which, Cmp::Le, ago),
                Some(Op::Lt) => dcond(which, Cmp::Gt, ago),
                Some(Op::Le) | Some(Op::Eq) | None => dcond(which, Cmp::Ge, ago),
            }
        }
    }
}

fn date(w: &Word, it: &Item, which: DateField) -> Result<Rule, QueryError> {
    let spec = |s: &str| parse_datespec(s).map_err(|m| err(it.span, m));
    if let Some((a, b)) = range(&it.text) {
        if let Some((_, span)) = w.op {
            return Err(err(span, "a range like 2023..2024 cannot be combined with an operator"));
        }
        return match (spec(a)?, spec(b)?) {
            (DateSpec::Abs(x), DateSpec::Abs(y)) => {
                Ok(all_of(vec![dcond(which, Cmp::Ge, at(x.start)), dcond(which, Cmp::Lt, at(y.end))]))
            }
            (DateSpec::Rel(x), DateSpec::Rel(y)) => {
                // Ages: the younger bound is the latest instant, the older the earliest.
                let (young, old) = if span_days(x) <= span_days(y) { (x, y) } else { (y, x) };
                Ok(all_of(vec![
                    dcond(which, Cmp::Ge, DateValue::Ago(old)),
                    dcond(which, Cmp::Le, DateValue::Ago(young)),
                ]))
            }
            _ => Err(err(it.span, "a date range must use two dates or two spans, not one of each")),
        };
    }
    Ok(date_rule(which, w.op.map(|o| o.0), spec(&it.text)?))
}

fn age_key(w: &Word, it: &Item, older: bool) -> Result<Rule, QueryError> {
    no_op(w)?;
    let spec = parse_datespec(&it.text).map_err(|m| err(it.span, m))?;
    let op = if older { Op::Gt } else { Op::Lt };
    // `older:2024` = before 2024; `newer:2024` = from 2025 on; spans are ages.
    Ok(match (spec, older) {
        (DateSpec::Abs(_), true) => date_rule(DateField::Modified, Some(Op::Lt), spec),
        (DateSpec::Abs(_), false) => date_rule(DateField::Modified, Some(Op::Gt), spec),
        (DateSpec::Rel(_), _) => date_rule(DateField::Modified, Some(op), spec),
    })
}

fn type_group(w: &Word, it: &Item, env: &Env<'_>) -> Result<Rule, QueryError> {
    no_op(w)?;
    let name = it.text.trim().to_lowercase();
    if let Some(g) = env.groups {
        return match groups::find(g, &name) {
            Some((id, _)) => Ok(cond(Condition::TypeGroup { group: id.to_string() })),
            None => {
                let known: Vec<&str> = g.keys().map(String::as_str).collect();
                Err(err(it.span, format!("unknown file-type group '{}'; known groups: {}", it.text, known.join(", "))))
            }
        };
    }
    Ok(cond(Condition::TypeGroup { group: name }))
}

/// Attribute names and their spellings.
fn attribute_named(s: &str) -> Option<Attributes> {
    let n = s.trim().to_lowercase().replace(['-', ' '], "_");
    let n = match n.as_str() {
        "read_only" | "ro" => "readonly",
        "onlineonly" | "cloud" => "online_only",
        "reparse" | "link" => "reparse_point",
        other => other,
    };
    Attributes::from_name(n)
}

fn attr_error(it: &Item) -> QueryError {
    let names: Vec<&str> = Attributes::ALL.iter().map(|(_, n)| *n).collect();
    err(it.span, format!("unknown attribute '{}'; attributes: {}", it.text, names.join(", ")))
}

fn attr(w: &Word, it: &Item) -> Result<Rule, QueryError> {
    no_op(w)?;
    let (set, name) = match it.text.strip_prefix('!') {
        Some(rest) => (false, rest),
        None => (true, it.text.as_str()),
    };
    let a = attribute_named(name).ok_or_else(|| attr_error(it))?;
    Ok(cond(Condition::Attribute { attr: a, set }))
}

fn owner(w: &Word, it: &Item) -> Result<Rule, QueryError> {
    no_op(w)?;
    non_empty(it, "the owner")?;
    Ok(cond(Condition::Owner { value: it.text.trim().to_string() }))
}

fn kind_named(s: &str) -> Option<EntryKind> {
    match s.trim().to_lowercase().as_str() {
        "file" | "files" => Some(EntryKind::File),
        "dir" | "folder" | "directory" | "dirs" | "folders" => Some(EntryKind::Dir),
        "link" | "symlink" | "links" => Some(EntryKind::Link),
        "other" => Some(EntryKind::Other),
        _ => None,
    }
}

fn kind(w: &Word, it: &Item) -> Result<Rule, QueryError> {
    no_op(w)?;
    kind_named(&it.text)
        .map(|k| cond(Condition::Kind { kind: k }))
        .ok_or_else(|| err(it.span, format!("unknown kind '{}'; use file, dir, link or other", it.text)))
}

fn content(w: &Word, it: &Item) -> Result<Rule, QueryError> {
    no_op(w)?;
    non_empty(it, "the search text")?;
    Ok(cond(Condition::ContentContains { text: it.text.clone(), case_sensitive: false }))
}

fn is(w: &Word, it: &Item) -> Result<Rule, QueryError> {
    no_op(w)?;
    let name = it.text.trim().to_lowercase();
    Ok(match name.as_str() {
        "empty" | "zero" | "zerobyte" | "zero_byte" => cond(Condition::ZeroByte),
        "invalid" | "invalid_name" | "invalidname" => cond(Condition::InvalidName),
        "mismatch" | "mismatched" | "disguised" => cond(Condition::ExtensionMismatch),
        other => {
            if let Some(k) = kind_named(other) {
                cond(Condition::Kind { kind: k })
            } else if let Some(a) = attribute_named(other) {
                cond(Condition::Attribute { attr: a, set: true })
            } else {
                return Err(err(
                    it.span,
                    format!(
                        "unknown state '{}'; use empty, hidden, invalid, mismatch, file, dir or an attribute name",
                        it.text
                    ),
                ));
            }
        }
    })
}

fn top(w: &Word) -> Result<(Rule, Span), QueryError> {
    no_op(w)?;
    if w.items.len() != 1 {
        return Err(err(w.span, "top: takes a single number, e.g. top:50"));
    }
    let it = &w.items[0];
    let n = parse_u32(&it.text).map_err(|m| err(it.span, m))?;
    Ok((cond(Condition::LargestN { n }), w.span))
}
