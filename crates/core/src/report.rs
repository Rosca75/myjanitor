//! Operation reports (SAF-04), result exports (CLN-ACT-11) and the audit export (SAF-12).

use serde::{Deserialize, Serialize};

use crate::job::{ItemOutcome, RunId};
use crate::time::Timestamp;

/// Export file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Html,
}

/// Totals of a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReportTotals {
    pub files: u64,
    pub succeeded: u64,
    pub skipped: u64,
    pub failed: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub bytes_saved: u64,
}

/// One step's section of a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportSection {
    pub title: String,
    /// Action summary ("To Recycle Bin").
    pub action: String,
    pub totals: ReportTotals,
    pub items: Vec<ItemOutcome>,
}

/// The report of one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub run_id: RunId,
    pub title: String,
    pub started: Timestamp,
    pub finished: Option<Timestamp>,
    pub totals: ReportTotals,
    pub sections: Vec<ReportSection>,
    /// Undo is possible until this time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_until: Option<Timestamp>,
}
