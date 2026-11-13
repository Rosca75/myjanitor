//! Session results: the index and the opportunities built from it. Owner: W3-D.

use std::sync::Arc;

use mj_core::{Candidate, CandidateGroup, Diagnostics, FileIndex, Opportunity, OpportunityId, SelectionOverrides};

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
}

/// `Error::NotFound`-style error for an unknown opportunity id.
pub fn unknown(id: &OpportunityId) -> mj_core::Error {
    mj_core::Error::InvalidInput(format!("unknown opportunity `{}` (scan again)", id.0))
}
