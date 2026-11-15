//! Opportunity cards (Option B overview) and the savings summary.

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionKind};
use crate::badge::Badges;
use crate::candidate::Source;
use crate::estimate::Confidence;
use crate::params::{ParamValues, SettingSpec, StepPreset};
use crate::safety::{SafetyClass, SavingsBucket};

/// Overview tab / nav section a card belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Clean,
    Optimise,
    Archive,
}

/// Whether a feature can run here; disabled features stay visible with a reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Availability {
    Available,
    /// One-line reason, e.g. "Requires ffmpeg on PATH" or "Windows only".
    Disabled {
        reason: String,
    },
}

impl Availability {
    /// True when available.
    pub fn is_available(&self) -> bool {
        matches!(self, Availability::Available)
    }
}

/// Stable opportunity id: `"det:<detector_id>"`, `"opt:<optimiser_id>"` or `"custom:<n>"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpportunityId(pub String);

/// A sub-group tab of the step review ("Same meeting twice 6").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubGroup {
    /// Matches `Candidate::sub_group`.
    pub key: String,
    pub label: String,
    pub count: u64,
}

/// One card of the overview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Opportunity {
    pub id: OpportunityId,
    pub source: Source,
    pub category: Category,
    pub title: String,
    /// One-line description with live numbers ("214 files in 61 groups · keeps newest").
    pub description: String,
    pub safety: SafetyClass,
    /// Current size of the rows `saving` counts.
    pub size_before: u64,
    /// Bytes this card frees with its current selection and action: the number the card
    /// shows. In `opportunities_list` it is the card's share of `SavingsSummary::total`
    /// (every file counted once, see `overlap`); in `step_detail` the card on its own.
    pub saving: u64,
    pub confidence: Confidence,
    /// Count shown on the card, in `unit`s.
    pub count: u64,
    pub unit: String,
    /// Total candidates (selected or not).
    pub candidates: u64,
    pub selected: u64,
    /// Saving of selected rows that is not in `saving` because another opportunity counts
    /// the same file (it frees more there, removes the folder holding it, or keeps the
    /// file as the last copy of a group). `saving + overlap` is the card on its own.
    #[serde(default)]
    pub overlap: u64,
    /// Current size of the rows listed for review: not selected (and selectable), or
    /// selected while the card's action changes nothing. Never part of `saving`.
    #[serde(default)]
    pub to_review: u64,
    /// Number of rows behind `to_review`.
    #[serde(default)]
    pub to_review_count: u64,
    /// Union of the candidates' badges.
    pub badges: Badges,
    pub default_action: Action,
    pub available_actions: Vec<ActionKind>,
    pub presets: Vec<StepPreset>,
    pub settings: Vec<SettingSpec>,
    pub params: ParamValues,
    #[serde(default)]
    pub sub_groups: Vec<SubGroup>,
    pub availability: Availability,
    /// True when a plan step references this opportunity.
    pub in_plan: bool,
    /// Ranking score; higher first in "Biggest saving first" (engine-defined, saving-based).
    pub rank: f64,
    pub feature_ids: Vec<String>,
}

/// Totals for the hero card and status strip.
///
/// `total` is what applying every available opportunity with its current selection and
/// action frees, every file counted once. It equals the sum of the cards' `saving` and
/// `reversible + opt_in + review_only` (each counted row in the bucket of its class).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SavingsSummary {
    pub total: u64,
    pub reversible: u64,
    pub opt_in: u64,
    pub review_only: u64,
    /// Available opportunities with at least one selected row (what `total` is "across").
    pub opportunities: u64,
    /// Sum of the cards' `overlap`: savings listed by several opportunities, counted once.
    #[serde(default)]
    pub overlap: u64,
    /// Available opportunities that list rows for review (`Opportunity::to_review_count`).
    #[serde(default)]
    pub review_opportunities: u64,
    /// Sum of the cards' `to_review`: current size of the rows listed for review. Never
    /// part of `total`; the hero names it in words next to the total.
    #[serde(default)]
    pub to_review: u64,
    /// Bytes freed by runs applied this session (status strip "Reclaimed").
    #[serde(default)]
    pub reclaimed: u64,
}

impl SavingsSummary {
    /// Count `saving` bytes of rows of class `safety`: into the total and their bucket.
    pub fn add(&mut self, safety: SafetyClass, saving: u64) {
        self.total += saving;
        match safety.bucket() {
            SavingsBucket::Reversible => self.reversible += saving,
            SavingsBucket::OptIn => self.opt_in += saving,
            SavingsBucket::ReviewOnly => self.review_only += saving,
        }
    }
}
