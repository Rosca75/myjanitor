//! Custom search (rule builder + quick query, CLN-ENG-*). Owner: W3-D.
//!
//! `search_run` evaluates a rule over the current index through `mj-filters`
//! (global exclusions applied, three-valued semantics, media conditions answered from
//! the scan cache's fingerprints) and turns the matches into a `custom:<n>` card that
//! behaves like any other (step review, plan). Nothing is pre-selected unless the
//! default action is reversible; protected and confidential rows are badged and locked.

use std::sync::Arc;

use mj_core::{
    lock, Action, ActionKind, Availability, Candidate, CandidateId, Category, Confidence, DetectorOutput, EntryKind,
    Error, FileEntry, FingerprintStore, Opportunity, ParamValues, Result, Source, TargetKind,
};
use mj_filters::content::ContentConfig;
use mj_filters::media::{MediaMeta, MediaMetaProvider, MediaSource};

use crate::api::{
    Event, OpportunitiesChanged, SearchParseRequest, SearchParseResponse, SearchRunRequest, SearchSuggestRequest,
    Suggestion,
};
use crate::opportunities::{build_record, CardInfo};
use crate::Engine;

/// At most this many matches become rows (the rest is reported in the diagnostics).
const MAX_ROWS: usize = 200_000;

/// Media metadata from the scan cache (resolution, EXIF date, camera, GPS): no file I/O.
struct CacheMedia {
    engine: Arc<Engine>,
}

impl MediaMetaProvider for CacheMedia {
    fn media_meta(&self, entry: &FileEntry) -> Option<MediaMeta> {
        let fp = self.engine.cache.get(&entry.path, entry.stamp())?;
        if fp.width.is_none() && fp.exif.is_none() {
            return None;
        }
        let exif = fp.exif.clone().unwrap_or_default();
        Some(MediaMeta {
            width: fp.width,
            height: fp.height,
            camera_model: exif.camera.clone(),
            date_taken: exif.date_taken,
            has_gps: fp.exif.as_ref().map(|e| e.has_gps),
            ..MediaMeta::default()
        })
    }
}

impl Engine {
    /// Parse a quick query.
    pub fn search_parse(&self, req: SearchParseRequest) -> Result<SearchParseResponse> {
        Ok(match mj_filters::parse(&req.query) {
            Ok(rule) => SearchParseResponse { rule: Some(rule), errors: Vec::new() },
            Err(errors) => SearchParseResponse { rule: None, errors },
        })
    }

    /// Autocomplete.
    pub fn search_suggest(&self, req: SearchSuggestRequest) -> Result<Vec<Suggestion>> {
        Ok(mj_filters::suggest(&req.query, req.cursor))
    }

    /// Run a rule over the index as a custom opportunity.
    pub fn search_run(self: &Arc<Self>, req: SearchRunRequest) -> Result<Opportunity> {
        let index = {
            let r = self.results.read().unwrap_or_else(|e| e.into_inner());
            r.index.clone().ok_or_else(|| Error::InvalidInput("scan first".into()))?
        };
        let settings = lock(&self.settings).clone();
        let groups = mj_filters::groups::effective(&settings.type_groups);
        let ctx = mj_filters::FilterContext { now: mj_core::Timestamp::now(), type_groups: &groups };
        let exclusions = mj_filters::Exclusions::from_entries(&settings.safety.exclusions);
        let content = ContentConfig {
            media: Some(MediaSource(Arc::new(CacheMedia { engine: Arc::clone(self) }))),
            // CLN-CNT-01: PDF, Word, Excel and PowerPoint text.
            extractors: crate::extract::builtin(),
            ..ContentConfig::default()
        };
        let cancel = mj_core::CancelToken::new();
        let sel = mj_filters::select_with(
            &index,
            &req.rule,
            &ctx,
            mj_filters::SelectOptions {
                topmost_only: req.topmost_only,
                cancel: Some(&cancel),
                exclusions: Some(&exclusions),
                content,
            },
        )?;
        let action = req.action.clone().unwrap_or(Action::Trash);
        let removal = crate::plan::removes(&action);
        let preselect = removal && !action.safety(false).requires_confirmation();
        // RF-1: a whole folder is pre-selected only when the rule asks for folders
        // (`kind:dir`); one matched on its name, path length or attributes is only listed.
        let preselect_folders = preselect && mj_filters::asks_for_folders(&req.rule);
        let mut out = DetectorOutput { diagnostics: sel.diagnostics.to_core(), ..Default::default() };
        let (title, custom_id) = {
            let mut r = self.results.write().unwrap_or_else(|e| e.into_inner());
            let title = req.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Custom search".into());
            let existing = r
                .opportunities
                .iter()
                .find(|o| matches!(o.opportunity.source, Source::Custom(_)) && o.opportunity.title == title);
            let id = match existing.map(|o| o.opportunity.source.clone()) {
                Some(Source::Custom(id)) => id,
                _ => {
                    r.next_custom += 1;
                    r.next_custom.to_string()
                }
            };
            (title, id)
        };
        for &p in sel.positions.iter().take(MAX_ROWS) {
            let e = index.entry(p);
            let (target, size, files, selected) = match e.kind {
                EntryKind::Dir => {
                    let t = index.totals(p);
                    (TargetKind::Folder, t.reclaimable, Some(t.files), preselect_folders)
                }
                EntryKind::File => (TargetKind::File, e.reclaimable(), None, preselect),
                // Links are never targets of a custom search (they would not be followed).
                _ => continue,
            };
            out.candidates.push(Candidate {
                id: CandidateId(0),
                path: e.path.clone(),
                target,
                source: Source::Custom(custom_id.clone()),
                size,
                size_after: if removal { 0 } else { size },
                confidence: Confidence::Exact,
                reason: "Matches the search".into(),
                badges: mj_core::Badges::empty(),
                default_selected: selected,
                group: None,
                sub_group: None,
                modified: e.modified,
                stamp: e.stamp(),
                file_count: files,
                columns: Default::default(),
            });
        }
        if sel.positions.len() > MAX_ROWS {
            out.diagnostics
                .notes
                .push(format!("{} more matches are not listed; narrow the search", sel.positions.len() - MAX_ROWS));
        }
        let card = CardInfo {
            id: mj_core::OpportunityId(format!("custom:{custom_id}")),
            source: Source::Custom(custom_id),
            category: Category::Clean,
            title,
            description: "Custom search".into(),
            safety: action.safety(false),
            default_action: action.kind(),
            available_actions: vec![
                ActionKind::Trash,
                ActionKind::Quarantine,
                ActionKind::Move,
                ActionKind::Copy,
                ActionKind::PermanentDelete,
            ],
            presets: Vec::new(),
            settings: Vec::new(),
            unit: "files".into(),
            feature_ids: vec!["CLN-ENG-01".into(), "CLN-ENG-05".into()],
            availability: Availability::Available,
        };
        let badges = self.badger(&index, &out, false, &cancel);
        let mut rec = build_record(self, card, out, ParamValues::default(), &|c| {
            badges.get(&c.path).copied().unwrap_or_default()
        });
        rec.opportunity.default_action = action;
        let opp = rec.opportunity.clone();
        let generation = {
            let mut r = self.results.write().unwrap_or_else(|e| e.into_inner());
            match r.get_mut(&opp.id) {
                Some(slot) => *slot = rec,
                None => r.opportunities.push(rec),
            }
            r.generation
        };
        if self.in_plan(&opp.id).is_some() {
            let plan = self.refresh_plan();
            let _ = self.plan_changed(plan);
        }
        self.emit(Event::OpportunitiesChanged(OpportunitiesChanged { generation }));
        Ok(opp)
    }
}
