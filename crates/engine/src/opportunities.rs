//! Opportunity building and the overview list. Owner: W3-D.
//!
//! Ranking (frozen default): "Biggest saving first" orders by `saving` desc, then
//! title; `rank` = saving in bytes as f64. Disabled cards (unavailable optimiser,
//! detector not implemented here) are listed with their reason and never counted in
//! the hero summary; available cards with no candidate are hidden. The hero summary
//! buckets each selected row by its own safety class (`columns["safety"]`), so a card
//! mixing classes is split correctly.

use std::collections::BTreeMap;

use mj_core::{
    Action, ActionKind, Availability, Badges, Category, Confidence, DetectorInfo, DetectorOutput, GroupId,
    OpportunityId, OptimiserInfo, ParamValues, Result, SafetyClass, SavingsBucket, SavingsSummary, SelectionOverrides,
    Source, SubGroup,
};

use crate::api::{OpportunitiesList, OpportunitiesRequest, OpportunitySort, OpportunityTab, TabCounts};
use crate::results::{candidate_safety, is_selected, severity, skip_badges, OpportunityRecord};
use crate::Engine;

/// Static card data shared by detectors and optimisers.
#[derive(Clone)]
pub(crate) struct CardInfo {
    pub id: OpportunityId,
    pub source: Source,
    pub category: Category,
    pub title: String,
    pub description: String,
    pub safety: SafetyClass,
    pub default_action: ActionKind,
    pub available_actions: Vec<ActionKind>,
    pub presets: Vec<mj_core::StepPreset>,
    pub settings: Vec<mj_core::SettingSpec>,
    pub unit: String,
    pub feature_ids: Vec<String>,
    pub availability: Availability,
}

impl CardInfo {
    /// From a detector.
    pub fn detector(i: DetectorInfo) -> Self {
        let id = OpportunityId(format!(
            "det:{}",
            serde_json::to_value(i.id).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
        ));
        let availability = if i.platform.here() {
            Availability::Available
        } else {
            Availability::Disabled { reason: "Not available on this platform".into() }
        };
        CardInfo {
            id,
            source: Source::Detector(i.id),
            category: i.category,
            title: i.title,
            description: i.description,
            safety: i.safety,
            default_action: i.default_action,
            available_actions: i.available_actions,
            presets: Vec::new(),
            settings: i.settings,
            unit: i.unit,
            feature_ids: i.feature_ids,
            availability,
        }
    }

    /// From an optimiser.
    pub fn optimiser(i: OptimiserInfo, availability: Availability) -> Self {
        let id = OpportunityId(format!(
            "opt:{}",
            serde_json::to_value(i.id).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
        ));
        CardInfo {
            id,
            source: Source::Optimiser(i.id),
            category: i.category,
            title: i.title,
            description: i.description,
            safety: i.safety,
            default_action: i.default_action,
            available_actions: i.available_actions,
            presets: i.presets,
            settings: i.settings,
            unit: i.unit,
            feature_ids: i.feature_ids,
            availability,
        }
    }
}

/// The concrete default action for an action kind (parameters from settings defaults).
pub(crate) fn default_action(kind: ActionKind, source: &Source, settings: &mj_core::Settings) -> Action {
    match (kind, source) {
        (ActionKind::Trash, _) => Action::Trash,
        (ActionKind::Quarantine, _) => Action::Quarantine { purge_after_days: settings.general.quarantine_days },
        (ActionKind::PermanentDelete, _) => Action::PermanentDelete,
        (ActionKind::Hardlink, _) => Action::Hardlink,
        (ActionKind::Reflink, _) => Action::Reflink,
        (ActionKind::Dehydrate, _) => Action::Dehydrate,
        (ActionKind::Optimise, Source::Optimiser(id)) => Action::Optimise {
            optimiser: *id,
            params: ParamValues::default(),
            output: mj_core::OutputMode::Replace,
            originals: settings.general.default_originals.clone(),
            keep_if_not_smaller: true,
        },
        (ActionKind::Archive, _) => Action::Archive {
            format: mj_core::ArchiveFormat::Zip,
            level: 9,
            dest: None,
            originals: settings.general.default_originals.clone(),
            keep_sources: false,
        },
        (ActionKind::FsCompress, _) => Action::FsCompress {
            algorithm: if cfg!(windows) {
                mj_core::FsCompressAlgo::Xpress8k
            } else {
                mj_core::FsCompressAlgo::BtrfsZstd
            },
        },
        (ActionKind::FsDecompress, _) => Action::FsDecompress,
        _ => Action::None,
    }
}

/// Turn one analysis output into a record: assign candidate ids, re-key groups,
/// apply badges and the auto-uncheck rule, compute card numbers. `params` are the
/// effective (user-visible) parameters the analysis ran with.
pub(crate) fn build_record(
    engine: &Engine,
    card: CardInfo,
    mut out: DetectorOutput,
    params: ParamValues,
    extra_badges: &dyn Fn(&mj_core::Candidate) -> Badges,
) -> OpportunityRecord {
    let settings = mj_core::lock(&engine.settings).clone();
    let skip = skip_badges(&settings);
    let mut regroup: BTreeMap<GroupId, GroupId> = BTreeMap::new();
    for g in &mut out.groups {
        let new = GroupId(engine.next_id());
        regroup.insert(g.id, new);
        g.id = new;
    }
    for c in &mut out.candidates {
        c.id = mj_core::CandidateId(engine.next_id());
        c.group = c.group.and_then(|g| regroup.get(&g).copied());
        c.badges |= extra_badges(c);
        if c.badges.intersects(Badges::LABELLED) {
            c.badges |= Badges::CONFIDENTIAL;
        }
        c.normalise_selection();
    }
    let action = default_action(card.default_action, &card.source, &settings);
    let opportunity = mj_core::Opportunity {
        id: card.id,
        source: card.source,
        category: card.category,
        title: card.title,
        description: card.description,
        safety: card.safety,
        size_before: 0,
        saving: 0,
        confidence: Confidence::Estimated,
        count: 0,
        unit: card.unit,
        candidates: 0,
        selected: 0,
        badges: Badges::empty(),
        default_action: action,
        available_actions: card.available_actions,
        params,
        presets: card.presets,
        settings: card.settings,
        sub_groups: Vec::new(),
        availability: card.availability,
        in_plan: false,
        rank: 0.0,
        feature_ids: card.feature_ids,
    };
    let mut rec = OpportunityRecord {
        opportunity,
        candidates: out.candidates,
        groups: out.groups,
        diagnostics: out.diagnostics,
        selection: SelectionOverrides::default(),
    };
    recompute(&mut rec, skip);
    rec
}

/// A card for an analysis that could not run here (shown greyed with the reason; never
/// presented as "nothing found").
pub(crate) fn disabled_record(engine: &Engine, mut card: CardInfo, reason: String) -> OpportunityRecord {
    card.availability = Availability::Disabled { reason };
    let params = mj_core::params::defaults(&card.settings);
    build_record(engine, card, DetectorOutput::default(), params, &|_| Badges::empty())
}

/// Recompute the card numbers from candidates and the current selection. Online-only
/// rows never count towards savings (SCAN-09).
pub(crate) fn recompute(rec: &mut OpportunityRecord, skip: Badges) {
    let (mut before, mut saving, mut selected, mut badges) = (0u64, 0u64, 0u64, Badges::empty());
    let mut conf = Confidence::Exact;
    let mut subs: BTreeMap<String, u64> = BTreeMap::new();
    for c in &rec.candidates {
        badges |= c.badges;
        if let Some(s) = &c.sub_group {
            *subs.entry(s.clone()).or_default() += 1;
        }
        if is_selected(rec, c, skip) {
            selected += 1;
            if !c.badges.contains(Badges::ONLINE_ONLY) {
                before += c.size;
                saving += c.saving();
            }
            conf = conf.max(c.confidence);
        }
    }
    let o = &mut rec.opportunity;
    o.candidates = rec.candidates.len() as u64;
    o.selected = selected;
    o.count = selected;
    o.size_before = before;
    o.saving = saving;
    o.confidence = if selected == 0 { Confidence::Exact } else { conf };
    o.badges = badges;
    o.rank = saving as f64;
    o.sub_groups = subs.into_iter().map(|(key, count)| SubGroup { label: key.replace('_', " "), key, count }).collect();
}

/// Savings of one record split by the class of each selected row.
pub(crate) fn split_by_safety(rec: &OpportunityRecord, skip: Badges) -> BTreeMap<u8, (SafetyClass, u64)> {
    let mut m: BTreeMap<u8, (SafetyClass, u64)> = BTreeMap::new();
    for c in &rec.candidates {
        if is_selected(rec, c, skip) && !c.badges.contains(Badges::ONLINE_ONLY) {
            let s = candidate_safety(rec, c);
            m.entry(severity(s)).or_insert((s, 0)).1 += c.saving();
        }
    }
    m
}
fn safety_order(s: SafetyClass) -> u8 {
    match s {
        SafetyClass::Reversible => 0,
        SafetyClass::Rebuildable => 1,
        SafetyClass::LightCopy => 2,
        SafetyClass::LossyOptIn => 3,
        SafetyClass::Permanent => 4,
        SafetyClass::ReviewOnly => 5,
    }
}

fn in_tab(o: &mj_core::Opportunity, tab: OpportunityTab) -> bool {
    match tab {
        OpportunityTab::All => true,
        OpportunityTab::Clean => o.category == Category::Clean && o.safety != SafetyClass::ReviewOnly,
        OpportunityTab::Optimise => o.category == Category::Optimise && o.safety != SafetyClass::ReviewOnly,
        OpportunityTab::Archive => o.category == Category::Archive && o.safety != SafetyClass::ReviewOnly,
        OpportunityTab::ReviewOnly => o.safety == SafetyClass::ReviewOnly,
    }
}

impl Engine {
    /// Cards for a tab, the hero summary and the tab counts.
    pub fn opportunities_list(&self, req: OpportunitiesRequest) -> Result<OpportunitiesList> {
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let in_plan: Vec<OpportunityId> =
            mj_core::lock(&self.plan).plan.steps.iter().map(|s| s.opportunity.clone()).collect();
        let skip = skip_badges(&mj_core::lock(&self.settings));
        let mut summary = SavingsSummary {
            reclaimed: self.reclaimed.load(std::sync::atomic::Ordering::Relaxed),
            ..SavingsSummary::default()
        };
        let mut counts = TabCounts::default();
        let mut items = Vec::new();
        for rec in &results.opportunities {
            let mut o = rec.opportunity.clone();
            o.in_plan = in_plan.contains(&o.id);
            let available = o.availability.is_available();
            if o.candidates == 0 && available {
                continue;
            }
            if available {
                summary.opportunities += 1;
                for (_, (class, bytes)) in split_by_safety(rec, skip) {
                    summary.total += bytes;
                    match class.bucket() {
                        SavingsBucket::Reversible => summary.reversible += bytes,
                        SavingsBucket::OptIn => summary.opt_in += bytes,
                        SavingsBucket::ReviewOnly => summary.review_only += bytes,
                    }
                }
            }
            counts.all += 1;
            for (tab, n) in [
                (OpportunityTab::Clean, &mut counts.clean),
                (OpportunityTab::Optimise, &mut counts.optimise),
                (OpportunityTab::Archive, &mut counts.archive),
                (OpportunityTab::ReviewOnly, &mut counts.review_only),
            ] {
                if in_tab(&o, tab) {
                    *n += 1;
                }
            }
            if in_tab(&o, req.tab) {
                items.push(o);
            }
        }
        match req.sort {
            OpportunitySort::Saving => {
                items.sort_by(|a, b| b.saving.cmp(&a.saving).then_with(|| a.title.cmp(&b.title)))
            }
            OpportunitySort::Count => items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.title.cmp(&b.title))),
            OpportunitySort::Safety => {
                items.sort_by(|a, b| safety_order(a.safety).cmp(&safety_order(b.safety)).then(b.saving.cmp(&a.saving)))
            }
            OpportunitySort::Name => items.sort_by_key(|a| a.title.to_lowercase()),
        }
        Ok(OpportunitiesList {
            generation: results.generation,
            items,
            summary,
            counts,
            scanned_at: results.index.as_ref().map(|i| i.scanned_at),
            cached: results.cached,
        })
    }
}
