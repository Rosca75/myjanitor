//! Reports and exports (SAF-04, SAF-12, CLN-ACT-11). Owner: W3-D (writers in
//! `mj_actions::report`, W2-E). Exports never overwrite: `name (1).csv`.

use std::io::Write;

use mj_core::{lock, Candidate, Error, ExportFormat, Report, Result, RunKind};

use crate::api::{ExportResult, ReportExportKind, ReportExportRequest, ResultsExportRequest, RunRequest};
use crate::results::{is_selected, skip_badges};
use crate::Engine;

fn export_result((path, bytes, rows): (std::path::PathBuf, u64, u64)) -> ExportResult {
    ExportResult { path, bytes, rows }
}

/// Refuse export targets inside protected locations (the guard's rule for writes).
fn check_dest(engine: &Engine, dest: &std::path::Path) -> Result<()> {
    let settings = lock(&engine.settings).clone();
    let p = mj_actions::ProtectedPaths::new(&settings.safety.protected_paths);
    if p.is_protected(dest) {
        return Err(Error::Protected(dest.to_path_buf()));
    }
    Ok(())
}

impl Engine {
    /// Report of one run.
    pub fn report_get(&self, req: RunRequest) -> Result<Report> {
        let run =
            self.journal.run(req.run_id).ok_or_else(|| Error::InvalidInput(format!("unknown run {}", req.run_id.0)))?;
        let entries = self.journal.entries_of(run.run_id);
        let steps = lock(&self.run_meta).get(&run.run_id.0).map(|m| m.steps.clone()).unwrap_or_default();
        Ok(mj_actions::report::from_journal(&run, &entries, &steps))
    }

    /// The newest report of an apply or scheduled run, if any.
    pub fn report_last(&self) -> Result<Option<Report>> {
        let mut runs = self.journal.runs();
        runs.sort_by_key(|r| std::cmp::Reverse(r.run_id));
        match runs.into_iter().find(|r| r.kind != RunKind::Undo && !self.journal.entries_of(r.run_id).is_empty()) {
            Some(r) => self.report_get(RunRequest { run_id: r.run_id }).map(Some),
            None => Ok(None),
        }
    }

    /// Write a run's report (CSV/HTML) or the audit CSV.
    pub fn report_export(&self, req: ReportExportRequest) -> Result<ExportResult> {
        check_dest(self, &req.dest)?;
        match req.kind {
            ReportExportKind::Report => {
                let run_id = req.run_id.ok_or_else(|| Error::InvalidInput("a report needs a run id".into()))?;
                let report = self.report_get(RunRequest { run_id })?;
                let rows: u64 = report.sections.iter().map(|s| s.items.len() as u64).sum();
                let r = mj_actions::report::write_export(&req.dest, |out: &mut dyn Write| {
                    match req.format {
                        ExportFormat::Csv => mj_actions::report::report_csv(&report, out)?,
                        ExportFormat::Html => mj_actions::report::report_html(&report, out)?,
                    }
                    Ok(rows)
                })?;
                Ok(export_result(r))
            }
            ReportExportKind::Audit => {
                if req.format != ExportFormat::Csv {
                    return Err(Error::InvalidInput("the audit log is exported as CSV".into()));
                }
                let entries = match req.run_id {
                    Some(id) => {
                        self.journal.run(id).ok_or_else(|| Error::InvalidInput(format!("unknown run {}", id.0)))?;
                        self.journal.entries_of(id)
                    }
                    None => self.journal.all_entries(),
                };
                let rows = entries.len() as u64;
                let r = mj_actions::report::write_export(&req.dest, |out: &mut dyn Write| {
                    mj_actions::report::audit_csv(&entries, out)?;
                    Ok(rows)
                })?;
                Ok(export_result(r))
            }
        }
    }

    /// Export candidates of one or all opportunities (CLN-ACT-11). The `selected`
    /// column is the effective selection.
    pub fn results_export(&self, req: ResultsExportRequest) -> Result<ExportResult> {
        check_dest(self, &req.dest)?;
        let skip = skip_badges(&lock(&self.settings));
        let (title, rows) = {
            let results = self.results.read().unwrap_or_else(|e| e.into_inner());
            if results.index.is_none() {
                return Err(Error::InvalidInput("scan first".into()));
            }
            let recs: Vec<_> = match &req.opportunity_id {
                Some(id) => vec![results.get(id).ok_or_else(|| crate::results::unknown(id))?],
                None => results.opportunities.iter().collect(),
            };
            let title = match &req.opportunity_id {
                Some(_) => recs[0].opportunity.title.clone(),
                None => "MyJanitor results".to_string(),
            };
            let mut rows: Vec<Candidate> = Vec::new();
            for rec in recs {
                for c in &rec.candidates {
                    let sel = is_selected(rec, c, skip);
                    if req.selected_only && !sel {
                        continue;
                    }
                    let mut c = c.clone();
                    // The writers print `default_selected` as the selected column.
                    c.default_selected = sel;
                    rows.push(c);
                }
            }
            (title, rows)
        };
        let n = rows.len() as u64;
        let r = mj_actions::report::write_export(&req.dest, |out: &mut dyn Write| {
            match req.format {
                ExportFormat::Csv => mj_actions::report::candidates_csv(&rows, out)?,
                ExportFormat::Html => mj_actions::report::candidates_html(&title, &rows, out)?,
            }
            Ok(n)
        })?;
        Ok(export_result(r))
    }
}
