//! Opportunity building and the overview list. Owner: W3-D; numbers: W6-B.
//!
//! Ranking (frozen default): "Biggest saving first" orders by `saving` desc, then
//! title; `rank` = saving in bytes as f64. Disabled cards (unavailable optimiser,
//! detector not implemented here) are listed with their reason and never counted in
//! the hero summary; available cards with no candidate are hidden.
//!
//! Numbers (W6-B): every byte figure comes from [`crate::savings::tally`]. A record keeps
//! its own figures (each row on its own, with the card's default action) for the step
//! review; `opportunities_list` replaces them with the card's share of the hero total,
//! computed over every available card at once so that a file several cards list is
//! counted once. `summary.total` is therefore exactly the sum of the cards' `saving`,
//! and of its three buckets (each counted row in the bucket of its own safety class,
//! `columns["safety"]`, so a card mixing classes is split correctly).

use std::collections::BTreeMap;

use mj_core::{
    Action, ActionKind, Availability, Badges, Category, Confidence, DetectorInfo, DetectorOutput, GroupId,
    OpportunityId, OptimiserInfo, ParamValues, Result, SafetyClass, SavingsSummary, SelectionOverrides, Source,
    SubGroup,
};

use crate::api::{OpportunitiesList, OpportunitiesRequest, OpportunitySort, OpportunityTab, TabCounts};
use crate::results::{skip_badges, OpportunityRecord, Results};
use crate::savings::{tally, Part, Tally};
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
        overlap: 0,
        to_review: 0,
        to_review_count: 0,
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

/// Recompute the record's own numbers from its candidates and the current selection:
/// every row on its own, with the card's default action (what the step review shows).
/// Online-only rows never count towards savings (SCAN-09).
pub(crate) fn recompute(rec: &mut OpportunityRecord, skip: Badges) {
    let action = rec.opportunity.default_action.clone();
    let t = tally(&[Part { rec, action: &action }], skip, false).remove(0);
    let mut badges = Badges::empty();
    let mut subs: BTreeMap<String, u64> = BTreeMap::new();
    for c in &rec.candidates {
        badges |= c.badges;
        if let Some(s) = &c.sub_group {
            *subs.entry(s.clone()).or_default() += 1;
        }
    }
    let o = &mut rec.opportunity;
    o.candidates = rec.candidates.len() as u64;
    o.badges = badges;
    o.sub_groups = subs.into_iter().map(|(key, count)| SubGroup { label: key.replace('_', " "), key, count }).collect();
    put_numbers(o, &CardShare::of(&t));
}

/// The numbers of one card, as [`tally`] counted them.
#[derive(Debug, Clone)]
pub(crate) struct CardShare {
    selected: u64,
    size_before: u64,
    saving: u64,
    overlap: u64,
    to_review: u64,
    to_review_count: u64,
    confidence: Confidence,
    by_class: Vec<(SafetyClass, u64)>,
}

impl CardShare {
    fn of(t: &Tally) -> CardShare {
        CardShare {
            selected: t.selected,
            size_before: t.bytes_before,
            saving: t.saving(),
            overlap: t.overlap,
            to_review: t.to_review,
            to_review_count: t.to_review_rows,
            confidence: if t.selected == 0 { Confidence::Exact } else { t.confidence },
            by_class: t.by_class.values().copied().collect(),
        }
    }
}

/// Write a share into a card.
fn put_numbers(o: &mut mj_core::Opportunity, s: &CardShare) {
    o.selected = s.selected;
    o.count = s.selected;
    o.size_before = s.size_before;
    o.saving = s.saving;
    o.overlap = s.overlap;
    o.to_review = s.to_review;
    o.to_review_count = s.to_review_count;
    o.confidence = s.confidence;
    o.rank = s.saving as f64;
}

/// `(shares_key, shares)` of the last [`Engine::card_shares`] computation.
pub(crate) type SharesCache = (u64, std::sync::Arc<Vec<Option<CardShare>>>);

/// True for the cards the hero counts: available, with at least one candidate.
fn counted_card(rec: &OpportunityRecord) -> bool {
    rec.opportunity.availability.is_available() && !rec.candidates.is_empty()
}

/// Everything the shares depend on, hashed: the safety settings, and per counted card its
/// action, every row (id, size, estimate, badges, default), the selection and the groups'
/// kept copies. Paths never change under an id, so they are not hashed.
fn shares_key(results: &Results, actions: &[Option<&Action>], skip: Badges) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    skip.bits().hash(&mut h);
    results.generation.hash(&mut h);
    for (rec, action) in results.opportunities.iter().zip(actions) {
        let Some(action) = action else { continue };
        rec.opportunity.id.hash(&mut h);
        format!("{action:?}").hash(&mut h);
        rec.candidates.len().hash(&mut h);
        for c in &rec.candidates {
            (c.id, c.size, c.size_after, c.badges.bits(), c.default_selected).hash(&mut h);
        }
        rec.selection.include.hash(&mut h);
        rec.selection.exclude.hash(&mut h);
        for g in &rec.groups {
            (g.id, g.members.len()).hash(&mut h);
            g.keeper.hash(&mut h);
        }
    }
    h.finish()
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
    /// The share of every record in the hero total (`None` for cards the hero does not
    /// count), in `results.opportunities` order. A card in the plan is counted with its
    /// step's action, any other with its default action. Cached until an input changes.
    pub(crate) fn card_shares(&self, results: &Results, skip: Badges) -> std::sync::Arc<Vec<Option<CardShare>>> {
        let steps: Vec<(OpportunityId, Action)> =
            mj_core::lock(&self.plan).plan.steps.iter().map(|s| (s.opportunity.clone(), s.action.clone())).collect();
        let actions: Vec<Option<&Action>> = results
            .opportunities
            .iter()
            .map(|rec| {
                counted_card(rec).then(|| {
                    let id = &rec.opportunity.id;
                    steps.iter().find(|(o, _)| o == id).map(|(_, a)| a).unwrap_or(&rec.opportunity.default_action)
                })
            })
            .collect();
        let key = shares_key(results, &actions, skip);
        let mut cache = mj_core::lock(&self.shares);
        if let Some((k, shares)) = cache.as_ref() {
            if *k == key {
                return shares.clone();
            }
        }
        let parts: Vec<Part<'_>> = results
            .opportunities
            .iter()
            .zip(&actions)
            .filter_map(|(rec, a)| a.map(|action| Part { rec, action }))
            .collect();
        let mut tallies = tally(&parts, skip, true).into_iter();
        let shares: Vec<Option<CardShare>> =
            actions.iter().map(|a| a.and_then(|_| tallies.next()).map(|t| CardShare::of(&t))).collect();
        let shares = std::sync::Arc::new(shares);
        *cache = Some((key, shares.clone()));
        shares
    }

    /// Cards for a tab, the hero summary and the tab counts.
    pub fn opportunities_list(&self, req: OpportunitiesRequest) -> Result<OpportunitiesList> {
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let in_plan: Vec<OpportunityId> =
            mj_core::lock(&self.plan).plan.steps.iter().map(|s| s.opportunity.clone()).collect();
        let skip = skip_badges(&mj_core::lock(&self.settings));
        let shares = self.card_shares(&results, skip);
        let mut summary = SavingsSummary {
            reclaimed: self.reclaimed.load(std::sync::atomic::Ordering::Relaxed),
            ..SavingsSummary::default()
        };
        let mut counts = TabCounts::default();
        let mut items = Vec::new();
        for (rec, share) in results.opportunities.iter().zip(shares.iter()) {
            let mut o = rec.opportunity.clone();
            o.in_plan = in_plan.contains(&o.id);
            let available = o.availability.is_available();
            if rec.candidates.is_empty() && available {
                continue;
            }
            if let Some(share) = share {
                put_numbers(&mut o, share);
                for (class, bytes) in &share.by_class {
                    summary.add(*class, *bytes);
                }
                summary.overlap += share.overlap;
                summary.opportunities += u64::from(share.selected > 0);
                summary.review_opportunities += u64::from(share.to_review_count > 0);
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
        // Disabled cards always come after the available ones.
        let rank_of = |o: &mj_core::Opportunity| !o.availability.is_available();
        match req.sort {
            OpportunitySort::Saving => items.sort_by(|a, b| {
                rank_of(a).cmp(&rank_of(b)).then(b.saving.cmp(&a.saving)).then_with(|| a.title.cmp(&b.title))
            }),
            OpportunitySort::Count => items.sort_by(|a, b| {
                rank_of(a).cmp(&rank_of(b)).then(b.count.cmp(&a.count)).then_with(|| a.title.cmp(&b.title))
            }),
            OpportunitySort::Safety => items.sort_by(|a, b| {
                rank_of(a)
                    .cmp(&rank_of(b))
                    .then(safety_order(a.safety).cmp(&safety_order(b.safety)))
                    .then(b.saving.cmp(&a.saving))
                    .then_with(|| a.title.cmp(&b.title))
            }),
            OpportunitySort::Name => items.sort_by(|a, b| {
                rank_of(a).cmp(&rank_of(b)).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            }),
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
