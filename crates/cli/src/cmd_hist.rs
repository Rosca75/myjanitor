//! `history`, `undo`, `report`. Owner: W3-E.

use mj_core::{ExportFormat, Report, RunId};
use mj_engine::api::{PageRequest, ReportExportKind, ReportExportRequest, RunRequest};

use crate::args::Parsed;
use crate::cmd_plan::finish;
use crate::ctx::{Ctx, Fail, Res};
use crate::out::{bytes, json, table, when};

/// `history`.
pub fn history(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let limit = a.number("limit")?.unwrap_or(20) as u32;
    let h = ctx.engine.history_list(PageRequest { offset: 0, limit })?;
    if ctx.json {
        println!("{}", json(&h));
        return Ok(());
    }
    if h.runs.is_empty() {
        println!("No runs yet.");
        return Ok(());
    }
    let mut rows = vec![["RUN", "WHEN", "WHAT", "DONE", "FAILED", "SAVED", "UNDO"].map(String::from).to_vec()];
    for r in &h.runs {
        let undo = if r.undoable {
            r.undo_until.map(|t| format!("until {}", when(t))).unwrap_or_else(|| "yes".into())
        } else {
            "-".into()
        };
        rows.push(vec![
            r.run_id.0.to_string(),
            when(r.started),
            r.title.clone(),
            r.done.to_string(),
            r.failed.to_string(),
            bytes(r.bytes_saved),
            undo,
        ]);
    }
    print!("{}", table(&rows, &[0, 3, 4, 5]));
    println!("{} of {} runs shown.", h.runs.len(), h.total);
    Ok(())
}

/// `undo [--last | --run ID]`.
pub fn undo(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    if a.flag("last") && a.value("run").is_some() {
        return Err(Fail::Usage("use either --last or --run, not both".into()));
    }
    let job = match a.number("run")? {
        Some(id) => ctx.engine.undo_run(RunRequest { run_id: RunId(id) })?,
        None => ctx.engine.undo_last()?,
    };
    let jobs = ctx.wait_jobs(&[job.id])?;
    let ok: u64 = jobs.iter().filter_map(|j| j.outcome.as_ref()).map(|o| o.succeeded).sum();
    if !ctx.json {
        println!("Restored {ok} item(s).");
    }
    finish(&ctx, job.run_id.map(|r| r.0).unwrap_or(0), &jobs)
}

fn text_report(r: &Report) -> String {
    let mut s = format!(
        "{} (run {})\nStarted {}{}\n",
        r.title,
        r.run_id.0,
        when(r.started),
        r.finished.map(|f| format!(", finished {}", when(f))).unwrap_or_default()
    );
    let t = &r.totals;
    s.push_str(&format!(
        "{} files: {} done, {} skipped, {} failed. {} before, {} after, {} saved.\n",
        t.files,
        t.succeeded,
        t.skipped,
        t.failed,
        bytes(t.bytes_before),
        bytes(t.bytes_after),
        bytes(t.bytes_saved)
    ));
    if let Some(u) = r.undo_until {
        s.push_str(&format!("Can be undone until {}.\n", when(u)));
    }
    for sec in &r.sections {
        s.push_str(&format!(
            "\n{} ({}): {} done, {} skipped, {} failed, {} saved\n",
            sec.title,
            sec.action,
            sec.totals.succeeded,
            sec.totals.skipped,
            sec.totals.failed,
            bytes(sec.totals.bytes_saved)
        ));
        for e in sec.items.iter().filter(|i| i.error.is_some()).take(20) {
            s.push_str(&format!(
                "  failed: {}: {}\n",
                mj_core::pathutil::display_lossy(&e.path),
                e.error.as_ref().map(|x| x.message.as_str()).unwrap_or("")
            ));
        }
    }
    s
}

/// `report [--last | --run ID] [--format text|csv|html] [--out FILE]`.
pub fn report(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let format = a.value("format").unwrap_or(if ctx.json { "json" } else { "text" });
    let report = match a.number("run")? {
        Some(id) => ctx.engine.report_get(RunRequest { run_id: RunId(id) })?,
        None => ctx
            .engine
            .report_last()?
            .ok_or_else(|| Fail::Error("there is no report yet: nothing has been applied".into()))?,
    };
    match format {
        "text" => print!("{}", text_report(&report)),
        "json" => println!("{}", json(&report)),
        "csv" | "html" => {
            let out = a.value("out").ok_or_else(|| Fail::Usage(format!("--format {format} needs --out <FILE>")))?;
            let dest = std::path::absolute(out).map_err(|e| Fail::Error(format!("{out}: {e}")))?;
            let fmt = if format == "csv" { ExportFormat::Csv } else { ExportFormat::Html };
            let r = ctx.engine.report_export(ReportExportRequest {
                run_id: Some(report.run_id),
                kind: ReportExportKind::Report,
                format: fmt,
                dest,
            })?;
            println!("Wrote {} rows to {} ({}).", r.rows, r.path.display(), bytes(r.bytes));
        }
        other => return Err(Fail::Usage(format!("unknown report format `{other}`; use text, csv or html"))),
    }
    Ok(())
}
