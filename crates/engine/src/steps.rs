//! Step review commands (B2: table, selection, parameters, preview). Owner: W3-D.
//! Wave 1: `step_detail` and selection are real over the in-memory records;
//! paging/sorting is real; previews and parameter re-estimation are not.

use mj_core::{Badges, Error, Result};

use crate::api::{
    CandidatePreviewRequest, CandidateRow, CandidateSortKey, CandidatesPage, CandidatesPageRequest, OpportunityRequest,
    Preview, SelectionKeepRuleRequest, SelectionSetRequest, StepDetail, StepParamsRequest, StepParamsResponse,
    StepSummary,
};
use crate::opportunities::recompute;
use crate::results::{unknown, OpportunityRecord};
use crate::Engine;

fn summary_of(rec: &OpportunityRecord) -> StepSummary {
    let o = &rec.opportunity;
    StepSummary {
        candidates: o.candidates,
        selected: o.selected,
        size_before: o.size_before,
        size_after: o.size_before.saturating_sub(o.saving),
        saving: o.saving,
        confidence: o.confidence,
    }
}

impl Engine {
    /// Header and right panel of a step.
    pub fn step_detail(&self, req: OpportunityRequest) -> Result<StepDetail> {
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let rec = results.get(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
        let lossy = rec.opportunity.safety == mj_core::SafetyClass::LossyOptIn;
        Ok(StepDetail {
            opportunity: rec.opportunity.clone(),
            summary: summary_of(rec),
            diagnostics: rec.diagnostics.clone(),
            step_id: self.in_plan(&req.opportunity_id),
            lossy,
            safety: rec.opportunity.safety,
        })
    }

    /// One page of candidates.
    pub fn candidates_page(&self, req: CandidatesPageRequest) -> Result<CandidatesPage> {
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let rec = results.get(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
        let filter = req.filter.as_ref().map(|f| f.to_lowercase());
        let mut rows: Vec<&mj_core::Candidate> = rec
            .candidates
            .iter()
            .filter(|c| req.sub_group.is_none() || c.sub_group == req.sub_group)
            .filter(|c| filter.as_ref().is_none_or(|f| c.path.to_string_lossy().to_lowercase().contains(f)))
            .collect();
        match req.sort {
            CandidateSortKey::Saving => rows.sort_by_key(|c| c.saving()),
            CandidateSortKey::Size => rows.sort_by_key(|c| c.size),
            CandidateSortKey::Name => {
                rows.sort_by_key(|c| c.path.file_name().map(|n| n.to_string_lossy().to_lowercase()))
            }
            CandidateSortKey::Path => rows.sort_by(|a, b| a.path.cmp(&b.path)),
            CandidateSortKey::Modified => rows.sort_by_key(|c| c.modified),
            CandidateSortKey::Group => rows.sort_by_key(|c| (c.group, !c.badges.contains(Badges::KEEPER))),
        }
        if req.descending {
            rows.reverse();
        }
        let total = rows.len() as u64;
        let limit = req.limit.clamp(1, 500) as usize;
        let page: Vec<CandidateRow> = rows
            .into_iter()
            .skip(req.offset as usize)
            .take(limit)
            .map(|c| {
                let blocked = c.badges.intersects(Badges::BLOCKING);
                CandidateRow {
                    candidate: c.clone(),
                    selected: !blocked && rec.selection.is_selected(c.id, c.default_selected),
                    actionable: !blocked,
                    blocked_reason: blocked.then(|| "Protected path".to_string()),
                }
            })
            .collect();
        let groups =
            rec.groups.iter().filter(|g| page.iter().any(|r| r.candidate.group == Some(g.id))).cloned().collect();
        Ok(CandidatesPage { opportunity_id: req.opportunity_id, total, offset: req.offset, rows: page, groups })
    }

    /// Check / uncheck rows.
    pub fn selection_set(&self, req: SelectionSetRequest) -> Result<StepSummary> {
        let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
        let rec = results.get_mut(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
        let targets: Vec<(mj_core::CandidateId, bool)> = rec
            .candidates
            .iter()
            .filter(|c| match &req.ids {
                Some(ids) => ids.contains(&c.id),
                None => req.sub_group.is_none() || c.sub_group == req.sub_group,
            })
            .map(|c| (c.id, c.default_selected))
            .collect();
        for (id, def) in targets {
            rec.selection.set(id, def, req.selected);
        }
        recompute(rec);
        Ok(summary_of(rec))
    }

    /// Apply a keep rule to every group of the step.
    pub fn selection_apply_keep_rule(&self, req: SelectionKeepRuleRequest) -> Result<StepSummary> {
        let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
        let rec = results.get_mut(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
        let choices = mj_actions::select::apply_keep_rule(&rec.groups, &rec.candidates, &req.rule);
        for (id, selected) in choices {
            if let Some(c) = rec.candidates.iter().find(|c| c.id == id) {
                let def = c.default_selected;
                rec.selection.set(id, def, selected);
            }
        }
        recompute(rec);
        Ok(summary_of(rec))
    }

    /// Back to the default selection.
    pub fn selection_reset(&self, req: OpportunityRequest) -> Result<StepSummary> {
        let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
        let rec = results.get_mut(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
        rec.selection = Default::default();
        recompute(rec);
        Ok(summary_of(rec))
    }

    /// Change step parameters and re-estimate.
    pub fn step_params_set(&self, req: StepParamsRequest) -> Result<StepParamsResponse> {
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        results.get(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
        Err(Error::not_implemented("step parameter re-estimation (W3-D)"))
    }

    /// Preview of one candidate.
    pub fn candidate_preview(&self, req: CandidatePreviewRequest) -> Result<Preview> {
        let _ = req;
        Ok(Preview {
            kind: "none".into(),
            mime: None,
            data: None,
            text: None,
            waveform: None,
            reason: Some("Previews are not available yet".into()),
        })
    }
}
