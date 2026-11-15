//! Session results: the index and the opportunities built from it. Owner: W3-D.
//!
//! Selection rule (safety first): a row is acted on only when it is selected **and**
//! actionable. Protected rows (`Badges::BLOCKING`) are never actionable; signed,
//! labelled and encrypted rows are locked while the matching `Settings::safety::skip_*`
//! is on, because the action engine would skip them anyway (D-3D108).

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use mj_core::{
    Badges, Candidate, CandidateGroup, Diagnostics, FileIndex, Opportunity, OpportunityId, SafetyClass,
    SelectionOverrides, Settings,
};

/// One opportunity with everything the step review needs.
#[derive(Debug, Clone)]
pub struct OpportunityRecord {
    pub opportunity: Opportunity,
    pub candidates: Vec<Candidate>,
    pub groups: Vec<CandidateGroup>,
    pub diagnostics: Diagnostics,
    pub selection: SelectionOverrides,
}

/// Results of the latest completed scan.
#[derive(Debug, Default)]
pub struct Results {
    /// Scan generation these results belong to (0 = none yet).
    pub generation: u64,
    pub index: Option<Arc<FileIndex>>,
    pub opportunities: Vec<OpportunityRecord>,
    pub cached: bool,
    /// Counter for `custom:<n>` ids.
    pub next_custom: u64,
    /// ENG-11: path keys of items applied since `index` was built that are no longer at
    /// their place (removed, moved, replaced under another name). The index is immutable:
    /// a re-analysis or a search on it leaves these out. Cleared when a scan publishes.
    pub removed: HashSet<String>,
    /// ENG-12: the rule of every custom-search card (`custom:<id>`, the request that built
    /// it). A scan runs each again on its new index, so the card and its plan step survive.
    pub custom_rules: Vec<(String, crate::api::SearchRunRequest)>,
}

impl Results {
    /// The record of an opportunity.
    pub fn get(&self, id: &OpportunityId) -> Option<&OpportunityRecord> {
        self.opportunities.iter().find(|o| &o.opportunity.id == id)
    }

    /// Mutable record of an opportunity.
    pub fn get_mut(&mut self, id: &OpportunityId) -> Option<&mut OpportunityRecord> {
        self.opportunities.iter_mut().find(|o| &o.opportunity.id == id)
    }

    /// The record holding a candidate.
    pub fn find_candidate(&self, id: mj_core::CandidateId) -> Option<(&OpportunityRecord, &Candidate)> {
        self.opportunities.iter().find_map(|r| r.candidates.iter().find(|c| c.id == id).map(|c| (r, c)))
    }
}

/// True when `p` or a folder above it is in `removed` (ENG-11).
pub(crate) fn is_removed(removed: &HashSet<String>, p: &Path) -> bool {
    !removed.is_empty() && p.ancestors().any(|a| removed.contains(&mj_core::pathutil::path_key(a)))
}

/// ENG-11: take the rows at or below a path that is `gone` out of a card. Groups that lose
/// members drop them, and a group left with fewer than two members leaves the card with
/// its last row (nothing is a duplicate or a pair of nothing); the kept badges follow the
/// selection again and the totals are recomputed. Returns true when anything changed.
pub(crate) fn drop_gone(rec: &mut OpportunityRecord, gone: &dyn Fn(&Path) -> bool, skip: Badges) -> bool {
    let before = rec.candidates.len();
    rec.candidates.retain(|c| !gone(&c.path));
    let mut changed = rec.candidates.len() != before;
    let mut lone = Vec::new();
    for g in &mut rec.groups {
        let n = g.members.len();
        g.members.retain(|m| !gone(m));
        if g.members.len() != n {
            changed = true;
            if g.members.len() < 2 {
                lone.push(g.id);
            }
        }
    }
    if !lone.is_empty() {
        rec.groups.retain(|g| !lone.contains(&g.id));
        rec.candidates.retain(|c| !c.group.is_some_and(|g| lone.contains(&g)));
    }
    if changed {
        crate::steps::sync_keepers(rec, skip);
        crate::opportunities::recompute(rec, skip);
    }
    changed
}

/// `invalid_input` for an unknown opportunity id.
pub fn unknown(id: &OpportunityId) -> mj_core::Error {
    mj_core::Error::InvalidInput(format!("unknown opportunity `{}` (scan again)", id.0))
}

/// Badges the safety settings make the action engine skip.
pub fn skip_badges(settings: &Settings) -> Badges {
    mj_actions::safety::skip_badges(settings)
}

/// Why a row cannot be selected, if it cannot.
pub fn blocked_reason(c: &Candidate, skip: Badges) -> Option<String> {
    if c.badges.intersects(Badges::BLOCKING) {
        return Some("Protected path: never changed".into());
    }
    let hit = c.badges & skip;
    if hit.contains(Badges::SIGNED) {
        return Some("Digitally signed: skipped (Settings › Safety)".into());
    }
    if hit.contains(Badges::LABELLED) {
        return Some("Sensitivity label: skipped (Settings › Safety)".into());
    }
    if hit.contains(Badges::ENCRYPTED) {
        return Some("Encrypted: skipped (Settings › Safety)".into());
    }
    None
}

/// Effective selection: selected by default or by the user, and actionable.
pub fn is_selected(rec: &OpportunityRecord, c: &Candidate, skip: Badges) -> bool {
    blocked_reason(c, skip).is_none() && rec.selection.is_selected(c.id, c.default_selected)
}

/// G-04: after a rescan, a row the previous results already had keeps the selection the
/// user saw (matched by path): a row they unchecked stays unchecked, and a row that was
/// checked stays checked only while its scan stamp is unchanged. Only rows new to this scan
/// take the detector's default. Without this, a rescan silently put back the defaults of
/// steps the user had curated.
pub(crate) fn carry_selections(old: &[OpportunityRecord], new: &mut [OpportunityRecord]) {
    use mj_core::pathutil::path_key;
    for rec in new.iter_mut() {
        let Some(prev) = old.iter().find(|o| o.opportunity.id == rec.opportunity.id) else { continue };
        let seen: std::collections::HashMap<String, (bool, mj_core::FileStamp)> = prev
            .candidates
            .iter()
            .map(|c| (path_key(&c.path), (prev.selection.is_selected(c.id, c.default_selected), c.stamp)))
            .collect();
        let mut sel = mj_core::SelectionOverrides::default();
        for c in &rec.candidates {
            let Some((was, stamp)) = seen.get(&path_key(&c.path)) else { continue };
            let now = *was && *stamp == c.stamp;
            sel.set(c.id, c.default_selected, now);
        }
        rec.selection = sel;
    }
}

/// Safety class of one candidate (its override, else the card's).
pub fn candidate_safety(rec: &OpportunityRecord, c: &Candidate) -> SafetyClass {
    c.safety_override().unwrap_or(rec.opportunity.safety)
}

/// Severity order used to combine classes (the most demanding wins).
pub fn severity(s: SafetyClass) -> u8 {
    match s {
        SafetyClass::ReviewOnly => 0,
        SafetyClass::LightCopy => 1,
        SafetyClass::Reversible => 2,
        SafetyClass::Rebuildable => 3,
        SafetyClass::LossyOptIn => 4,
        SafetyClass::Permanent => 5,
    }
}

/// True for actions after which a removed group member's content is gone from the
/// scanned tree (F-02: every group must keep one member).
pub(crate) fn removes_copy(a: &mj_core::Action) -> bool {
    matches!(a, mj_core::Action::Trash | mj_core::Action::Quarantine { .. } | mj_core::Action::PermanentDelete)
}

/// F-02: for every group of `rec`, the member that survives the removal of the selected
/// rows: the group's keeper when it is not selected, else the first member that is not
/// selected. `None` when every member is selected (nothing would survive).
pub(crate) fn group_survivors(
    rec: &OpportunityRecord,
    skip: Badges,
) -> std::collections::HashMap<mj_core::GroupId, Option<std::path::PathBuf>> {
    use std::collections::{HashMap, HashSet};
    let selected: HashSet<&std::path::Path> =
        rec.candidates.iter().filter(|c| is_selected(rec, c, skip)).map(|c| c.path.as_path()).collect();
    let mut out = HashMap::new();
    for g in &rec.groups {
        let left = |p: &std::path::PathBuf| !selected.contains(p.as_path());
        let survivor = g.keeper.as_ref().filter(|k| left(k)).or_else(|| g.members.iter().find(|m| left(m))).cloned();
        out.insert(g.id, survivor);
    }
    out
}

/// Number of groups of `rec` whose every member is selected.
pub(crate) fn groups_losing_every_copy(rec: &OpportunityRecord, skip: Badges) -> u64 {
    group_survivors(rec, skip).values().filter(|s| s.is_none()).count() as u64
}
