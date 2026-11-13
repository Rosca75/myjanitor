//! Rule compilation and evaluation. Owner: W2-C.
//!
//! Semantics (frozen): `all([])` matches everything, `any([])` nothing; a
//! missing timestamp never satisfies a date condition; `TextMode::Wildcard`
//! uses `mj_core::glob` (name mode for `Name`, path mode for `Path`); regexes
//! are compiled once (`regex` crate) and invalid ones are `Error::InvalidInput`;
//! `LargestN` is applied after the rest of the tree; `topmost_only` keeps a
//! matching folder and drops matches below it (CLN-ENG-04).
//!
//! Additions made while implementing (all documented in `docs/decisions/W2-C.md`):
//!
//! * Evaluation is three-valued (true / false / unknown). A condition that
//!   could not be evaluated (unreadable header, document with no extractor,
//!   content search cut off by the size cap) is *unknown*, and `NOT unknown` is
//!   unknown, so "files that do not contain X" never lists files that were not
//!   actually searched. Only a definite `true` selects a file.
//! * Children of `all` / `any` are evaluated cheapest first (metadata before
//!   text matching before header reads before full-content reads), so content
//!   is only read for files that survived every cheap test.
//! * Extension, type-group, mismatch, content and zero-byte conditions apply to
//!   regular files only.
//! * Content conditions never touch online-only (cloud placeholder) files:
//!   reading one would download it.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use mj_core::pathutil::{extension_lower, has_invalid_windows_name};
use mj_core::{
    Attributes, CancelToken, Cmp, Condition, DateField, DateValue, EntryKind, Error, FileEntry, FileIndex, Result,
    Rule, TextMode, Timestamp,
};

use crate::content::{self, ContentConfig, ContentOutcome, MismatchOutcome, Needle};
use crate::exclusions::Exclusions;
use crate::groups;
use crate::matcher::TextMatcher;
use crate::timeutil;

/// Rule trees nested deeper than this are refused (hostile preset files).
pub const MAX_RULE_DEPTH: usize = 64;
/// Rule trees with more nodes than this are refused.
pub const MAX_RULE_NODES: usize = 20_000;

/// Evaluation context.
pub struct FilterContext<'a> {
    /// "Now" for relative dates.
    pub now: Timestamp,
    /// File-type groups (`Settings::type_groups`).
    pub type_groups: &'a BTreeMap<String, Vec<String>>,
}

/// Three-valued truth: `Unknown` means "could not be evaluated".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tri {
    True,
    False,
    Unknown,
}

impl Tri {
    fn from(b: bool) -> Tri {
        if b {
            Tri::True
        } else {
            Tri::False
        }
    }

    fn not(self) -> Tri {
        match self {
            Tri::True => Tri::False,
            Tri::False => Tri::True,
            Tri::Unknown => Tri::Unknown,
        }
    }
}

/// Counters shared by every evaluation of one compiled rule.
#[derive(Debug, Default)]
struct Counters {
    content_reads: AtomicU64,
    header_reads: AtomicU64,
    content_unreadable: AtomicU64,
    header_unreadable: AtomicU64,
    content_binary: AtomicU64,
    content_truncated: AtomicU64,
    skipped_online_only: AtomicU64,
    unavailable: Mutex<BTreeMap<String, u64>>,
    unreadable_sample: Mutex<Vec<PathBuf>>,
}

impl Counters {
    fn bump(c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }

    fn unreadable(&self, e: &FileEntry) {
        let mut s = mj_core::lock(&self.unreadable_sample);
        if s.len() < 10 {
            s.push(e.path.clone());
        }
    }
}

/// What the evaluation could *not* cover, plus I/O counters. Never silently empty
/// when something was skipped (ARCHITECTURE §5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterDiagnostics {
    /// Entries skipped by the global exclusions.
    pub excluded: u64,
    /// Files whose content was read for a content condition.
    pub content_reads: u64,
    /// Headers read for an extension-mismatch condition.
    pub header_reads: u64,
    /// Content searches that failed to read the file or run the extractor.
    pub content_unreadable: u64,
    /// Header reads that failed.
    pub header_unreadable: u64,
    /// Binary files skipped by a content search.
    pub content_binary: u64,
    /// Content searches cut off by the byte cap without finding the text.
    pub content_truncated: u64,
    /// Online-only files not read (reading would download them).
    pub skipped_online_only: u64,
    /// Document files not searched because no extractor is registered, by extension.
    pub content_unavailable: BTreeMap<String, u64>,
    /// Up to ten unreadable paths.
    pub unreadable_sample: Vec<PathBuf>,
}

impl FilterDiagnostics {
    /// Convert to the shared `Diagnostics` shape (skipped reasons and notes).
    pub fn to_core(&self) -> mj_core::Diagnostics {
        let mut d = mj_core::Diagnostics {
            unreadable: self.content_unreadable + self.header_unreadable,
            unreadable_sample: self.unreadable_sample.clone(),
            ..Default::default()
        };
        let mut add = |reason: &str, n: u64| {
            if n > 0 {
                d.skipped.insert(reason.to_string(), n);
            }
        };
        add("excluded", self.excluded);
        add("content_binary", self.content_binary);
        add("content_truncated", self.content_truncated);
        add("online_only_not_read", self.skipped_online_only);
        for (ext, n) in &self.content_unavailable {
            add(&format!("content_unavailable_{ext}"), *n);
        }
        if !self.content_unavailable.is_empty() {
            let exts: Vec<&str> = self.content_unavailable.keys().map(String::as_str).collect();
            d.notes.push(format!(
                "Text search is not available for {} files: no text extractor is installed for them.",
                exts.join(", ")
            ));
        }
        d
    }
}

/// A leaf condition after validation.
#[derive(Debug)]
enum Leaf {
    Name(TextMatcher),
    Path(TextMatcher),
    Ext(HashSet<String>),
    Size(Cmp, u64),
    ZeroByte,
    Date { which: DateField, cmp: Cmp, at: Timestamp },
    Attr { attr: Attributes, set: bool },
    Owner(String),
    Depth(Cmp, u32),
    PathLen(Cmp, u32),
    InvalidName,
    Kind(EntryKind),
    Mismatch,
    Content(Needle),
}

#[derive(Debug)]
enum Node {
    All(Vec<Node>),
    Any(Vec<Node>),
    Not(Box<Node>),
    Leaf(Leaf),
}

impl Node {
    /// Relative cost used to order children (cheap first).
    fn cost(&self) -> u32 {
        match self {
            Node::All(c) | Node::Any(c) => c.iter().map(Node::cost).max().unwrap_or(0),
            Node::Not(c) => c.cost(),
            Node::Leaf(l) => match l {
                Leaf::Size(..) | Leaf::ZeroByte | Leaf::Kind(_) | Leaf::Depth(..) | Leaf::Attr { .. } => 1,
                Leaf::Date { .. } | Leaf::Owner(_) => 2,
                Leaf::Ext(_) => 3,
                Leaf::PathLen(..) | Leaf::InvalidName => 4,
                Leaf::Name(m) | Leaf::Path(m) => 10 + m.cost() as u32,
                Leaf::Mismatch => 100,
                Leaf::Content(_) => 200,
            },
        }
    }

    fn sort_children(children: &mut [Node]) {
        children.sort_by_key(Node::cost);
    }

    fn eval(&self, e: &FileEntry, rt: &Runtime<'_>) -> Tri {
        match self {
            Node::All(c) => {
                let mut unknown = false;
                for n in c {
                    match n.eval(e, rt) {
                        Tri::False => return Tri::False,
                        Tri::Unknown => unknown = true,
                        Tri::True => {}
                    }
                }
                if unknown {
                    Tri::Unknown
                } else {
                    Tri::True
                }
            }
            Node::Any(c) => {
                let mut unknown = false;
                for n in c {
                    match n.eval(e, rt) {
                        Tri::True => return Tri::True,
                        Tri::Unknown => unknown = true,
                        Tri::False => {}
                    }
                }
                if unknown {
                    Tri::Unknown
                } else {
                    Tri::False
                }
            }
            Node::Not(c) => c.eval(e, rt).not(),
            Node::Leaf(l) => l.eval(e, rt),
        }
    }
}

/// Borrowed evaluation environment.
struct Runtime<'a> {
    counters: &'a Counters,
    content: &'a ContentConfig,
}

fn file_name(e: &FileEntry) -> std::borrow::Cow<'_, str> {
    e.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
}

impl Leaf {
    fn eval(&self, e: &FileEntry, rt: &Runtime<'_>) -> Tri {
        let is_file = e.kind == EntryKind::File;
        match self {
            Leaf::Name(m) => Tri::from(m.is_match(&file_name(e))),
            Leaf::Path(m) => Tri::from(m.is_match(&e.path.to_string_lossy())),
            Leaf::Ext(set) => Tri::from(is_file && set.contains(&extension_lower(&e.path))),
            Leaf::Size(cmp, bytes) => Tri::from(cmp.test(e.size, *bytes)),
            Leaf::ZeroByte => Tri::from(is_file && e.size == 0),
            Leaf::Date { which, cmp, at } => {
                let ts = match which {
                    DateField::Created => e.created,
                    DateField::Modified => e.modified,
                    DateField::Accessed => e.accessed,
                };
                Tri::from(ts.is_some_and(|t| cmp.test(t.0, at.0)))
            }
            Leaf::Attr { attr, set } => {
                Tri::from(if *set { e.attrs.contains(*attr) } else { !e.attrs.intersects(*attr) })
            }
            Leaf::Owner(o) => Tri::from(e.owner.as_ref().is_some_and(|x| x.to_lowercase() == *o)),
            Leaf::Depth(cmp, v) => Tri::from(cmp.test(e.depth, *v)),
            Leaf::PathLen(cmp, v) => {
                let n = e.path.to_string_lossy().encode_utf16().count();
                Tri::from(cmp.test(n as u64, *v as u64))
            }
            Leaf::InvalidName => Tri::from(has_invalid_windows_name(&file_name(e))),
            Leaf::Kind(k) => Tri::from(e.kind == *k),
            Leaf::Mismatch => self.eval_mismatch(e, rt),
            Leaf::Content(n) => self.eval_content(e, rt, n),
        }
    }

    fn eval_mismatch(&self, e: &FileEntry, rt: &Runtime<'_>) -> Tri {
        let ext = extension_lower(&e.path);
        if e.kind != EntryKind::File || e.size == 0 || mj_formats::sniff::expected_kinds(&ext).is_empty() {
            return Tri::False;
        }
        if e.is_online_only() {
            Counters::bump(&rt.counters.skipped_online_only);
            return Tri::Unknown;
        }
        Counters::bump(&rt.counters.header_reads);
        match content::extension_mismatch(&e.path, &ext) {
            MismatchOutcome::Mismatch => Tri::True,
            MismatchOutcome::Fine => Tri::False,
            MismatchOutcome::Unreadable => {
                Counters::bump(&rt.counters.header_unreadable);
                rt.counters.unreadable(e);
                Tri::Unknown
            }
        }
    }

    fn eval_content(&self, e: &FileEntry, rt: &Runtime<'_>, needle: &Needle) -> Tri {
        if e.kind != EntryKind::File || e.size == 0 {
            return Tri::False;
        }
        if e.is_online_only() {
            Counters::bump(&rt.counters.skipped_online_only);
            return Tri::Unknown;
        }
        Counters::bump(&rt.counters.content_reads);
        let ext = extension_lower(&e.path);
        match content::search_file(&e.path, &ext, needle, rt.content) {
            ContentOutcome::Found => Tri::True,
            ContentOutcome::NotFound { truncated: false } => Tri::False,
            ContentOutcome::NotFound { truncated: true } => {
                Counters::bump(&rt.counters.content_truncated);
                Tri::Unknown
            }
            ContentOutcome::Binary => {
                Counters::bump(&rt.counters.content_binary);
                Tri::False
            }
            ContentOutcome::Unavailable => {
                *mj_core::lock(&rt.counters.unavailable).entry(ext).or_default() += 1;
                Tri::Unknown
            }
            ContentOutcome::Unreadable => {
                Counters::bump(&rt.counters.content_unreadable);
                rt.counters.unreadable(e);
                Tri::Unknown
            }
        }
    }
}

/// A rule ready for fast evaluation.
#[derive(Debug)]
pub struct CompiledRule {
    rule: Rule,
    root: Node,
    largest: Option<u32>,
    content: ContentConfig,
    counters: Counters,
    expensive: bool,
}

impl CompiledRule {
    /// The source rule.
    pub fn rule(&self) -> &Rule {
        &self.rule
    }

    /// The `LargestN` limit lifted out of the top-level `all`, if any.
    pub fn largest_n(&self) -> Option<u32> {
        self.largest
    }

    /// True when the rule reads file contents or headers (callers should poll cancel more often).
    pub fn reads_files(&self) -> bool {
        self.expensive
    }

    /// Per-entry test (ignores `LargestN`, which needs the whole result).
    /// Entries that could not be evaluated (unknown) do not match.
    pub fn matches(&self, entry: &FileEntry) -> bool {
        let rt = Runtime { counters: &self.counters, content: &self.content };
        self.root.eval(entry, &rt) == Tri::True
    }

    /// Snapshot of what has been skipped or read so far.
    pub fn diagnostics(&self) -> FilterDiagnostics {
        let c = &self.counters;
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        FilterDiagnostics {
            excluded: 0,
            content_reads: get(&c.content_reads),
            header_reads: get(&c.header_reads),
            content_unreadable: get(&c.content_unreadable),
            header_unreadable: get(&c.header_unreadable),
            content_binary: get(&c.content_binary),
            content_truncated: get(&c.content_truncated),
            skipped_online_only: get(&c.skipped_online_only),
            content_unavailable: mj_core::lock(&c.unavailable).clone(),
            unreadable_sample: mj_core::lock(&c.unreadable_sample).clone(),
        }
    }
}

struct Compiler<'a> {
    ctx: &'a FilterContext<'a>,
    nodes: usize,
}

fn invalid(what: &str, why: impl std::fmt::Display) -> Error {
    Error::InvalidInput(format!("{what}: {why}"))
}

impl Compiler<'_> {
    fn build(&mut self, rule: &Rule, depth: usize) -> Result<Node> {
        self.nodes += 1;
        if depth > MAX_RULE_DEPTH {
            return Err(Error::InvalidInput(format!("the rule is nested more than {MAX_RULE_DEPTH} levels deep")));
        }
        if self.nodes > MAX_RULE_NODES {
            return Err(Error::InvalidInput(format!("the rule has more than {MAX_RULE_NODES} conditions")));
        }
        Ok(match rule {
            Rule::All(c) => self.build_list(c.iter().collect(), true, depth)?,
            Rule::Any(c) => self.build_list(c.iter().collect(), false, depth)?,
            Rule::Not(c) => Node::Not(Box::new(self.build(c, depth + 1)?)),
            Rule::Cond(c) => Node::Leaf(self.leaf(c)?),
        })
    }

    fn build_list(&mut self, rules: Vec<&Rule>, all: bool, depth: usize) -> Result<Node> {
        let mut nodes = rules.into_iter().map(|r| self.build(r, depth + 1)).collect::<Result<Vec<_>>>()?;
        Node::sort_children(&mut nodes);
        Ok(if all { Node::All(nodes) } else { Node::Any(nodes) })
    }

    fn text(&self, field: &str, mode: TextMode, pattern: &str, cs: bool, path: bool) -> Result<TextMatcher> {
        TextMatcher::new(mode, pattern, cs, path).map_err(|e| invalid(&format!("{field} condition '{pattern}'"), e))
    }

    fn leaf(&self, c: &Condition) -> Result<Leaf> {
        Ok(match c {
            Condition::Name { mode, pattern, case_sensitive } => {
                Leaf::Name(self.text("name", *mode, pattern, *case_sensitive, false)?)
            }
            Condition::Path { mode, pattern, case_sensitive } => {
                Leaf::Path(self.text("path", *mode, pattern, *case_sensitive, true)?)
            }
            Condition::Extension { values } => {
                if values.is_empty() {
                    return Err(Error::InvalidInput("extension condition has no extensions".into()));
                }
                Leaf::Ext(values.iter().map(|v| groups::normalise_ext(v)).collect())
            }
            Condition::TypeGroup { group } => match groups::find(self.ctx.type_groups, group) {
                Some((_, exts)) => Leaf::Ext(exts.iter().map(|v| groups::normalise_ext(v)).collect()),
                None => {
                    let known: Vec<&str> = self.ctx.type_groups.keys().map(String::as_str).collect();
                    return Err(invalid(
                        &format!("unknown file-type group '{group}'"),
                        format!("known groups: {}", known.join(", ")),
                    ));
                }
            },
            Condition::Size { cmp, bytes } => Leaf::Size(*cmp, *bytes),
            Condition::ZeroByte => Leaf::ZeroByte,
            Condition::LargestN { .. } => {
                return Err(Error::InvalidInput(
                    "'largest N' is only valid as a direct child of the top-level 'all' (it limits the whole result)"
                        .into(),
                ))
            }
            Condition::Date { which, cmp, value } => {
                let at = match value {
                    DateValue::At(t) => *t,
                    DateValue::Ago(span) => {
                        timeutil::span_before(*span, self.ctx.now).map_err(|e| invalid("date condition", e))?
                    }
                };
                Leaf::Date { which: *which, cmp: *cmp, at }
            }
            Condition::Attribute { attr, set } => {
                if attr.is_empty() {
                    return Err(Error::InvalidInput("attribute condition names no attribute".into()));
                }
                Leaf::Attr { attr: *attr, set: *set }
            }
            Condition::Owner { value } => {
                if value.trim().is_empty() {
                    return Err(Error::InvalidInput("owner condition is empty".into()));
                }
                Leaf::Owner(value.trim().to_lowercase())
            }
            Condition::Depth { cmp, value } => Leaf::Depth(*cmp, *value),
            Condition::PathLength { cmp, value } => Leaf::PathLen(*cmp, *value),
            Condition::InvalidName => Leaf::InvalidName,
            Condition::Kind { kind } => Leaf::Kind(*kind),
            Condition::ExtensionMismatch => Leaf::Mismatch,
            Condition::ContentContains { text, case_sensitive } => {
                if text.is_empty() {
                    return Err(Error::InvalidInput("content condition has no text".into()));
                }
                Leaf::Content(Needle::new(text, *case_sensitive))
            }
        })
    }
}

fn reads_files(n: &Node) -> bool {
    match n {
        Node::All(c) | Node::Any(c) => c.iter().any(reads_files),
        Node::Not(c) => reads_files(c),
        Node::Leaf(l) => matches!(l, Leaf::Mismatch | Leaf::Content(_)),
    }
}

/// Compile a rule (validates regexes, groups and `LargestN` placement).
pub fn compile(rule: &Rule, ctx: &FilterContext<'_>) -> Result<CompiledRule> {
    compile_with(rule, ctx, ContentConfig::default())
}

/// [`compile`] with content-search limits and text extractors.
pub fn compile_with(rule: &Rule, ctx: &FilterContext<'_>, content: ContentConfig) -> Result<CompiledRule> {
    let mut c = Compiler { ctx, nodes: 0 };
    // `LargestN` is lifted out of the top-level `all`; anywhere else it is an error.
    let (root, largest) = match rule {
        Rule::Cond(Condition::LargestN { n }) => (Node::All(Vec::new()), Some(*n)),
        Rule::All(children) => {
            let mut rest = Vec::new();
            let mut largest: Option<u32> = None;
            for ch in children {
                match ch {
                    Rule::Cond(Condition::LargestN { n }) => largest = Some(largest.map_or(*n, |m| m.min(*n))),
                    other => rest.push(other),
                }
            }
            (c.build_list(rest, true, 0)?, largest)
        }
        other => (c.build(other, 0)?, None),
    };
    let expensive = reads_files(&root);
    Ok(CompiledRule { rule: rule.clone(), root, largest, content, counters: Counters::default(), expensive })
}

/// Options for [`select_with`].
#[derive(Default)]
pub struct SelectOptions<'a> {
    /// Keep only the topmost matching folder (CLN-ENG-04).
    pub topmost_only: bool,
    /// Cancel / pause token, polled during evaluation.
    pub cancel: Option<&'a CancelToken>,
    /// Global exclusions applied before the rule (CLN-ENG-03).
    pub exclusions: Option<&'a Exclusions>,
    /// Content-search limits and document text extractors.
    pub content: ContentConfig,
}

/// Result of [`select_with`].
#[derive(Debug)]
pub struct Selection {
    /// Positions of matching entries, in index order.
    pub positions: Vec<usize>,
    /// What could not be covered.
    pub diagnostics: FilterDiagnostics,
}

/// Active "covered" ranges of the index (subtrees of excluded or kept folders).
/// Positions are queried in ascending order, so finished ranges are dropped.
#[derive(Default)]
struct Covers(Vec<Range<usize>>);

impl Covers {
    fn covers(&mut self, i: usize) -> bool {
        self.0.retain(|r| r.end > i);
        self.0.iter().any(|r| r.contains(&i))
    }

    fn add(&mut self, r: Range<usize>) {
        if !r.is_empty() {
            self.0.push(r);
        }
    }
}

/// Positions of the index entries matching `rule`, in index order.
pub fn select(index: &FileIndex, rule: &Rule, ctx: &FilterContext<'_>, topmost_only: bool) -> Result<Vec<usize>> {
    let opts = SelectOptions { topmost_only, ..SelectOptions::default() };
    Ok(select_with(index, rule, ctx, opts)?.positions)
}

/// Full-featured selection: exclusions, cancel support, content extractors and diagnostics.
pub fn select_with(
    index: &FileIndex,
    rule: &Rule,
    ctx: &FilterContext<'_>,
    opts: SelectOptions<'_>,
) -> Result<Selection> {
    let compiled = compile_with(rule, ctx, opts.content.clone())?;
    let poll_every = if compiled.reads_files() { 1 } else { 256 };
    let mut excluded_covers = Covers::default();
    let mut excluded = 0u64;
    let mut positions = Vec::new();
    for (i, e) in index.entries().iter().enumerate() {
        if i % poll_every == 0 {
            if let Some(c) = opts.cancel {
                c.checkpoint()?;
            }
        }
        if excluded_covers.covers(i) {
            excluded += 1;
            continue;
        }
        if let Some(ex) = opts.exclusions {
            if ex.is_excluded(e) {
                excluded += 1;
                if e.is_dir() {
                    excluded_covers.add(index.subtree(i));
                }
                continue;
            }
        }
        if compiled.matches(e) {
            positions.push(i);
        }
    }
    if let Some(n) = compiled.largest_n() {
        positions = keep_largest(index, positions, n as usize);
    }
    if opts.topmost_only {
        positions = topmost(index, &positions);
    }
    let mut diagnostics = compiled.diagnostics();
    diagnostics.excluded = excluded;
    Ok(Selection { positions, diagnostics })
}

/// Keep the `n` largest entries (ties broken by index order), returned in index order.
fn keep_largest(index: &FileIndex, mut positions: Vec<usize>, n: usize) -> Vec<usize> {
    if positions.len() > n {
        positions.sort_by(|&a, &b| index.entry(b).size.cmp(&index.entry(a).size).then(a.cmp(&b)));
        positions.truncate(n);
        positions.sort_unstable();
    }
    positions
}

/// Collapse matches to the highest matching folder: drop every match inside the
/// subtree of another matching folder (CLN-ENG-04). `positions` must be ascending.
pub fn topmost(index: &FileIndex, positions: &[usize]) -> Vec<usize> {
    let mut covers = Covers::default();
    let mut out = Vec::with_capacity(positions.len());
    for &p in positions {
        if covers.covers(p) {
            continue;
        }
        out.push(p);
        if index.entry(p).is_dir() {
            covers.add(index.subtree(p));
        }
    }
    out
}

/// Names of the groups a rule refers to (for UI hints and validation by callers).
pub fn referenced_groups(rule: &Rule) -> BTreeSet<String> {
    fn walk(r: &Rule, out: &mut BTreeSet<String>) {
        match r {
            Rule::All(c) | Rule::Any(c) => c.iter().for_each(|x| walk(x, out)),
            Rule::Not(c) => walk(c, out),
            Rule::Cond(Condition::TypeGroup { group }) => {
                out.insert(group.clone());
            }
            Rule::Cond(_) => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(rule, &mut out);
    out
}

#[cfg(test)]
#[path = "eval_tests.rs"]
mod tests;
