//! Step review commands (B2: table, selection, parameters, preview). Owner: W3-D.
//!
//! Selection never makes a protected or locked row selected (see `results.rs`).
//! Every change recomputes the card, updates the plan step when the opportunity is in
//! the plan (`plan://changed`) and emits `opportunities://changed`.

use std::cmp::Ordering;
use std::io::Read;

use mj_core::{lock, Badges, Error, ParamValue, Result, SafetyClass};

use crate::api::{
    CandidatePreviewRequest, CandidateRow, CandidateSortKey, CandidatesPage, CandidatesPageRequest, Event,
    OpportunitiesChanged, OpportunityRequest, Preview, SelectionKeepRuleRequest, SelectionSetRequest, StepDetail,
    StepParamsRequest, StepParamsResponse, StepSummary,
};
use crate::opportunities::recompute;
use crate::results::{blocked_reason, is_selected, skip_badges, unknown, OpportunityRecord};
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

/// True when the parameters make the step lossy (a lossy setting switched on, or the
/// values of a lossy preset).
pub(crate) fn params_lossy(rec: &OpportunityRecord, params: &mj_core::ParamValues) -> bool {
    let o = &rec.opportunity;
    let setting = o.settings.iter().any(|s| {
        s.lossy
            && match params.0.get(&s.key) {
                Some(ParamValue::Bool(b)) => *b,
                Some(_) => true,
                None => matches!(s.default_value(), ParamValue::Bool(true)),
            }
    });
    let preset = o.presets.iter().any(|p| p.lossy && p.values.0.iter().all(|(k, v)| params.0.get(k) == Some(v)));
    setting || preset
}

/// Minimal standard base64 (no external crate).
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn none(reason: &str) -> Preview {
    Preview { kind: "none".into(), mime: None, data: None, text: None, waveform: None, reason: Some(reason.into()) }
}

/// Image previews are the file itself, only for small web-displayable images.
const PREVIEW_IMAGE_MAX: u64 = 4 << 20;

impl Engine {
    fn after_selection_change(&self, opportunity: &mj_core::OpportunityId) {
        if self.in_plan(opportunity).is_some() {
            {
                // A dry-run measurement no longer applies to a changed selection.
                let mut st = lock(&self.plan);
                if let Some(step) = st.plan.steps.iter().find(|s| &s.opportunity == opportunity).map(|s| s.id) {
                    st.measured.remove(&step);
                }
            }
            let plan = self.refresh_plan();
            let _ = self.plan_changed(plan);
        }
        let generation = self.results.read().unwrap_or_else(|e| e.into_inner()).generation;
        self.emit(Event::OpportunitiesChanged(OpportunitiesChanged { generation }));
    }

    /// Header and right panel of a step.
    pub fn step_detail(&self, req: OpportunityRequest) -> Result<StepDetail> {
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let rec = results.get(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
        let step = self.in_plan(&req.opportunity_id);
        let (lossy, safety) = match step.and_then(|id| lock(&self.plan).plan.steps.iter().find(|s| s.id == id).cloned())
        {
            Some(s) => (s.lossy, s.safety),
            None => (rec.opportunity.safety == SafetyClass::LossyOptIn, rec.opportunity.safety),
        };
        let mut opportunity = rec.opportunity.clone();
        opportunity.in_plan = step.is_some();
        Ok(StepDetail {
            opportunity,
            summary: summary_of(rec),
            diagnostics: rec.diagnostics.clone(),
            step_id: step,
            lossy,
            safety,
        })
    }

    /// One page of candidates.
    pub fn candidates_page(&self, req: CandidatesPageRequest) -> Result<CandidatesPage> {
        let skip = skip_badges(&lock(&self.settings));
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let rec = results.get(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
        let filter = req.filter.as_ref().map(|f| f.to_lowercase()).filter(|f| !f.is_empty());
        let mut rows: Vec<&mj_core::Candidate> = rec
            .candidates
            .iter()
            .filter(|c| req.sub_group.is_none() || c.sub_group == req.sub_group)
            .filter(|c| filter.as_ref().is_none_or(|f| c.path.to_string_lossy().to_lowercase().contains(f)))
            .collect();
        // Ascending order with the path as a total tie-breaker; `saving`/`size` default to
        // biggest first, like the overview ("descending" reverses the default).
        let by_path = |a: &&mj_core::Candidate, b: &&mj_core::Candidate| a.path.cmp(&b.path);
        let cmp = |a: &&mj_core::Candidate, b: &&mj_core::Candidate| -> Ordering {
            match req.sort {
                CandidateSortKey::Saving => b.saving().cmp(&a.saving()).then_with(|| by_path(a, b)),
                CandidateSortKey::Size => b.size.cmp(&a.size).then_with(|| by_path(a, b)),
                CandidateSortKey::Name => {
                    let n = |c: &mj_core::Candidate| c.path.file_name().map(|n| n.to_string_lossy().to_lowercase());
                    n(a).cmp(&n(b)).then_with(|| by_path(a, b))
                }
                CandidateSortKey::Path => by_path(a, b),
                CandidateSortKey::Modified => a.modified.cmp(&b.modified).then_with(|| by_path(a, b)),
                CandidateSortKey::Group => (a.group, !a.badges.contains(Badges::KEEPER))
                    .cmp(&(b.group, !b.badges.contains(Badges::KEEPER)))
                    .then_with(|| by_path(a, b)),
            }
        };
        rows.sort_by(cmp);
        if req.descending {
            rows.reverse();
        }
        let total = rows.len() as u64;
        let limit = req.limit.clamp(1, 500) as usize;
        let page: Vec<CandidateRow> = rows
            .into_iter()
            .skip(usize::try_from(req.offset).unwrap_or(usize::MAX))
            .take(limit)
            .map(|c| {
                let blocked = blocked_reason(c, skip);
                CandidateRow {
                    candidate: c.clone(),
                    selected: is_selected(rec, c, skip),
                    actionable: blocked.is_none(),
                    blocked_reason: blocked,
                }
            })
            .collect();
        let groups =
            rec.groups.iter().filter(|g| page.iter().any(|r| r.candidate.group == Some(g.id))).cloned().collect();
        Ok(CandidatesPage { opportunity_id: req.opportunity_id, total, offset: req.offset, rows: page, groups })
    }

    /// Check / uncheck rows. Locked rows never become selected.
    pub fn selection_set(&self, req: SelectionSetRequest) -> Result<StepSummary> {
        let skip = skip_badges(&lock(&self.settings));
        let summary = {
            let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
            let rec = results.get_mut(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
            if let Some(ids) = &req.ids {
                if let Some(bad) = ids.iter().find(|id| !rec.candidates.iter().any(|c| c.id == **id)) {
                    return Err(Error::InvalidInput(format!("unknown candidate {} (scan again)", bad.0)));
                }
            }
            let targets: Vec<(mj_core::CandidateId, bool)> = rec
                .candidates
                .iter()
                .filter(|c| match &req.ids {
                    Some(ids) => ids.contains(&c.id),
                    None => req.sub_group.is_none() || c.sub_group == req.sub_group,
                })
                .filter(|c| !req.selected || blocked_reason(c, skip).is_none())
                // F-02: "Select all" (no ids) never selects the copy a group keeps; only an
                // explicit pick of that row does (and apply still keeps one per group).
                .filter(|c| !(req.selected && req.ids.is_none() && c.badges.contains(Badges::KEEPER)))
                .map(|c| (c.id, c.default_selected))
                .collect();
            for (id, def) in targets {
                rec.selection.set(id, def, req.selected);
            }
            recompute(rec, skip);
            summary_of(rec)
        };
        self.after_selection_change(&req.opportunity_id);
        Ok(summary)
    }

    /// Apply a keep rule to every group of the step (CLN-ACT-01).
    pub fn selection_apply_keep_rule(&self, req: SelectionKeepRuleRequest) -> Result<StepSummary> {
        let skip = skip_badges(&lock(&self.settings));
        let summary = {
            let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
            let rec = results.get_mut(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
            if rec.groups.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "{} has no groups to apply a keep rule to",
                    rec.opportunity.title
                )));
            }
            let choices = mj_actions::select::apply_keep_rule(&rec.groups, &rec.candidates, &req.rule);
            // ACT-7: index rows by id once (was a scan of every row per choice).
            let at: std::collections::HashMap<mj_core::CandidateId, usize> =
                rec.candidates.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
            for (id, selected) in choices {
                if let Some(c) = at.get(&id).map(|&i| &mut rec.candidates[i]) {
                    // The keeper badge follows the rule so the table shows what is kept.
                    if selected {
                        c.badges.remove(Badges::KEEPER);
                    } else {
                        c.badges |= Badges::KEEPER;
                    }
                    let blocked = blocked_reason(c, skip).is_some();
                    let def = c.default_selected;
                    rec.selection.set(id, def, selected && !blocked);
                }
            }
            // The first keeper row of each group, found in one pass.
            let mut keepers: std::collections::HashMap<mj_core::GroupId, &std::path::Path> =
                std::collections::HashMap::new();
            for c in rec.candidates.iter().filter(|c| c.badges.contains(Badges::KEEPER)) {
                if let Some(g) = c.group {
                    keepers.entry(g).or_insert(c.path.as_path());
                }
            }
            for g in &mut rec.groups {
                g.keeper = keepers.get(&g.id).map(|p| p.to_path_buf());
            }
            recompute(rec, skip);
            summary_of(rec)
        };
        self.after_selection_change(&req.opportunity_id);
        Ok(summary)
    }

    /// Back to the default selection.
    pub fn selection_reset(&self, req: OpportunityRequest) -> Result<StepSummary> {
        let skip = skip_badges(&lock(&self.settings));
        let summary = {
            let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
            let rec = results.get_mut(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
            rec.selection = Default::default();
            recompute(rec, skip);
            summary_of(rec)
        };
        self.after_selection_change(&req.opportunity_id);
        Ok(summary)
    }

    /// Change step parameters (preset first, then values) and re-estimate by running the
    /// analysis again on the current index.
    pub fn step_params_set(&self, req: StepParamsRequest) -> Result<StepParamsResponse> {
        let (mut params, presets, keys) = {
            let results = self.results.read().unwrap_or_else(|e| e.into_inner());
            let rec = results.get(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
            if !rec.opportunity.availability.is_available() {
                return Err(Error::unsupported(rec.opportunity.title.clone(), "this opportunity is disabled here"));
            }
            let keys: Vec<String> = rec.opportunity.settings.iter().map(|s| s.key.clone()).collect();
            (rec.opportunity.params.clone(), rec.opportunity.presets.clone(), keys)
        };
        if let Some(pid) = &req.preset_id {
            let p = presets
                .iter()
                .find(|p| &p.id == pid)
                .ok_or_else(|| Error::InvalidInput(format!("unknown preset {pid}")))?;
            params = params.merged(&p.values);
        }
        if let Some(k) =
            req.params.0.keys().find(|k| !keys.contains(k) && !presets.iter().any(|p| p.values.0.contains_key(*k)))
        {
            return Err(Error::InvalidInput(format!("unknown parameter `{k}`")));
        }
        params = params.merged(&req.params);
        let mut rec = self.rerun_analysis(&req.opportunity_id, params.clone())?;
        let lossy = params_lossy(&rec, &params);
        let safety = if lossy && !matches!(rec.opportunity.safety, SafetyClass::ReviewOnly) {
            SafetyClass::LossyOptIn
        } else {
            rec.opportunity.safety
        };
        rec.opportunity.params = params.clone();
        let summary = summary_of(&rec);
        {
            let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = results.get_mut(&req.opportunity_id) {
                *slot = rec;
            }
        }
        if self.in_plan(&req.opportunity_id).is_some() {
            {
                let mut st = lock(&self.plan);
                let mut measured_drop = None;
                if let Some(step) = st.plan.steps.iter_mut().find(|s| s.opportunity == req.opportunity_id) {
                    step.params = params.clone();
                    step.lossy = lossy;
                    if let mj_core::Action::Optimise { params: p, .. } = &mut step.action {
                        *p = params.clone();
                    }
                    measured_drop = Some(step.id);
                }
                if let Some(id) = measured_drop {
                    st.measured.remove(&id);
                }
            }
            let plan = self.refresh_plan();
            let _ = self.plan_changed(plan);
        }
        let generation = self.results.read().unwrap_or_else(|e| e.into_inner()).generation;
        self.emit(Event::OpportunitiesChanged(OpportunitiesChanged { generation }));
        Ok(StepParamsResponse { params, summary, lossy, safety })
    }

    /// Preview of one candidate (SAF-02): small images as-is, the first 4 KiB of text.
    /// Online-only files are never read (reading would download them).
    pub fn candidate_preview(&self, req: CandidatePreviewRequest) -> Result<Preview> {
        let (path, badges, target) = {
            let results = self.results.read().unwrap_or_else(|e| e.into_inner());
            let (_, c) = results
                .find_candidate(req.candidate_id)
                .ok_or_else(|| Error::InvalidInput(format!("unknown candidate {} (scan again)", req.candidate_id.0)))?;
            (c.path.clone(), c.badges, c.target)
        };
        if target != mj_core::TargetKind::File {
            return Ok(none("Folders have no preview"));
        }
        if badges.contains(Badges::ONLINE_ONLY) {
            return Ok(none("Online-only file: a preview would download it"));
        }
        let ext = mj_core::pathutil::extension_lower(&path);
        let md = std::fs::symlink_metadata(&path).map_err(|e| Error::io(&path, e))?;
        if !md.is_file() {
            return Ok(none("Not a regular file"));
        }
        let mime = match ext.as_str() {
            "jpg" | "jpeg" => Some("image/jpeg"),
            "png" => Some("image/png"),
            "gif" => Some("image/gif"),
            "webp" => Some("image/webp"),
            "bmp" => Some("image/bmp"),
            _ => None,
        };
        if let Some(mime) = mime {
            if md.len() > PREVIEW_IMAGE_MAX {
                return Ok(none("Image too large for a quick preview"));
            }
            let data = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
            return Ok(Preview {
                kind: "image".into(),
                mime: Some(mime.into()),
                data: Some(base64(&data)),
                text: None,
                waveform: None,
                reason: None,
            });
        }
        if matches!(ext.as_str(), "mp3" | "wav" | "m4a" | "flac" | "ogg" | "opus" | "aac" | "wma") {
            return Ok(none("Waveform previews come with the audio optimiser"));
        }
        let mut f = std::fs::File::open(&path).map_err(|e| Error::io(&path, e))?;
        let mut buf = vec![0u8; 4096];
        let mut n = 0;
        while n < buf.len() {
            match f.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) => return Err(Error::io(&path, e)),
            }
        }
        buf.truncate(n);
        if buf.contains(&0) {
            return Ok(none("No preview for this file type"));
        }
        let mut text = String::from_utf8_lossy(&buf).into_owned();
        // Keep only printable text (file contents are hostile; the UI renders as text).
        text.retain(|c| !c.is_control() || c == '\n' || c == '\t');
        Ok(Preview {
            kind: "text".into(),
            mime: Some("text/plain".into()),
            data: None,
            text: Some(text),
            waveform: None,
            reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
