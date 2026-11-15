//! A small table-driven engine shared by the cache, Office and font-cache detectors: each rule is a
//! path glob (matched against the paths in the scan index, so "resolving against the scanned
//! roots" needs no environment lookups) plus the application it belongs to.
//!
//! Matching is pre-filtered by the rule's literal last component (or its parent's) through hash
//! maps, so even a million-entry index runs the full glob only on the rare name collisions.
//! Owner: W2-D.

use std::collections::{BTreeMap, HashMap};

use mj_core::glob::{Glob, GlobMode};
use mj_core::{
    Candidate, DetectContext, DetectorId, DetectorOutput, EntryKind, FileEntry, FileIndex, Result, TargetKind,
};

use crate::common::{self, Ticker};

/// What a rule may match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Dir,
    File,
    /// A file or a folder (children of a temp folder).
    Any,
}

/// One rule of a table.
#[derive(Debug, Clone)]
pub struct RuleDef {
    /// Application name; empty = derive it from the matched entry's name (generic `~/.cache/*`).
    pub app: &'static str,
    /// Kind of junk ("Browser cache", "Crash dumps").
    pub what: &'static str,
    /// Why it is safe, as a short clause ("recreated automatically").
    pub note: &'static str,
    /// Path glob, `/`-separated, case-insensitive.
    pub pattern: String,
    pub target: Target,
    /// Pre-selected in the review list.
    pub select: bool,
    /// Skip entries modified within the minimum age (temp folders: programs may be using them).
    pub aged: bool,
    /// Skip runtime sockets/session folders of `/tmp` (`.X11-unix`, `systemd-private-*`, ...).
    pub runtime_tmp: bool,
}

/// Rule constructor with the common defaults (`select`, not aged, not runtime tmp).
pub fn rule(
    app: &'static str,
    what: &'static str,
    note: &'static str,
    pattern: impl Into<String>,
    target: Target,
) -> RuleDef {
    RuleDef { app, what, note, pattern: pattern.into(), target, select: true, aged: false, runtime_tmp: false }
}

impl RuleDef {
    /// Not pre-selected.
    pub fn review(mut self) -> Self {
        self.select = false;
        self
    }

    /// Skip recently modified entries.
    pub fn aged(mut self) -> Self {
        self.aged = true;
        self
    }

    /// Skip runtime sockets and session folders.
    pub fn runtime_tmp(mut self) -> Self {
        self.runtime_tmp = true;
        self
    }
}

struct Compiled {
    def: RuleDef,
    glob: Glob,
}

/// A compiled rule table.
pub struct RuleSet {
    rules: Vec<Compiled>,
    by_leaf: HashMap<String, Vec<usize>>,
    by_parent: HashMap<String, Vec<usize>>,
    wild: Vec<usize>,
}

fn is_literal(s: &str) -> bool {
    !s.contains(['*', '?', '['])
}

impl RuleSet {
    /// Compile `defs`; table order is the priority order (first match wins).
    pub fn new(defs: Vec<RuleDef>) -> RuleSet {
        let mut set =
            RuleSet { rules: Vec::new(), by_leaf: HashMap::new(), by_parent: HashMap::new(), wild: Vec::new() };
        for def in defs {
            let i = set.rules.len();
            let segs: Vec<&str> = def.pattern.split('/').collect();
            let leaf = segs.last().copied().unwrap_or("");
            let parent = if segs.len() >= 2 { segs[segs.len() - 2] } else { "" };
            if is_literal(leaf) {
                set.by_leaf.entry(leaf.to_lowercase()).or_default().push(i);
            } else if is_literal(parent) && !parent.is_empty() {
                set.by_parent.entry(parent.to_lowercase()).or_default().push(i);
            } else {
                set.wild.push(i);
            }
            let glob = Glob::new(&def.pattern, GlobMode::Path, true);
            set.rules.push(Compiled { def, glob });
        }
        set
    }

    /// The first rule (table order) matching `e`.
    pub fn find(&self, e: &FileEntry) -> Option<&RuleDef> {
        let kind_ok = |t: Target| {
            matches!(
                (t, e.kind),
                (Target::Dir, EntryKind::Dir)
                    | (Target::File, EntryKind::File)
                    | (Target::Any, EntryKind::Dir | EntryKind::File)
            )
        };
        if !matches!(e.kind, EntryKind::Dir | EntryKind::File) {
            return None;
        }
        let name = common::name_lower(e);
        let parent = e.path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_lowercase());
        let mut cand: Vec<usize> = Vec::new();
        if let Some(v) = self.by_leaf.get(&name) {
            cand.extend(v);
        }
        if let Some(v) = parent.as_ref().and_then(|p| self.by_parent.get(p)) {
            cand.extend(v);
        }
        cand.extend(&self.wild);
        cand.sort_unstable();
        let text = e.path.to_string_lossy();
        cand.into_iter()
            .map(|i| &self.rules[i])
            .find(|c| kind_ok(c.def.target) && c.glob.matches(&text))
            .map(|c| &c.def)
    }
}

/// One matched entry, ready to become a candidate (built-in rules and `winapp2.ini` rules alike).
#[derive(Debug, Clone)]
pub struct Hit {
    pub pos: usize,
    pub app: String,
    pub what: String,
    pub note: String,
    pub select: bool,
    /// Derived from a catch-all rule (no known application): yields to specific hits inside it.
    pub generic: bool,
}

/// Names `/tmp` keeps for running sessions; removing them breaks the desktop.
fn is_runtime_tmp(name_lower: &str) -> bool {
    [".x", ".ice", ".font", ".xim", "systemd-private-", "snap-private-tmp", "ssh-", "tmux-", "pulse-", "dbus-", "snap."]
        .iter()
        .any(|p| name_lower.starts_with(p))
}

/// Scan the index with `set`, add `extra` hits (winapp2), keep the topmost, and build candidates.
///
/// * `force_review`: no candidate is pre-selected (review-only detectors).
/// * `min_age_hours`: applied to `aged` rules.
pub fn run_rules(
    ctx: &DetectContext<'_>,
    set: &RuleSet,
    id: DetectorId,
    extra: Vec<Hit>,
    force_review: bool,
    min_age_hours: f64,
) -> Result<DetectorOutput> {
    let ix = ctx.index;
    let cutoff = common::hours_ago(ctx.now, min_age_hours);
    let mut out = DetectorOutput::default();
    let mut tick = Ticker::new(ctx, ix.len() as u64);
    let mut hits: BTreeMap<usize, Hit> = BTreeMap::new();
    for (pos, e) in ix.entries().iter().enumerate() {
        tick.tick()?;
        if !common::usable(e) {
            continue;
        }
        let Some(r) = set.find(e) else { continue };
        if r.runtime_tmp && is_runtime_tmp(&common::name_lower(e)) {
            out.diagnostics.skip("runtime_session_file");
            continue;
        }
        if r.aged && !common::before(e.modified, cutoff) {
            out.diagnostics.skip("recently_modified");
            continue;
        }
        let app = if r.app.is_empty() { e.name() } else { r.app.to_string() };
        hits.insert(
            pos,
            Hit { pos, app, what: r.what.into(), note: r.note.into(), select: r.select, generic: r.app.is_empty() },
        );
    }
    for h in extra {
        hits.entry(h.pos).or_insert(h);
    }
    // A catch-all hit (`~/.cache/<unknown>`) that contains a specific one gives way to it, so a
    // known application is reported under its own name instead of vanishing into its parent.
    let mut covers: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for h in hits.values().filter(|h| !h.generic) {
        let mut cur = ix.parent(h.pos);
        while let Some(a) = cur {
            if !covers.insert(a) {
                break;
            }
            cur = ix.parent(a);
        }
    }
    let positions: Vec<usize> = hits.keys().copied().filter(|p| !(hits[p].generic && covers.contains(p))).collect();
    for pos in common::topmost(ix, &positions) {
        let h = &hits[&pos];
        let Some(c) = build(ix, h, id, force_review, &mut out) else { continue };
        tick.found(c.saving());
        out.candidates.push(c);
    }
    tick.finish();
    Ok(out)
}

fn build(ix: &FileIndex, h: &Hit, id: DetectorId, force_review: bool, out: &mut DetectorOutput) -> Option<Candidate> {
    let e = ix.entry(h.pos);
    let select = h.select && !force_review;
    let note = if h.note.is_empty() { String::new() } else { format!(" ({})", h.note) };
    let mut c = if e.kind == EntryKind::Dir {
        let t = ix.totals(h.pos);
        if t.files == 0 {
            out.diagnostics.skip("empty_folder"); // the empty-folders detector owns these
            return None;
        }
        let mut c = common::folder_candidate(ix, h.pos, id, format!("{}: {}{note}", h.app, h.what), select);
        c.target = TargetKind::Folder;
        c
    } else {
        common::file_candidate(e, id, format!("{}: {}{note}", h.app, h.what), select)
    };
    c.badges |= mj_core::Badges::REBUILDABLE;
    c.sub_group = Some(common::slug(&h.app));
    c.columns.insert("app".into(), h.app.clone());
    c.columns.insert("kind".into(), h.what.clone());
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::Timestamp;

    fn ix(paths: &[(&str, EntryKind)]) -> FileIndex {
        let entries = paths
            .iter()
            .map(|(p, k)| {
                let mut e = FileEntry::new(*p, *k, if *k == EntryKind::File { 10 } else { 0 });
                e.depth = 1;
                e
            })
            .collect();
        FileIndex::new(vec!["/".into()], entries, Default::default(), Timestamp(0))
    }

    #[test]
    fn literal_leaf_parent_and_wild_rules_match() {
        let set = RuleSet::new(vec![
            rule("A", "cache", "", "**/AppData/Local/Foo/Cache", Target::Dir),
            rule("B", "children", "", "**/Temp/*", Target::Any),
            rule("C", "db", "", "**/Explorer/thumbcache_*.db", Target::File),
            rule("D", "abs", "", "/tmp/*", Target::Any),
        ]);
        let t = ix(&[
            ("C:/Users/x/AppData/Local/Foo/Cache", EntryKind::Dir),
            ("C:/Users/x/AppData/Local/Foo/cache2", EntryKind::Dir),
            ("C:/Users/x/AppData/Local/Temp/junk.bin", EntryKind::File),
            ("C:/Users/x/AppData/Local/Temp/sub/deep.bin", EntryKind::File),
            ("C:/Users/x/Explorer/ThumbCache_256.db", EntryKind::File),
            ("C:/Users/x/Explorer/other.db", EntryKind::File),
            ("/tmp/a", EntryKind::File),
            ("/home/tmp/a", EntryKind::File),
        ]);
        let hit: Vec<_> = t
            .entries()
            .iter()
            .filter_map(|e| set.find(e).map(|r| (e.path.to_string_lossy().into_owned(), r.app)))
            .collect();
        let names: Vec<&str> = hit.iter().map(|(p, a)| if p.ends_with("Foo/Cache") { "A" } else { a }).collect();
        assert_eq!(hit.len(), 4, "{hit:?}");
        assert!(names.contains(&"A") && names.contains(&"B") && names.contains(&"C") && names.contains(&"D"));
        // Only direct children of Temp, not the nested file; not a lookalike folder.
        assert!(!hit.iter().any(|(p, _)| p.ends_with("deep.bin") || p.ends_with("cache2") || p.contains("/home/")));
    }

    #[test]
    fn dir_rules_do_not_match_files_and_vice_versa() {
        let set = RuleSet::new(vec![rule("A", "x", "", "**/Cache", Target::Dir)]);
        let t = ix(&[("/r/Cache", EntryKind::File), ("/r/d/Cache", EntryKind::Dir), ("/r/l/Cache", EntryKind::Link)]);
        let n = t.entries().iter().filter(|e| set.find(e).is_some()).count();
        assert_eq!(n, 1);
    }
}
