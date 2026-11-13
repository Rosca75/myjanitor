//! Reports and exports (SAF-04, SAF-12, CLN-ACT-11). Owner: W3-D (writers in `mj_actions::report`, W2-E).

use mj_core::{Error, Report, Result};

use crate::api::{ExportResult, ReportExportRequest, ResultsExportRequest, RunRequest};
use crate::Engine;

impl Engine {
    /// Report of one run.
    pub fn report_get(&self, req: RunRequest) -> Result<Report> {
        let _ = req;
        Err(Error::not_implemented("reports (W3-D)"))
    }

    /// The newest report, if any.
    pub fn report_last(&self) -> Result<Option<Report>> {
        Ok(None)
    }

    /// Write a report or the audit log.
    pub fn report_export(&self, req: ReportExportRequest) -> Result<ExportResult> {
        let _ = req;
        Err(Error::not_implemented("report export (W3-D)"))
    }

    /// Export candidates.
    pub fn results_export(&self, req: ResultsExportRequest) -> Result<ExportResult> {
        let _ = req;
        Err(Error::not_implemented("results export (W3-D)"))
    }
}
