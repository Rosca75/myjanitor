//! Session results: the index and the opportunities built from it. Owner: W3-D.
//!
//! Selection rule (safety first): a row is acted on only when it is selected **and**
//! actionable. Protected rows (`Badges::BLOCKING`) are never actionable; signed,
//! labelled and encrypted rows are locked while the matching `Settings::safety::skip_*`
//! is on, because the action engine would skip them anyway (D-3D108).

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
