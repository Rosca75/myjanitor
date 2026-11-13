//! Jobs: background work items shown in the queue (OPT-ENG-11).

use serde::{Deserialize, Serialize};

use crate::error::ErrorInfo;
use crate::plan::StepId;
use crate::time::Timestamp;

/// Job id (unique within a session, < 2^53).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

/// One "Apply plan" (or undo) run: groups its jobs, journal entries and report.
/// Persisted in the journal, so it is unique across sessions (milliseconds of the start time,
/// bumped on collision).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub u64);

/// What a job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    DryRun,
    Apply,
    Undo,
    PurgeQuarantine,
    Export,
}

/// Job lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Paused,
    Cancelling,
    Cancelled,
    Completed,
    /// Completed, but some items failed or were skipped.
    CompletedWithErrors,
    Failed,
}

impl JobState {
    /// True for terminal states.
    pub fn is_finished(self) -> bool {
        matches!(self, JobState::Cancelled | JobState::Completed | JobState::CompletedWithErrors | JobState::Failed)
    }
}

/// Live counters of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct JobProgress {
    pub done: u64,
    pub total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Bytes freed so far.
    pub saved: u64,
}

/// Result of one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Done,
    Skipped,
    Failed,
}

/// Outcome of one item, as listed in reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemOutcome {
    #[serde(with = "crate::pathutil::serde_path")]
    pub path: std::path::PathBuf,
    pub status: ItemStatus,
    pub bytes_before: u64,
    pub bytes_after: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

/// Totals of a finished job.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct JobOutcome {
    pub succeeded: u64,
    pub skipped: u64,
    pub failed: u64,
    pub bytes_saved: u64,
    /// First failures (bounded to 50; the report has all).
    pub errors: Vec<ItemOutcome>,
}

/// A queued or running job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    pub run_id: Option<RunId>,
    pub step_id: Option<StepId>,
    pub title: String,
    pub state: JobState,
    pub progress: JobProgress,
    pub created: Timestamp,
    pub started: Option<Timestamp>,
    pub finished: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<JobOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}
