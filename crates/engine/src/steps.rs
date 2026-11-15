//! Step review commands (B2: table, selection, parameters, preview). Owner: W3-D.
//!
//! Selection never makes a protected or locked row selected (see `results.rs`).
//! Every change recomputes the card, updates the plan step when the opportunity is in
//! the plan (`plan://changed`) and emits `opportunities://changed`.

use std::io::Read;

use mj_core::{lock, Badges, Error, ParamValue, Result, SafetyClass};

use crate::api::{
    CandidatePreviewRequest, CandidateRow, CandidatesPage, CandidatesPageRequest, Event, OpportunitiesChanged,
    OpportunityRequest, Preview, SelectionKeepRuleRequest, SelectionSetRequest, StepDetail, StepParamsRequest,
    StepParamsResponse, StepSummary,
};
use crate::opportunities::recompute;
use crate::results::{blocked_reason, is_selected, skip_badges, unknown, OpportunityRecord};
use crate::review_order::{fingerprint, order, OrderKey, Query};
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

/// ENG-2: after the analysis of a step ran again (new parameters), every row the user
/// ticked or unticked themselves keeps that choice, matched by path; every other row takes
/// the new run's default. Unlike a rescan (`results::carry_selections`), the files did not
/// change, so a row the new parameters no longer pick by default is not kept ticked just
/// because the old default ticked it.
pub(crate) fn carry_user_choices(old: &OpportunityRecord, new: &mut OpportunityRecord) {
    use mj_core::pathutil::path_key;
    let chosen: std::collections::HashMap<String, bool> = old
        .candidates
        .iter()
        .filter_map(|c| {
            if old.selection.include.contains(&c.id) {
                Some((path_key(&c.path), true))
            } else if old.selection.exclude.contains(&c.id) {
                Some((path_key(&c.path), false))
            } else {
                None
            }
        })
        .collect();
    let mut sel = mj_core::SelectionOverrides::default();
    for c in &new.candidates {
        if let Some(&on) = chosen.get(&path_key(&c.path)) {
            sel.set(c.id, c.default_selected, on);
        }
    }
    new.selection = sel;
}

fn none(reason: &str) -> Preview {
    Preview { kind: "none".into(), mime: None, data: None, text: None, waveform: None, reason: Some(reason.into()) }
}

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

    /// One page of candidates, flat or with groups kept together (W6-A, see
    /// `review_order`). The order of a request shape is cached across pages.
    pub fn candidates_page(&self, req: CandidatesPageRequest) -> Result<CandidatesPage> {
        let skip = skip_badges(&lock(&self.settings));
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let rec = results.get(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
        let has_groups = !rec.groups.is_empty();
        let limit = req.limit.clamp(1, 500) as usize;
        let query = Query {
            sub_group: req.sub_group.clone(),
            filter: req.filter.as_ref().map(|f| f.to_lowercase()).filter(|f| !f.is_empty()),
            min_size: req.min_size.filter(|&m| m > 0),
            sort: req.sort,
            descending: req.descending,
            keep_groups: req.keep_groups && has_groups,
            // The flat order does not depend on the page size.
            limit: if req.keep_groups && has_groups { limit } else { 0 },
        };
        let key = OrderKey {
            opportunity: req.opportunity_id.clone(),
            generation: results.generation,
            fingerprint: fingerprint(&rec.candidates),
            query,
        };
        let cached = lock(&self.review_order).get(&key);
        let order = match cached {
            Some(o) => o,
            None => {
                let o = std::sync::Arc::new(order(&rec.candidates, &key.query));
                lock(&self.review_order).put(key.clone(), std::sync::Arc::clone(&o));
                o
            }
        };
        let span = order.span(req.offset, limit);
        let total = order.total();
        let (start, end) = (span.start.min(total) as usize, span.end.min(total) as usize);
        let page: Vec<CandidateRow> = order.rows[start..end]
            .iter()
            .map(|&i| {
                let c = &rec.candidates[i as usize];
                let blocked = blocked_reason(c, skip);
                CandidateRow {
                    candidate: c.clone(),
                    selected: is_selected(rec, c, skip),
                    actionable: blocked.is_none(),
                    blocked_reason: blocked,
                }
            })
            .collect();
        let on_page: std::collections::HashSet<mj_core::GroupId> =
            page.iter().filter_map(|r| r.candidate.group).collect();
        let groups: Vec<mj_core::CandidateGroup> =
            rec.groups.iter().filter(|g| on_page.contains(&g.id)).cloned().collect();
        let group_stats = groups.iter().filter_map(|g| order.stats.get(&g.id).copied()).collect();
        Ok(CandidatesPage {
            opportunity_id: req.opportunity_id,
            total,
            offset: span.start,
            rows: page,
            groups,
            group_stats,
            has_groups,
            keep_groups: key.query.keep_groups,
            page: span.page,
            pages: span.pages,
            next_offset: span.next,
            prev_offset: span.prev,
        })
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
            for (id, selected) in choices {
                if let Some(c) = rec.candidates.iter_mut().find(|c| c.id == id) {
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
            for g in &mut rec.groups {
                g.keeper = rec
                    .candidates
                    .iter()
                    .find(|c| c.group == Some(g.id) && c.badges.contains(Badges::KEEPER))
                    .map(|c| c.path.clone());
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
        let generation = self.results.read().unwrap_or_else(|e| e.into_inner()).generation;
        let mut rec = self.rerun_analysis(&req.opportunity_id, params.clone())?;
        let lossy = params_lossy(&rec, &params);
        let safety = if lossy && !matches!(rec.opportunity.safety, SafetyClass::ReviewOnly) {
            SafetyClass::LossyOptIn
        } else {
            rec.opportunity.safety
        };
        rec.opportunity.params = params.clone();
        let summary = {
            let skip = skip_badges(&lock(&self.settings));
            let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
            // The re-run read the index of `generation`: a scan that finished meanwhile
            // replaced the results, and this record must not overwrite them.
            if results.generation != generation {
                return Err(Error::Conflict("a new scan finished while the step was re-estimated; try again".into()));
            }
            let slot = results.get_mut(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
            // ENG-2: the rows the user ticked or unticked keep their choice.
            carry_user_choices(slot, &mut rec);
            recompute(&mut rec, skip);
            let summary = summary_of(&rec);
            *slot = rec;
            summary
        };
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

    /// Preview of one candidate (SAF-02): a bounded image thumbnail, the first 4 KiB of text.
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
        if crate::thumbs::is_image(&path) {
            // A real bounded thumbnail (at most 512 px; IPC-9), never the whole file.
            return Ok(match self.thumbnail_of(&path, badges, target, crate::thumbs::PREVIEW_SIDE) {
                Ok(img) => Preview {
                    kind: "image".into(),
                    mime: Some(img.mime.into()),
                    data: Some(img.base64.clone()),
                    text: None,
                    waveform: None,
                    reason: None,
                },
                Err(why) => none(why.sentence()),
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
