//! Report and export writers (SAF-04, SAF-12, CLN-ACT-11). **Owner: W2-E.**
//! CSV per RFC 4180 (UTF-8 with BOM for Excel, `\r\n`); HTML self-contained
//! (inline CSS, no script, every value HTML-escaped). Columns are listed in
//! docs/IPC.md §13.

use std::io::Write;

use mj_core::{Candidate, Error, JournalEntry, Report, Result};

/// Report of one run as CSV.
pub fn report_csv(report: &Report, out: &mut dyn Write) -> Result<()> {
    let _ = (report, out);
    Err(Error::not_implemented("report CSV (W2-E)"))
}

/// Report of one run as HTML.
pub fn report_html(report: &Report, out: &mut dyn Write) -> Result<()> {
    let _ = (report, out);
    Err(Error::not_implemented("report HTML (W2-E)"))
}

/// Audit log of journal entries (file, action, size before/after, SHA-256, time, state).
pub fn audit_csv(entries: &[JournalEntry], out: &mut dyn Write) -> Result<()> {
    let _ = (entries, out);
    Err(Error::not_implemented("audit CSV (W2-E)"))
}

/// Candidate list (results export) as CSV.
pub fn candidates_csv(candidates: &[Candidate], out: &mut dyn Write) -> Result<()> {
    let _ = (candidates, out);
    Err(Error::not_implemented("results CSV (W2-E)"))
}

/// Candidate list as HTML.
pub fn candidates_html(title: &str, candidates: &[Candidate], out: &mut dyn Write) -> Result<()> {
    let _ = (title, candidates, out);
    Err(Error::not_implemented("results HTML (W2-E)"))
}
