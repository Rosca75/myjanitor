//! The undo journal: one record per action on one item, written **before** the
//! action (state `intent`) and updated after (`done`/`failed`). Enough to undo
//! every reversible action and to produce the audit CSV (SAF-03, SAF-12).
//!
//! On disk (owned by `mj-actions`): JSON Lines, append-only, one full
//! `JournalEntry` snapshot per line; the last line for an `id` wins on replay;
//! a torn final line is ignored. See ARCHITECTURE.md §7.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::action::ActionKind;
use crate::candidate::TargetKind;
use crate::entry::FileStamp;
use crate::error::ErrorInfo;
use crate::job::{JobId, RunId};
use crate::plan::StepId;
use crate::time::Timestamp;

/// Journal entry id (unique across sessions: run id * 1_000_000 + item index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(pub u64);

/// Lifecycle of a journal entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalState {
    /// Written before acting. An `intent` without a later record means the app stopped mid-action:
    /// recovery inspects both locations and settles it as `done` or `failed`.
    Intent,
    Done,
    Failed,
    /// Skipped before acting (changed, protected, confidential...).
    Skipped,
    Undone,
    UndoFailed,
    /// The retained original was purged (quarantine expiry, trash emptied); no longer undoable.
    Purged,
}

/// How to find a trashed item again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashTicket {
    /// Path of the item inside the trash (`~/.local/share/Trash/files/x`,
    /// `C:\$Recycle.Bin\<SID>\$Rxxxxx.ext`) when known.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub location: Option<PathBuf>,
    /// Metadata file (`.trashinfo`, `$Ixxxxx`) when known.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub info: Option<PathBuf>,
    pub trashed_at: Timestamp,
}

/// One journal record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: EntryId,
    pub run_id: RunId,
    pub job_id: JobId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<StepId>,
    /// Time of this record.
    pub at: Timestamp,
    pub state: JournalState,
    pub action: ActionKind,
    pub target: TargetKind,
    /// Where the item was before the action.
    #[serde(with = "crate::pathutil::serde_path")]
    pub original: PathBuf,
    pub original_stamp: FileStamp,
    /// SHA-256 (hex) of the original content, when computed (audit; files only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_sha256: Option<String>,
    pub bytes_before: u64,
    pub bytes_after: u64,
    /// Where the original now lives (quarantine path, move destination, backup copy).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub retained_at: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trash: Option<TrashTicket>,
    /// Files this action created (copy, optimised output, light copy, link, archive); undo removes them.
    #[serde(default, skip_serializing_if = "Vec::is_empty", with = "crate::pathutil::serde_path::vec")]
    pub created: Vec<PathBuf>,
    /// For hard/reflink replacement: the keeper the original was linked to.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub link_source: Option<PathBuf>,
    /// Undo is possible (until `undo_deadline`).
    pub reversible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_deadline: Option<Timestamp>,
    /// Lossy transform (audit).
    #[serde(default)]
    pub lossy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

/// Kind of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    /// "Apply plan" from the GUI or CLI.
    Apply,
    /// A scheduled profile.
    Schedule,
    /// Undo / restore of earlier entries.
    Undo,
}

/// Header record of a run, stored beside the entries (History list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: RunId,
    pub kind: RunKind,
    /// "Plan · 4 steps" or the schedule name.
    pub title: String,
    pub started: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<Timestamp>,
    /// Step titles in order.
    #[serde(default)]
    pub steps: Vec<String>,
    /// For undo runs: the run being undone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_of: Option<RunId>,
}
