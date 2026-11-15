//! Report and export writers (SAF-04, SAF-12, CLN-ACT-11). **Owner: W2-E.**
//! CSV per RFC 4180 (UTF-8 with BOM for Excel, `\r\n`); HTML self-contained
//! (inline CSS, no script, every value HTML-escaped). Columns are listed in
//! docs/IPC.md (`report_export`, `results_export`).
//!
//! CSV cells are neutralised against formula injection: a text cell starting
//! with `=`, `+`, `-`, `@`, a tab or a carriage return is prefixed with `'` so
//! spreadsheet programs show it as text instead of evaluating it (OWASP
//! guidance). Numbers written by MyJanitor itself are never prefixed.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use mj_core::{
    ActionKind, Candidate, Error, ItemOutcome, ItemStatus, JournalEntry, JournalState, Report, ReportSection,
    ReportTotals, Result, RunRecord, Source, Timestamp,
};

const BOM: &[u8] = b"\xEF\xBB\xBF";

fn w(out: &mut dyn Write, s: &str) -> Result<()> {
    out.write_all(s.as_bytes()).map_err(|e| Error::Io { path: None, kind: "other", message: e.to_string() })
}

/// One CSV text cell: formula-neutralised, quoted when needed.
pub fn csv_cell(s: &str) -> String {
    let neutral = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    if neutral.contains([',', '"', '\n', '\r']) || neutral.starts_with(' ') || neutral.ends_with(' ') {
        format!("\"{}\"", neutral.replace('"', "\"\""))
    } else {
        neutral
    }
}

/// HTML-escape text content and attribute values.
pub fn html_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => o.push('\u{fffd}'),
            c => o.push(c),
        }
    }
    o
}

fn row(out: &mut dyn Write, cells: &[String]) -> Result<()> {
    w(out, &cells.join(","))?;
    w(out, "\r\n")
}

fn text(s: &str) -> String {
    csv_cell(s)
}

fn path_text(p: &Path) -> String {
    csv_cell(&mj_core::pathutil::display_lossy(p))
}

fn status_name(s: ItemStatus) -> &'static str {
    match s {
        ItemStatus::Done => "done",
        ItemStatus::Skipped => "skipped",
        ItemStatus::Failed => "failed",
    }
}

fn snake<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|x| x.as_str().map(str::to_string)).unwrap_or_default()
}

/// Report of one run as CSV.
pub fn report_csv(report: &Report, out: &mut dyn Write) -> Result<()> {
    out.write_all(BOM).map_err(Error::from)?;
    row(out, &["step", "path", "status", "bytes_before", "bytes_after", "saved", "error"].map(String::from))?;
    for s in &report.sections {
        for i in &s.items {
            row(
                out,
                &[
                    text(&s.title),
                    path_text(&i.path),
                    status_name(i.status).into(),
                    i.bytes_before.to_string(),
                    i.bytes_after.to_string(),
                    item_saved(i).to_string(),
                    text(&i.error.as_ref().map(|e| e.message.clone()).unwrap_or_default()),
                ],
            )?;
        }
    }
    Ok(())
}

fn item_saved(i: &ItemOutcome) -> u64 {
    if i.status == ItemStatus::Done {
        i.bytes_before.saturating_sub(i.bytes_after)
    } else {
        0
    }
}

const STYLE: &str = "body{font-family:Helvetica,Arial,sans-serif;font-size:12px;color:#1f2933;margin:24px}\
table{border-collapse:collapse;width:100%;margin-bottom:24px}th,td{border:1px solid #d9dee3;padding:4px 6px;\
text-align:left;vertical-align:top}th{background:#f3f5f7}td.n{text-align:right;font-variant-numeric:tabular-nums}\
h1{font-size:18px}h2{font-size:14px;margin-top:24px}.failed{color:#b91c1c}.skipped{color:#6b5310}";

fn html_head(out: &mut dyn Write, title: &str) -> Result<()> {
    w(
        out,
        &format!(
            "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{t}</title><style>{STYLE}</style></head><body>\n<h1>{t}</h1>\n",
            t = html_escape(title)
        ),
    )
}

/// Report of one run as HTML.
pub fn report_html(report: &Report, out: &mut dyn Write) -> Result<()> {
    html_head(out, &format!("MyJanitor report · {}", report.title))?;
    let t = &report.totals;
    w(
        out,
        &format!(
            "<p>Started {} · {} files · {} done · {} skipped · {} failed · {} saved</p>\n",
            html_escape(&report.started.to_rfc3339()),
            t.files,
            t.succeeded,
            t.skipped,
            t.failed,
            html_escape(&mj_core::size::format_bytes(t.bytes_saved))
        ),
    )?;
    for s in &report.sections {
        w(out, &format!("<h2>{} · {}</h2>\n", html_escape(&s.title), html_escape(&s.action)))?;
        w(
            out,
            "<table><tr><th>Path</th><th>Status</th><th>Before</th><th>After</th><th>Saved</th><th>Error</th></tr>\n",
        )?;
        for i in &s.items {
            let st = status_name(i.status);
            w(
                out,
                &format!(
                    "<tr class=\"{st}\"><td>{}</td><td>{st}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td>{}</td></tr>\n",
                    html_escape(&mj_core::pathutil::display_lossy(&i.path)),
                    i.bytes_before,
                    i.bytes_after,
                    item_saved(i),
                    html_escape(&i.error.as_ref().map(|e| format!("{} {}", e.message, e.hint)).unwrap_or_default()),
                ),
            )?;
        }
        w(out, "</table>\n")?;
    }
    w(out, "</body></html>\n")
}

/// Audit log of journal entries (file, action, size before/after, SHA-256, time, state).
pub fn audit_csv(entries: &[JournalEntry], out: &mut dyn Write) -> Result<()> {
    out.write_all(BOM).map_err(Error::from)?;
    row(
        out,
        &[
            "time",
            "run_id",
            "entry_id",
            "action",
            "state",
            "path",
            "retained_at",
            "bytes_before",
            "bytes_after",
            "sha256",
            "lossy",
            "error",
        ]
        .map(String::from),
    )?;
    for e in entries {
        let retained = e
            .retained_at
            .as_deref()
            .or(e.trash.as_ref().and_then(|t| t.location.as_deref()))
            .map(path_text)
            .unwrap_or_default();
        row(
            out,
            &[
                e.at.to_rfc3339(),
                e.run_id.0.to_string(),
                e.id.0.to_string(),
                snake(&e.action),
                snake(&e.state),
                path_text(&e.original),
                retained,
                e.bytes_before.to_string(),
                e.bytes_after.to_string(),
                e.original_sha256.clone().unwrap_or_default(),
                e.lossy.to_string(),
                text(&e.error.as_ref().map(|x| x.message.clone()).unwrap_or_default()),
            ],
        )?;
    }
    Ok(())
}

/// `det:<detector>`, `opt:<optimiser>` or `custom:<id>`.
pub fn opportunity_name(s: &Source) -> String {
    match s {
        Source::Detector(d) => format!("det:{}", snake(d)),
        Source::Optimiser(o) => format!("opt:{}", snake(o)),
        Source::Custom(c) => format!("custom:{c}"),
    }
}

/// Candidate list (results export) as CSV.
pub fn candidates_csv(candidates: &[Candidate], out: &mut dyn Write) -> Result<()> {
    out.write_all(BOM).map_err(Error::from)?;
    row(
        out,
        &["opportunity", "path", "size", "size_after", "saving", "reason", "badges", "selected", "group", "modified"]
            .map(String::from),
    )?;
    for c in candidates {
        row(
            out,
            &[
                text(&opportunity_name(&c.source)),
                path_text(&c.path),
                c.size.to_string(),
                c.size_after.to_string(),
                c.size.saturating_sub(c.size_after).to_string(),
                text(&c.reason),
                text(&c.badges.names().join(" ")),
                c.default_selected.to_string(),
                c.group.map(|g| g.0.to_string()).unwrap_or_default(),
                c.modified.map(Timestamp::to_rfc3339).unwrap_or_default(),
            ],
        )?;
    }
    Ok(())
}

/// Candidate list as HTML.
pub fn candidates_html(title: &str, candidates: &[Candidate], out: &mut dyn Write) -> Result<()> {
    html_head(out, title)?;
    w(out, "<table><tr><th>Opportunity</th><th>Path</th><th>Size</th><th>After</th><th>Saving</th><th>Reason</th><th>Badges</th><th>Selected</th><th>Group</th><th>Modified</th></tr>\n")?;
    for c in candidates {
        w(
            out,
            &format!(
                "<tr><td>{}</td><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                html_escape(&opportunity_name(&c.source)),
                html_escape(&mj_core::pathutil::display_lossy(&c.path)),
                c.size,
                c.size_after,
                c.size.saturating_sub(c.size_after),
                html_escape(&c.reason),
                html_escape(&c.badges.names().join(" ")),
                c.default_selected,
                c.group.map(|g| g.0.to_string()).unwrap_or_default(),
                html_escape(&c.modified.map(Timestamp::to_rfc3339).unwrap_or_default()),
            ),
        )?;
    }
    w(out, "</table>\n</body></html>\n")
}

/// Human label of an action kind for report sections.
pub fn action_label(k: ActionKind) -> &'static str {
    match k {
        ActionKind::Trash => "To Recycle Bin",
        ActionKind::Quarantine => "To quarantine",
        ActionKind::PermanentDelete => "Deleted permanently",
        ActionKind::Move => "Moved",
        ActionKind::Copy => "Copied",
        ActionKind::Hardlink => "Replaced with hard links",
        ActionKind::Reflink => "Replaced with reflinks",
        ActionKind::Dehydrate => "Made online-only",
        ActionKind::Optimise => "Optimised",
        ActionKind::Archive => "Archived",
        ActionKind::FsCompress => "Compressed (filesystem)",
        ActionKind::FsDecompress => "Decompressed (filesystem)",
        ActionKind::None => "No action",
    }
}

/// Build the report of a run from its journal entries (SAF-04). `step_titles` maps step ids
/// (in run order) to section titles; entries without a step go to one section per action.
pub fn from_journal(run: &RunRecord, entries: &[JournalEntry], step_titles: &[(u64, String)]) -> Report {
    let mut sections: Vec<(Option<u64>, ActionKind, ReportSection)> = Vec::new();
    let mut undo_until: Option<Timestamp> = None;
    for e in entries {
        let status = match e.state {
            JournalState::Done | JournalState::Undone | JournalState::UndoFailed | JournalState::Purged => {
                ItemStatus::Done
            }
            JournalState::Skipped => ItemStatus::Skipped,
            JournalState::Failed | JournalState::Intent => ItemStatus::Failed,
        };
        if e.state == JournalState::Done && e.reversible {
            if let Some(d) = e.undo_deadline {
                undo_until = Some(undo_until.map_or(d, |u| u.max(d)));
            }
        }
        let step = e.step_id.map(|s| s.0);
        let idx = match sections.iter().position(|(s, k, _)| *s == step && (step.is_some() || *k == e.action)) {
            Some(i) => i,
            None => {
                let title = step
                    .and_then(|s| step_titles.iter().find(|(id, _)| *id == s).map(|(_, t)| t.clone()))
                    .unwrap_or_else(|| action_label(e.action).to_string());
                sections.push((
                    step,
                    e.action,
                    ReportSection {
                        title,
                        action: action_label(e.action).into(),
                        totals: ReportTotals::default(),
                        items: Vec::new(),
                    },
                ));
                sections.len() - 1
            }
        };
        let item = ItemOutcome {
            path: e.original.clone(),
            status,
            bytes_before: e.bytes_before,
            bytes_after: if status == ItemStatus::Done { e.bytes_after } else { e.bytes_before },
            error: e.error.clone(),
        };
        let free = match e.action {
            ActionKind::Copy => 0,
            _ => item_saved(&item),
        };
        let t = &mut sections[idx].2.totals;
        t.files += 1;
        match status {
            ItemStatus::Done => t.succeeded += 1,
            ItemStatus::Skipped => t.skipped += 1,
            ItemStatus::Failed => t.failed += 1,
        }
        t.bytes_before += item.bytes_before;
        t.bytes_after += item.bytes_after;
        t.bytes_saved += free;
        sections[idx].2.items.push(item);
    }
    let mut totals = ReportTotals::default();
    for (_, _, s) in &sections {
        let t = s.totals;
        totals.files += t.files;
        totals.succeeded += t.succeeded;
        totals.skipped += t.skipped;
        totals.failed += t.failed;
        totals.bytes_before += t.bytes_before;
        totals.bytes_after += t.bytes_after;
        totals.bytes_saved += t.bytes_saved;
    }
    Report {
        run_id: run.run_id,
        title: run.title.clone(),
        started: run.started,
        finished: run.finished,
        totals,
        sections: sections.into_iter().map(|(_, _, s)| s).collect(),
        undo_until,
    }
}

/// Paths the OS refused access to (SAF-05: reported, never retried elevated).
pub fn permission_denied_paths(items: &[ItemOutcome]) -> Vec<PathBuf> {
    items
        .iter()
        .filter(|i| i.error.as_ref().is_some_and(|e| e.code == "permission_denied"))
        .map(|i| i.path.clone())
        .collect()
}

/// Create an export file at `dest`, never overwriting (`name (1).csv`...). Returns the
/// path written, its size, and what `write` returned (the row count).
pub fn write_export(dest: &Path, write: impl FnOnce(&mut dyn Write) -> Result<u64>) -> Result<(PathBuf, u64, u64)> {
    if !dest.is_absolute() {
        return Err(Error::InvalidInput(format!("not an absolute path: {}", dest.display())));
    }
    let mut target = crate::fsops::unique_path(dest);
    let mut file = None;
    for _ in 0..1000 {
        match OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(f) => {
                file = Some(f);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => target = crate::fsops::unique_path(dest),
            Err(e) => return Err(Error::io(&target, e)),
        }
    }
    let f = file.ok_or_else(|| Error::Conflict(format!("no free name for {}", dest.display())))?;
    let mut buf = std::io::BufWriter::new(f);
    let rows = match write(&mut buf) {
        Ok(r) => r,
        Err(e) => {
            drop(buf);
            let _ = std::fs::remove_file(&target);
            return Err(e);
        }
    };
    let f = buf.into_inner().map_err(|e| Error::io(&target, e.into_error()))?;
    f.sync_all().map_err(|e| Error::io(&target, e))?;
    let bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
    #[cfg(unix)]
    {
        // Lessons rule 8: a created file gets normal permissions, not a temp file's 0600.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644));
    }
    Ok((target, bytes, rows))
}
