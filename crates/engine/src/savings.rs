//! One account of what applying frees (W6-B).
//!
//! Every number the overview and the plan show is a sum over the same per-row decision,
//! made here and nowhere else: the hero total and its buckets, each card, each plan step,
//! the plan total, the per-drive bars and what a dry run measures.
//!
//! A selected row counts `size - size after its action`, where the action decides the
//! "after" ([`frees`]): a removal frees the whole row whatever the detector estimated for
//! another action, an optimisation frees its estimate, and an action that leaves the
//! original in place (review only, copy, light copy, archive keeping its sources) frees
//! nothing. A row is **not** counted when:
//!
//! - it is not selected, or locked (protected, or signed / labelled / encrypted while the
//!   matching safety setting is on);
//! - it is online-only: its bytes are not on this disk;
//! - every copy of its group is selected under an action that removes copies: apply
//!   keeps one copy and skips the group (F-02);
//! - another participant keeps it as the surviving copy of a group, or it is a folder
//!   holding such a copy, and this action would take it away: apply skips it (G-03);
//! - the same file is counted by another participant that frees more for it, or it lies
//!   inside a folder a participant frees more by acting on as a whole.
//!
//! The last two rules compare participants, so the result depends on who takes part:
//! every available card for the overview, the steps of the plan for the plan. With
//! `cross == false` each row stands alone (a card on its own, for the step review).
//!
//! "Frees more" is resolved exactly: rows are nodes of the folder tree, a node is worth
//! `max(its best saving, the sum of what is counted below it)`, ties go to the outer
//! node and, on one path, to the earlier participant.

use std::collections::{BTreeMap, HashMap, HashSet};

use mj_core::pathutil::path_key;
use mj_core::{Action, Badges, Candidate, Confidence, Estimate, GroupId, OriginalsPolicy, OutputMode, SafetyClass};

use crate::results::{
    blocked_reason, candidate_safety, group_survivors, is_selected, removes_copy, severity, OpportunityRecord,
};

/// What an action frees of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Frees {
    /// The row leaves this place: its whole size.
    Whole,
    /// The row stays, smaller: the candidate's own estimate.
    Estimated,
    /// The original stays as it is.
    Nothing,
}

/// What `action` frees of the rows it runs on.
pub(crate) fn frees(action: &Action) -> Frees {
    match action {
        Action::Trash
        | Action::Quarantine { .. }
        | Action::PermanentDelete
        | Action::Move { .. }
        | Action::Hardlink
        | Action::Reflink
        | Action::Dehydrate => Frees::Whole,
        Action::Copy { .. } | Action::FsDecompress | Action::None => Frees::Nothing,
        Action::Optimise { output: OutputMode::LightCopy { .. }, .. } => Frees::Nothing,
        Action::Archive { keep_sources: true, .. } => Frees::Nothing,
        Action::Optimise { .. } | Action::Archive { .. } | Action::FsCompress { .. } => Frees::Estimated,
    }
}

/// True when `action`, run on a kept copy (or a folder holding one), could leave no copy of
/// its group where the user can reach it: removals, moves, archives that take their
/// sources away, and optimisations or archives that delete the originals (G-03).
pub(crate) fn takes_copy_away(action: &Action) -> bool {
    match action {
        Action::Trash | Action::Quarantine { .. } | Action::PermanentDelete | Action::Move { .. } => true,
        Action::Archive { keep_sources, originals, .. } => !keep_sources || *originals == OriginalsPolicy::Delete,
        Action::Optimise { originals, .. } => *originals == OriginalsPolicy::Delete,
        _ => false,
    }
}

/// One record with the action it would run.
#[derive(Clone, Copy)]
pub(crate) struct Part<'a> {
    pub rec: &'a OpportunityRecord,
    pub action: &'a Action,
}

/// What became of one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowState {
    /// Not selected and not selectable (kept copy, locked, online-only).
    Idle,
    /// Not selected, could be: listed for review.
    Review,
    /// Selected by default but locked by a safety setting: skipped.
    Locked,
    /// Selected; apply skips it (every copy of its group selected, or no copy to link to).
    Lost,
    /// Selected and acted on; online-only, so nothing is freed here.
    Online,
    /// Selected, acted on, counted here.
    Counted,
    /// Selected; apply skips it because another participant keeps it as a last copy.
    Kept,
    /// Selected and acted on, but its saving is counted by another row.
    Shared,
}

/// Totals of one participant.
#[derive(Debug, Clone)]
pub(crate) struct Tally {
    /// Rows selected and actionable (the ticked rows).
    pub selected: u64,
    /// Rows acted on and counted here (online-only rows included).
    pub files: u64,
    /// Rows apply will skip (`Locked`, `Lost`, `Kept`).
    pub skipped: u64,
    /// Current size of the counted rows.
    pub bytes_before: u64,
    /// Their size after the action.
    pub bytes_after: u64,
    pub confidence: Confidence,
    /// Saving of `Kept` and `Shared` rows: what the participant would free on its own on
    /// top of [`Tally::saving`].
    pub overlap: u64,
    /// `(rows, bytes)` of the `Shared` rows.
    pub shared: (u64, u64),
    /// Number of `Kept` rows.
    pub kept: u64,
    /// Current size of the rows listed for review (see `Opportunity::to_review`).
    pub to_review: u64,
    pub to_review_rows: u64,
    /// Counted saving by the class of each row.
    pub by_class: BTreeMap<u8, (SafetyClass, u64)>,
    /// State and counted saving of every row, in candidate order.
    pub rows: Vec<(RowState, u64)>,
}

impl Tally {
    /// Counted saving.
    pub fn saving(&self) -> u64 {
        self.bytes_before.saturating_sub(self.bytes_after)
    }

    /// The estimate of a plan step.
    pub fn estimate(&self) -> Estimate {
        Estimate {
            files: self.files,
            bytes_before: self.bytes_before,
            bytes_after: self.bytes_after,
            confidence: if self.files == 0 { Confidence::Exact } else { self.confidence },
            skipped: self.skipped,
        }
    }
}

/// Size of `c` once an action that [`frees`] `f` ran.
fn size_after(c: &Candidate, f: Frees) -> u64 {
    match f {
        Frees::Whole => 0,
        Frees::Estimated => c.size_after.min(c.size),
        Frees::Nothing => c.size,
    }
}

/// A counted row in the folder tree.
struct Ent {
    /// `path_key` with `/` replaced by NUL, so a folder sorts right before what it holds.
    key: String,
    part: u32,
    row: u32,
    saving: u64,
}

/// One distinct path of the entries.
struct Node {
    /// First entry of this path in the sorted entries, and one past the last.
    first: usize,
    end: usize,
    /// Entry with the largest saving (the earliest participant on a tie).
    best: usize,
    parent: Option<usize>,
    below: u64,
    take_self: bool,
    alive: bool,
}

fn tree_key(path: &std::path::Path) -> String {
    path_key(path).replace('/', "\0")
}

/// True when `outer` is a proper ancestor of `inner` (both tree keys).
fn holds(outer: &str, inner: &str) -> bool {
    inner.len() > outer.len()
        && inner.starts_with(outer)
        && (outer.ends_with('\0') || inner.as_bytes()[outer.len()] == 0)
}

/// Groups by id; the first of a repeated id wins, as a linear search would.
fn by_first_id(groups: &[mj_core::CandidateGroup]) -> HashMap<GroupId, &mj_core::CandidateGroup> {
    let mut m = HashMap::with_capacity(groups.len());
    for g in groups {
        m.entry(g.id).or_insert(g);
    }
    m
}

/// The account of `parts`: one [`Tally`] per participant, in order.
pub(crate) fn tally(parts: &[Part<'_>], skip: Badges, cross: bool) -> Vec<Tally> {
    let mut out: Vec<Tally> = Vec::with_capacity(parts.len());
    // Groups of each participant with at least one row acted on, and the copy each keeps.
    let mut keeps: Vec<Vec<std::path::PathBuf>> = Vec::with_capacity(parts.len());
    for p in parts {
        let rec = p.rec;
        let f = frees(p.action);
        let links = matches!(p.action, Action::Hardlink | Action::Reflink);
        // Looked up only by the links, once per card (not once per selected row).
        let by_id: HashMap<GroupId, &mj_core::CandidateGroup> =
            if links { by_first_id(&rec.groups) } else { HashMap::new() };
        let survivors = if removes_copy(p.action) { group_survivors(rec, skip) } else { HashMap::new() };
        let mut acted: HashSet<GroupId> = HashSet::new();
        let mut kept_here: Vec<std::path::PathBuf> = Vec::new();
        let mut t = Tally {
            selected: 0,
            files: 0,
            skipped: 0,
            bytes_before: 0,
            bytes_after: 0,
            confidence: Confidence::Exact,
            overlap: 0,
            shared: (0, 0),
            kept: 0,
            to_review: 0,
            to_review_rows: 0,
            by_class: BTreeMap::new(),
            rows: Vec::with_capacity(rec.candidates.len()),
        };
        for c in &rec.candidates {
            let state = if !is_selected(rec, c, skip) {
                if blocked_reason(c, skip).is_some() {
                    if c.default_selected {
                        RowState::Locked
                    } else {
                        RowState::Idle
                    }
                } else if c.badges.intersects(Badges::KEEPER) || c.badges.intersects(Badges::ONLINE_ONLY) {
                    RowState::Idle
                } else {
                    RowState::Review
                }
            } else {
                t.selected += 1;
                let group = c.group.and_then(|g| by_id.get(&g).copied());
                let lost = c.group.is_some_and(|g| matches!(survivors.get(&g), Some(None)))
                    || (links && !group.and_then(|g| g.keeper.as_ref()).is_some_and(|k| *k != c.path));
                if lost {
                    RowState::Lost
                } else if c.badges.contains(Badges::ONLINE_ONLY) {
                    RowState::Online
                } else {
                    if let Some(g) = c.group {
                        if acted.insert(g) {
                            let keep = if links {
                                group.and_then(|x| x.keeper.clone())
                            } else {
                                survivors.get(&g).cloned().flatten()
                            };
                            kept_here.extend(keep);
                        }
                    }
                    RowState::Counted
                }
            };
            let saving = if state == RowState::Counted { c.size - size_after(c, f) } else { 0 };
            t.rows.push((state, saving));
        }
        keeps.push(kept_here);
        out.push(t);
    }

    if cross {
        // G-03: the key of every kept copy and of each folder above it.
        let mut held: HashSet<String> = HashSet::new();
        for k in keeps.iter().flatten() {
            let mut key = tree_key(k);
            while !key.is_empty() && held.insert(key.clone()) {
                match key.rfind('\0') {
                    Some(i) => key.truncate(i),
                    None => break,
                }
            }
        }
        let mut ents: Vec<Ent> = Vec::new();
        for (pi, (p, t)) in parts.iter().zip(out.iter_mut()).enumerate() {
            let away = !held.is_empty() && takes_copy_away(p.action);
            for (ri, (c, row)) in p.rec.candidates.iter().zip(t.rows.iter_mut()).enumerate() {
                if row.0 != RowState::Counted || (row.1 == 0 && !away) {
                    continue;
                }
                let key = tree_key(&c.path);
                if away && held.contains(&key) {
                    row.0 = RowState::Kept;
                } else if row.1 > 0 {
                    ents.push(Ent { key, part: pi as u32, row: ri as u32, saving: row.1 });
                }
            }
        }
        ents.sort_by(|a, b| a.key.cmp(&b.key).then(a.part.cmp(&b.part)).then(a.row.cmp(&b.row)));
        // Nodes in tree order; `open` is the chain of nodes holding the current one.
        let mut nodes: Vec<Node> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        let mut i = 0;
        while i < ents.len() {
            let mut end = i + 1;
            let mut best = i;
            while end < ents.len() && ents[end].key == ents[i].key {
                if ents[end].saving > ents[best].saving {
                    best = end;
                }
                end += 1;
            }
            while open.last().is_some_and(|&n| !holds(&ents[nodes[n].first].key, &ents[i].key)) {
                open.pop();
            }
            nodes.push(Node {
                first: i,
                end,
                best,
                parent: open.last().copied(),
                below: 0,
                take_self: true,
                alive: true,
            });
            open.push(nodes.len() - 1);
            i = end;
        }
        // Bottom-up: a node is worth the larger of itself and what is counted below it.
        for n in (0..nodes.len()).rev() {
            let own = ents[nodes[n].best].saving;
            nodes[n].take_self = own >= nodes[n].below;
            let worth = own.max(nodes[n].below);
            if let Some(p) = nodes[n].parent {
                nodes[p].below += worth;
            }
        }
        // Top-down: below a node counted as a whole, nothing else counts.
        for n in 0..nodes.len() {
            let alive = nodes[n].parent.is_none_or(|p| nodes[p].alive && !nodes[p].take_self);
            nodes[n].alive = alive;
            for (e, ent) in ents.iter().enumerate().take(nodes[n].end).skip(nodes[n].first) {
                if !(alive && nodes[n].take_self && e == nodes[n].best) {
                    out[ent.part as usize].rows[ent.row as usize].0 = RowState::Shared;
                }
            }
        }
    }

    for (p, t) in parts.iter().zip(out.iter_mut()) {
        let f = frees(p.action);
        let review_all = matches!(p.action, Action::None);
        for (c, (state, saving)) in p.rec.candidates.iter().zip(&t.rows) {
            let conf = if f == Frees::Estimated { c.confidence } else { Confidence::Exact };
            match state {
                RowState::Idle => {}
                RowState::Review => {
                    t.to_review += c.size;
                    t.to_review_rows += 1;
                }
                RowState::Locked | RowState::Lost => t.skipped += 1,
                RowState::Online => {
                    t.files += 1;
                    t.confidence = t.confidence.max(conf);
                }
                RowState::Counted => {
                    t.files += 1;
                    t.bytes_before += c.size;
                    t.bytes_after += c.size - saving;
                    t.confidence = t.confidence.max(conf);
                    if *saving > 0 {
                        let class = candidate_safety(p.rec, c);
                        t.by_class.entry(severity(class)).or_insert((class, 0)).1 += saving;
                    }
                    if review_all {
                        t.to_review += c.size;
                        t.to_review_rows += 1;
                    }
                }
                RowState::Kept => {
                    t.skipped += 1;
                    t.kept += 1;
                    t.overlap += saving;
                }
                RowState::Shared => {
                    t.overlap += saving;
                    t.shared.0 += 1;
                    t.shared.1 += saving;
                }
            }
        }
        // A row that is not counted here counts nothing.
        for row in &mut t.rows {
            if row.0 != RowState::Counted {
                row.1 = 0;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::{
        CandidateGroup, CandidateId, FileStamp, GroupKind, Opportunity, OpportunityId, ParamValues, SelectionOverrides,
        Source, TargetKind, Timestamp,
    };

    fn cand(id: u64, path: &str, size: u64, size_after: u64, selected: bool) -> Candidate {
        Candidate {
            id: CandidateId(id),
            path: path.into(),
            target: TargetKind::File,
            source: Source::Custom("t".into()),
            size,
            size_after,
            confidence: Confidence::Estimated,
            reason: String::new(),
            badges: Badges::empty(),
            default_selected: selected,
            group: None,
            sub_group: None,
            modified: None,
            stamp: FileStamp { size, mtime: Timestamp(0) },
            file_count: None,
            columns: Default::default(),
        }
    }

    fn folder(id: u64, path: &str, size: u64, size_after: u64) -> Candidate {
        Candidate { target: TargetKind::Folder, ..cand(id, path, size, size_after, true) }
    }

    fn rec(
        id: &str,
        safety: SafetyClass,
        candidates: Vec<Candidate>,
        groups: Vec<CandidateGroup>,
    ) -> OpportunityRecord {
        OpportunityRecord {
            opportunity: Opportunity {
                id: OpportunityId(id.into()),
                source: Source::Custom(id.into()),
                category: mj_core::Category::Clean,
                title: id.into(),
                description: String::new(),
                safety,
                size_before: 0,
                saving: 0,
                confidence: Confidence::Exact,
                count: 0,
                unit: "files".into(),
                candidates: candidates.len() as u64,
                selected: 0,
                overlap: 0,
                to_review: 0,
                to_review_count: 0,
                badges: Badges::empty(),
                default_action: Action::Trash,
                available_actions: Vec::new(),
                presets: Vec::new(),
                settings: Vec::new(),
                params: ParamValues::default(),
                sub_groups: Vec::new(),
                availability: mj_core::Availability::Available,
                in_plan: false,
                rank: 0.0,
                feature_ids: Vec::new(),
            },
            candidates,
            groups,
            diagnostics: Default::default(),
            selection: SelectionOverrides::default(),
        }
    }

    fn optimise() -> Action {
        Action::Optimise {
            optimiser: mj_core::OptimiserId::PngLossless,
            params: ParamValues::default(),
            output: OutputMode::Replace,
            originals: OriginalsPolicy::RecycleBin,
            keep_if_not_smaller: true,
        }
    }

    fn savings(t: &[Tally]) -> Vec<u64> {
        t.iter().map(Tally::saving).collect()
    }

    #[test]
    fn the_action_decides_what_a_row_frees() {
        let r = rec("a", SafetyClass::Reversible, vec![cand(1, "/r/a.png", 1000, 900, true)], vec![]);
        let one = |a: &Action| tally(&[Part { rec: &r, action: a }], Badges::empty(), true).remove(0);
        assert_eq!(one(&Action::Trash).saving(), 1000, "a removal frees the whole file");
        assert_eq!(one(&Action::Trash).confidence, Confidence::Exact);
        assert_eq!(one(&optimise()).saving(), 100);
        assert_eq!(one(&optimise()).confidence, Confidence::Estimated);
        let mut light = optimise();
        if let Action::Optimise { output, .. } = &mut light {
            *output = OutputMode::LightCopy { suffix: "_light".into() };
        }
        for nothing in [
            Action::None,
            Action::Copy { dest: "/elsewhere".into(), keep_structure: false },
            Action::FsDecompress,
            light,
        ] {
            let t = one(&nothing);
            assert_eq!((t.saving(), t.files, t.bytes_before), (0, 1, 1000), "{nothing:?}");
        }
        // Review only: the ticked row is still "to review".
        assert_eq!((one(&Action::None).to_review, one(&Action::None).to_review_rows), (1000, 1));
        assert_eq!(one(&Action::Trash).to_review, 0);
        // A candidate that claims to grow never counts negative.
        let grow = rec("g", SafetyClass::Reversible, vec![cand(1, "/r/a", 10, 50, true)], vec![]);
        assert_eq!(tally(&[Part { rec: &grow, action: &optimise() }], Badges::empty(), true)[0].saving(), 0);
    }

    #[test]
    fn rows_that_are_not_acted_on_count_nothing() {
        let mut online = cand(2, "/r/cloud.bin", 500, 0, true);
        online.badges |= Badges::ONLINE_ONLY;
        let mut signed = cand(3, "/r/signed.pdf", 300, 0, false);
        signed.badges |= Badges::SIGNED;
        let mut protected = cand(4, "/r/sys.dll", 200, 0, false);
        protected.badges |= Badges::PROTECTED;
        let unticked = cand(5, "/r/maybe.tmp", 70, 0, false);
        let mut r = rec(
            "a",
            SafetyClass::Reversible,
            vec![cand(1, "/r/a.tmp", 100, 0, true), online, signed, protected, unticked],
            vec![],
        );
        r.selection.include.insert(CandidateId(3)); // ticked while "skip signed" was off
        r.selection.include.insert(CandidateId(4)); // protected: never actionable
        let t = tally(&[Part { rec: &r, action: &Action::Trash }], Badges::SIGNED, true).remove(0);
        assert_eq!((t.saving(), t.selected, t.files), (100, 2, 2), "online-only is acted on but frees nothing");
        assert_eq!((t.to_review, t.to_review_rows), (70, 1), "only the row that could be ticked");
        let states: Vec<RowState> = t.rows.iter().map(|r| r.0).collect();
        assert_eq!(states, [RowState::Counted, RowState::Online, RowState::Idle, RowState::Idle, RowState::Review]);
        // With "skip signed" off the signed row counts.
        let t = tally(&[Part { rec: &r, action: &Action::Trash }], Badges::empty(), true).remove(0);
        assert_eq!(t.saving(), 400);
    }

    fn pair(keeper_selected: bool) -> OpportunityRecord {
        let mut keep = cand(1, "/r/keep/x.bin", 100, 0, false);
        keep.badges |= Badges::KEEPER;
        keep.group = Some(GroupId(7));
        let mut copy = cand(2, "/r/copy/x.bin", 100, 0, true);
        copy.group = Some(GroupId(7));
        let mut r = rec(
            "dup",
            SafetyClass::Reversible,
            vec![keep, copy],
            vec![CandidateGroup {
                id: GroupId(7),
                kind: GroupKind::Exact,
                label: String::new(),
                confidence: 100.0,
                keeper: Some("/r/keep/x.bin".into()),
                members: vec!["/r/keep/x.bin".into(), "/r/copy/x.bin".into()],
            }],
        );
        if keeper_selected {
            r.selection.include.insert(CandidateId(1));
        }
        r
    }

    #[test]
    fn a_group_with_every_copy_selected_is_skipped_only_by_actions_that_remove_copies() {
        let all = pair(true);
        let t = tally(&[Part { rec: &all, action: &Action::Trash }], Badges::empty(), true).remove(0);
        assert_eq!((t.saving(), t.selected, t.files, t.skipped), (0, 2, 0, 2));
        // Moving every copy away is allowed (the copies still exist at the destination).
        let mv = Action::Move { dest: "/elsewhere".into(), keep_structure: false };
        let t = tally(&[Part { rec: &all, action: &mv }], Badges::empty(), true).remove(0);
        assert_eq!((t.saving(), t.files, t.skipped), (200, 2, 0));
        // Linking needs a kept copy that is not the row itself.
        let t = tally(&[Part { rec: &all, action: &Action::Hardlink }], Badges::empty(), true).remove(0);
        assert_eq!((t.saving(), t.files, t.skipped), (100, 1, 1));
        // The kept copy is never "to review".
        let one = pair(false);
        let t = tally(&[Part { rec: &one, action: &Action::Trash }], Badges::empty(), true).remove(0);
        assert_eq!((t.saving(), t.to_review_rows), (100, 0));
    }

    #[test]
    fn one_file_in_two_opportunities_counts_for_the_one_that_frees_more() {
        let png = rec("png", SafetyClass::Reversible, vec![cand(1, "/r/a.png", 1000, 900, true)], vec![]);
        let dup = rec("dup", SafetyClass::Reversible, vec![cand(2, "/r/a.png", 1024, 0, true)], vec![]);
        let o = optimise();
        let t =
            tally(&[Part { rec: &png, action: &o }, Part { rec: &dup, action: &Action::Trash }], Badges::empty(), true);
        assert_eq!(savings(&t), [0, 1024]);
        assert_eq!((t[0].overlap, t[0].shared, t[0].files, t[0].selected), (100, (1, 100), 0, 1));
        assert_eq!(t[0].rows[0], (RowState::Shared, 0));
        // On its own, each card shows its full saving.
        assert_eq!(savings(&tally(&[Part { rec: &png, action: &o }], Badges::empty(), true)), [100]);
        let alone = tally(
            &[Part { rec: &png, action: &o }, Part { rec: &dup, action: &Action::Trash }],
            Badges::empty(),
            false,
        );
        assert_eq!(savings(&alone), [100, 1024]);
        // A tie goes to the earlier participant; the spelling of the path does not matter.
        let a = rec("a", SafetyClass::Reversible, vec![cand(1, "/r/x/../a.bin", 50, 0, true)], vec![]);
        let b = rec("b", SafetyClass::Reversible, vec![cand(2, "/r/a.bin", 50, 0, true)], vec![]);
        let t = tally(
            &[Part { rec: &a, action: &Action::Trash }, Part { rec: &b, action: &Action::Trash }],
            Badges::empty(),
            true,
        );
        assert_eq!(savings(&t), [50, 0]);
    }

    #[test]
    fn a_folder_and_the_files_inside_it_are_counted_once() {
        // Removing node_modules frees more than compressing two files in it.
        let dev = rec("dev", SafetyClass::Rebuildable, vec![folder(1, "/r/p/node_modules", 10_000, 0)], vec![]);
        let inside = rec(
            "fsc",
            SafetyClass::Reversible,
            vec![
                cand(2, "/r/p/node_modules/a.js", 4000, 1000, true),
                cand(3, "/r/p/node_modules/lib/b.js", 2000, 500, true),
                cand(4, "/r/p/node_modules.txt", 800, 300, true), // a sibling, not inside
                cand(5, "/r/p/src/c.rs", 600, 200, true),
            ],
            vec![],
        );
        let fsc = Action::FsCompress { algorithm: mj_core::FsCompressAlgo::Xpress8k };
        let parts = [Part { rec: &inside, action: &fsc }, Part { rec: &dev, action: &Action::Trash }];
        let t = tally(&parts, Badges::empty(), true);
        assert_eq!(savings(&t), [500 + 400, 10_000]);
        assert_eq!(t[0].overlap, 3000 + 1500);
        assert_eq!(t[0].by_class[&severity(SafetyClass::Reversible)].1, 900);
        assert_eq!(t[1].by_class[&severity(SafetyClass::Rebuildable)].1, 10_000);
        // An archive whose saving is unknown (0) never hides what is counted inside it.
        let arch = rec("arch", SafetyClass::Reversible, vec![folder(1, "/r/p/node_modules", 10_000, 10_000)], vec![]);
        let a = Action::Archive {
            format: mj_core::ArchiveFormat::Zip,
            level: 9,
            dest: None,
            originals: OriginalsPolicy::RecycleBin,
            keep_sources: false,
        };
        let t = tally(&[Part { rec: &inside, action: &fsc }, Part { rec: &arch, action: &a }], Badges::empty(), true);
        assert_eq!(savings(&t), [3000 + 1500 + 500 + 400, 0]);
        // Nested folders: the outer one wins when it frees at least as much as everything below.
        let outer = rec("outer", SafetyClass::Reversible, vec![folder(1, "/r/p", 20_000, 0)], vec![]);
        let t = tally(
            &[
                Part { rec: &inside, action: &fsc },
                Part { rec: &dev, action: &Action::Trash },
                Part { rec: &outer, action: &Action::Trash },
            ],
            Badges::empty(),
            true,
        );
        assert_eq!(savings(&t), [0, 0, 20_000]);
        assert_eq!(t.iter().map(|x| x.overlap).sum::<u64>(), 3000 + 1500 + 500 + 400 + 10_000);
    }

    #[test]
    fn the_copy_another_opportunity_keeps_is_not_counted_as_removed() {
        let dup = pair(false);
        let old = rec("old", SafetyClass::Reversible, vec![cand(9, "/r/keep/x.bin", 100, 0, true)], vec![]);
        let holding = rec("inactive", SafetyClass::Reversible, vec![folder(8, "/r/keep", 5000, 0)], vec![]);
        let parts = [
            Part { rec: &dup, action: &Action::Trash },
            Part { rec: &old, action: &Action::Trash },
            Part { rec: &holding, action: &Action::Trash },
        ];
        let t = tally(&parts, Badges::empty(), true);
        assert_eq!(savings(&t), [100, 0, 0], "one copy of x.bin always stays");
        assert_eq!((t[1].kept, t[1].skipped, t[1].overlap, t[1].files), (1, 1, 100, 0));
        assert_eq!((t[2].kept, t[2].overlap), (1, 5000), "a folder holding the kept copy is skipped too");
        // An action that leaves the file in place may run on the kept copy.
        let o = optimise();
        let t =
            tally(&[Part { rec: &dup, action: &Action::Trash }, Part { rec: &old, action: &o }], Badges::empty(), true);
        assert_eq!(savings(&t), [100, 100]);
        // Nothing is kept for a group nobody acts on.
        let mut idle = pair(false);
        idle.selection.exclude.insert(CandidateId(2));
        let t = tally(
            &[Part { rec: &idle, action: &Action::Trash }, Part { rec: &old, action: &Action::Trash }],
            Badges::empty(),
            true,
        );
        assert_eq!(savings(&t), [0, 100]);
    }

    #[test]
    fn tree_keys_keep_a_folder_next_to_its_content() {
        assert!(holds(&tree_key("/a/b".as_ref()), &tree_key("/a/b/c".as_ref())));
        assert!(!holds(&tree_key("/a/b".as_ref()), &tree_key("/a/b.txt".as_ref())));
        assert!(!holds(&tree_key("/a/b".as_ref()), &tree_key("/a/b".as_ref())));
        assert!(!holds(&tree_key("/a/b".as_ref()), &tree_key("/a/bc/d".as_ref())));
        let mut keys = [tree_key("/a/b.txt".as_ref()), tree_key("/a/b/c".as_ref()), tree_key("/a/b".as_ref())];
        keys.sort();
        assert_eq!(keys, [tree_key("/a/b".as_ref()), tree_key("/a/b/c".as_ref()), tree_key("/a/b.txt".as_ref())]);
    }

    #[test]
    #[ignore = "timing; run with --ignored --nocapture"]
    fn tally_is_linear_in_the_number_of_groups() {
        let n = 150_000u64;
        let cands: Vec<Candidate> = (0..n)
            .map(|i| Candidate { group: Some(GroupId(i / 2)), ..cand(i, &format!("/r/{i:07}.bin"), 1000, 1000, true) })
            .collect();
        let groups: Vec<CandidateGroup> = (0..n / 2)
            .map(|g| CandidateGroup {
                id: GroupId(g),
                kind: GroupKind::Exact,
                label: String::new(),
                confidence: 100.0,
                keeper: Some(format!("/r/{:07}.bin", g * 2).into()),
                members: vec![format!("/r/{:07}.bin", g * 2).into(), format!("/r/{:07}.bin", g * 2 + 1).into()],
            })
            .collect();
        let r = rec("d", SafetyClass::Reversible, cands, groups);
        for a in [Action::Trash, Action::Hardlink] {
            let t = std::time::Instant::now();
            let out = tally(&[Part { rec: &r, action: &a }], Badges::empty(), true);
            let took = t.elapsed();
            println!("{a:?}: {took:?}");
            assert!(out[0].selected > 0);
            assert!(took < std::time::Duration::from_secs(2), "tally took {took:?}: quadratic group lookup?");
        }
    }
}
