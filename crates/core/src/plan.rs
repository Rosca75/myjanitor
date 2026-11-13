//! The plan ("Your plan" panel): ordered steps, each an opportunity plus an action.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::action::{Action, OriginalsPolicy};
use crate::candidate::CandidateId;
use crate::estimate::Estimate;
use crate::opportunity::OpportunityId;
use crate::params::ParamValues;
use crate::safety::SafetyClass;
use crate::time::Timestamp;

/// Plan step id (unique within a session).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StepId(pub u64);

/// The user's deviations from the candidates' `default_selected` flags.
/// Session-only: candidate ids are not stable across scans, so saved schedules drop it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SelectionOverrides {
    /// Selected although `default_selected` is false.
    pub include: BTreeSet<CandidateId>,
    /// Deselected although `default_selected` is true.
    pub exclude: BTreeSet<CandidateId>,
}

impl SelectionOverrides {
    /// Effective selection of one candidate.
    pub fn is_selected(&self, id: CandidateId, default_selected: bool) -> bool {
        if default_selected {
            !self.exclude.contains(&id)
        } else {
            self.include.contains(&id)
        }
    }

    /// Record an explicit choice.
    pub fn set(&mut self, id: CandidateId, default_selected: bool, selected: bool) {
        self.include.remove(&id);
        self.exclude.remove(&id);
        if selected != default_selected {
            if selected {
                self.include.insert(id);
            } else {
                self.exclude.insert(id);
            }
        }
    }
}

/// One step of the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: StepId,
    pub opportunity: OpportunityId,
    /// Title shown in the plan ("Exact duplicates").
    pub title: String,
    /// Sub-line ("Keep newest · others to Recycle Bin").
    pub summary: String,
    pub action: Action,
    /// Step parameters (detector/optimiser settings), merged over the opportunity's.
    pub params: ParamValues,
    #[serde(default)]
    pub selection: SelectionOverrides,
    pub estimate: Estimate,
    pub safety: SafetyClass,
    /// The step is lossy and needs the confirmation dialog.
    pub lossy: bool,
}

/// The plan.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Plan {
    /// Session-scoped plan id.
    pub id: u64,
    pub name: String,
    pub steps: Vec<PlanStep>,
    /// Sum of step estimates.
    pub estimate: Estimate,
    /// "Before you apply" checklist, computed by the engine.
    pub checklist: Vec<ChecklistItem>,
    pub updated: Timestamp,
}

/// Severity/colour of a checklist row: ok = green check, warning = amber "!", info = blue "!".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecklistLevel {
    Ok,
    Warning,
    Info,
}

/// One "Before you apply" row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub level: ChecklistLevel,
    /// Stable code (`undo_window`, `lossy_steps`, `skipped_protected`, `synced_files`, ...).
    pub code: String,
    pub text: String,
}

/// The user's answer to the confirmation dialog for one lossy/irreversible step (SAF-11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepConfirmation {
    pub step_id: StepId,
    /// "I understand ..." checkbox. Must be true.
    pub acknowledged: bool,
    /// Originals policy chosen in the dialog (OPT-ENG-06).
    pub originals: OriginalsPolicy,
    /// Synced-folder alternative chosen: make synced originals online-only instead of
    /// removing them (SAF-10, CLN-ACT-07).
    #[serde(default)]
    pub synced_online_only: bool,
}
