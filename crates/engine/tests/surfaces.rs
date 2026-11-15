//! Events, paging, selection, plan editing, search, exports, history and disk surfaces.

mod common;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::{Action, Badges, OpportunityId};
use mj_engine::api::*;

fn oid(s: &str) -> OpportunityId {
    OpportunityId(s.into())
}

fn page_req(id: &str, offset: u64, limit: u32, sort: CandidateSortKey, descending: bool) -> CandidatesPageRequest {
    CandidatesPageRequest { opportunity_id: oid(id), sub_group: None, offset, limit, sort, descending, filter: None }
}

/// 12 groups of 3 identical files of different sizes.
fn dup_tree() -> TempDir {
    let t = TempDir::new("engine-dups").unwrap();
    for g in 0..12u64 {
        let data = bytes(100 + g, 1000 + (g as usize) * 5000);
        for c in 0..3 {
            t.write(format!("d{c}/file-{g:02}.bin"), &data).unwrap();
        }
    }
    t
}

#[test]
fn progress_is_throttled_and_stale_scans_publish_nothing() {
    let t = TempDir::new("engine-many").unwrap();
    for d in 0..60 {
        for f in 0..50 {
            t.write(format!("dir{d}/f{f}.txt"), format!("{d}-{f}").as_bytes()).unwrap();
        }
    }
    let f = engine_for(t);
    let e = &f.engine;
    let first = scan(e);
    assert_eq!(first.state, ScanRunState::Done);
    // Throttle: consecutive non-final progress events of one phase are >= 50 ms apart.
    let timed: Vec<_> = f.events.0.lock().unwrap().iter().cloned().collect();
    let progress: Vec<_> = timed
        .iter()
        .filter_map(|(t, ev)| match ev {
            Event::ScanProgress(p) => Some((*t, p.clone())),
            _ => None,
        })
        .collect();
    assert!(!progress.is_empty());
    for w in progress.windows(2) {
        let (t0, a) = &w[0];
        let (t1, b) = &w[1];
        let is_final = b.total.is_some_and(|t| b.done >= t);
        if a.phase == b.phase && !is_final {
            assert!(t1.duration_since(*t0).as_millis() >= 45, "progress not throttled: {a:?} {b:?}");
        }
    }
    let last = &progress.last().unwrap().1;
    assert_eq!(last.phase, Some(mj_core::Phase::Detecting));
    assert_eq!(Some(last.done), last.total, "the final value is always sent");
    let names: Vec<_> = f.events.events().iter().map(|e| e.name()).collect();
    assert!(names.contains(&"scan://detector") && names.contains(&"scan://done"));
    let before = e.opportunities_list(OpportunitiesRequest::default()).unwrap();

    // A second scan, paused at once and stopped: it publishes nothing.
    f.events.0.lock().unwrap().clear();
    let second = e.scan_start(ScanStartRequest::default()).unwrap();
    e.scan_pause().unwrap();
    assert_eq!(e.scan_start(ScanStartRequest::default()).unwrap_err().code(), "conflict");
    e.scan_stop().unwrap();
    let done = wait("stopped scan", || {
        let s = e.scan_status().unwrap();
        (!matches!(s.state, ScanRunState::Scanning | ScanRunState::Paused | ScanRunState::Stopping)).then_some(s)
    });
    assert_eq!(done.generation, second.generation);
    if done.state == ScanRunState::Cancelled {
        let after = e.opportunities_list(OpportunitiesRequest::default()).unwrap();
        assert_eq!(after.generation, before.generation, "stale scan must not publish");
        assert!(!f
            .events
            .events()
            .iter()
            .any(|ev| matches!(ev, Event::OpportunitiesChanged(c) if c.generation == second.generation)));
    }
    // The thread is gone: a new scan may start.
    wait("scan thread", || e.scan_start(ScanStartRequest::default()).ok());
    scan_wait_current(e);
}

fn scan_wait_current(e: &std::sync::Arc<mj_engine::Engine>) {
    wait("scan", || {
        let s = e.scan_status().unwrap();
        matches!(s.state, ScanRunState::Done | ScanRunState::Cancelled | ScanRunState::Failed).then_some(())
    });
}

#[test]
fn paging_sorting_and_selection() {
    let f = engine_for(dup_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let id = "det:exact_duplicates";
    let o = opp(e, id).unwrap();
    assert_eq!(o.candidates, 36);
    assert_eq!(o.selected, 24, "one keeper per group");
    let all = e.candidates_page(page_req(id, 0, 500, CandidateSortKey::Size, false)).unwrap();
    assert_eq!(all.total, 36);
    assert!(all.rows.windows(2).all(|w| w[0].candidate.size >= w[1].candidate.size), "size: biggest first");
    let rev = e.candidates_page(page_req(id, 0, 500, CandidateSortKey::Size, true)).unwrap();
    assert!(rev.rows.windows(2).all(|w| w[0].candidate.size <= w[1].candidate.size));
    let p2 = e.candidates_page(page_req(id, 10, 10, CandidateSortKey::Path, false)).unwrap();
    assert_eq!(p2.rows.len(), 10);
    assert_eq!(p2.offset, 10);
    let full = e.candidates_page(page_req(id, 0, 500, CandidateSortKey::Path, false)).unwrap();
    assert_eq!(p2.rows[0].candidate.id, full.rows[10].candidate.id);
    assert!(!p2.groups.is_empty() && p2.groups.len() <= 10);
    let grouped = e.candidates_page(page_req(id, 0, 500, CandidateSortKey::Group, false)).unwrap();
    assert!(grouped.rows[0].candidate.badges.contains(Badges::KEEPER), "keeper first in its group");
    let mut filtered = page_req(id, 0, 500, CandidateSortKey::Path, false);
    filtered.filter = Some("FILE-03".into());
    assert_eq!(e.candidates_page(filtered).unwrap().total, 3);
    assert!(e.candidates_page(page_req("det:nope", 0, 1, CandidateSortKey::Path, false)).is_err());

    // Uncheck everything, check two rows, select all, keep rule, reset.
    let s = e
        .selection_set(SelectionSetRequest { opportunity_id: oid(id), ids: None, sub_group: None, selected: false })
        .unwrap();
    assert_eq!(s.selected, 0);
    assert_eq!(s.saving, 0);
    let two: Vec<_> = full
        .rows
        .iter()
        .filter(|r| !r.candidate.badges.contains(Badges::KEEPER))
        .take(2)
        .map(|r| r.candidate.id)
        .collect();
    let s = e
        .selection_set(SelectionSetRequest { opportunity_id: oid(id), ids: Some(two), sub_group: None, selected: true })
        .unwrap();
    assert_eq!(s.selected, 2);
    assert!(f.events.events().iter().any(|ev| matches!(ev, Event::OpportunitiesChanged(_))));
    let bad = e.selection_set(SelectionSetRequest {
        opportunity_id: oid(id),
        ids: Some(vec![mj_core::CandidateId(u64::MAX >> 12)]),
        sub_group: None,
        selected: true,
    });
    assert_eq!(bad.unwrap_err().code(), "invalid_input");
    let s = e
        .selection_apply_keep_rule(SelectionKeepRuleRequest {
            opportunity_id: oid(id),
            rule: mj_core::KeepRule::ShortestPath,
        })
        .unwrap();
    assert_eq!(s.selected, 24);
    let rows = e.candidates_page(page_req(id, 0, 500, CandidateSortKey::Group, false)).unwrap();
    for g in &rows.groups {
        let keepers: Vec<_> = rows.rows.iter().filter(|r| r.candidate.group == Some(g.id) && !r.selected).collect();
        assert_eq!(keepers.len(), 1, "exactly one kept copy per group");
        assert_eq!(g.keeper.as_ref(), Some(&keepers[0].candidate.path));
    }
    let s = e
        .selection_set(SelectionSetRequest { opportunity_id: oid(id), ids: None, sub_group: None, selected: true })
        .unwrap();
    // F-02: "Select all" never selects the copy each group keeps (it used to: 36).
    assert_eq!(s.selected, 24, "select all leaves every group's kept copy unchecked");
    let s = e.selection_reset(OpportunityRequest { opportunity_id: oid(id) }).unwrap();
    assert_eq!(s.selected, 24);

    // The plan follows the selection.
    e.plan_add_step(PlanAddStepRequest { opportunity_id: oid(id), action: None, params: None }).unwrap();
    let before = e.plan_get().unwrap().estimate;
    e.selection_set(SelectionSetRequest { opportunity_id: oid(id), ids: None, sub_group: None, selected: false })
        .unwrap();
    let plan = e.plan_get().unwrap();
    assert_eq!(plan.estimate.files, 0);
    assert!(before.files > 0);
    assert!(plan.checklist.iter().any(|c| c.code == "empty_steps"));
}

#[test]
fn plan_editing_and_reorder() {
    let f = engine_for(standard_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let ids = ["det:temp_files", "det:dev_artefacts", "det:old_installers"];
    for id in ids {
        e.plan_add_step(PlanAddStepRequest { opportunity_id: oid(id), action: None, params: None }).unwrap();
    }
    let dup = e.plan_add_step(PlanAddStepRequest { opportunity_id: oid(ids[0]), action: None, params: None });
    assert_eq!(dup.unwrap_err().code(), "conflict");
    // `pdf_lossy` is disabled on every machine (no bundled image codec), unlike tool-gated optimisers
    // such as video, which become available when ffmpeg is installed.
    let disabled =
        e.plan_add_step(PlanAddStepRequest { opportunity_id: oid("opt:pdf_lossy"), action: None, params: None });
    assert_eq!(disabled.unwrap_err().code(), "unsupported");
    let bad_action = e.plan_add_step(PlanAddStepRequest {
        opportunity_id: oid("det:empty_folders"),
        action: Some(Action::Hardlink),
        params: None,
    });
    assert_eq!(bad_action.unwrap_err().code(), "invalid_input");
    let plan = e.plan_get().unwrap();
    let mut order: Vec<_> = plan.steps.iter().map(|s| s.id).collect();
    order.reverse();
    let plan = e.plan_reorder(PlanReorderRequest { step_ids: order.clone() }).unwrap();
    assert_eq!(plan.steps.iter().map(|s| s.id).collect::<Vec<_>>(), order);
    assert_eq!(plan.steps[0].opportunity, oid("det:old_installers"));
    let err = e.plan_reorder(PlanReorderRequest { step_ids: order[..2].to_vec() }).unwrap_err();
    assert_eq!(err.code(), "invalid_input");
    let err = e.plan_reorder(PlanReorderRequest { step_ids: vec![order[0], order[0], order[1]] }).unwrap_err();
    assert_eq!(err.code(), "invalid_input");
    // Update: quarantine instead of trash.
    let plan = e
        .plan_update_step(PlanUpdateStepRequest {
            step_id: order[0],
            action: Some(Action::Quarantine { purge_after_days: 7 }),
            params: None,
        })
        .unwrap();
    assert!(matches!(plan.steps[0].action, Action::Quarantine { purge_after_days: 7 }));
    let plan = e.plan_remove_step(StepRequest { step_id: order[1] }).unwrap();
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(e.plan_remove_step(StepRequest { step_id: order[1] }).unwrap_err().code(), "invalid_input");
    // Drives: the plan's saving is attributed to a volume.
    let drives = e.drives_list().unwrap();
    let total: u64 = drives.iter().map(|d| d.plan_saving).sum();
    assert!(total <= plan.estimate.saving());
    for d in &drives {
        assert_eq!(d.free_after_plan, d.volume.free + d.plan_saving);
    }
    assert!(e.plan_clear().unwrap().steps.is_empty());
    let names: Vec<_> = f.events.events().iter().map(|e| e.name()).collect();
    assert!(names.contains(&"plan://changed"));
}

#[test]
fn search_params_preview_and_disk() {
    let f = engine_for(standard_tree());
    let e = &f.engine;
    assert_eq!(
        e.search_run(SearchRunRequest {
            rule: mj_filters::parse("ext:tmp").unwrap(),
            title: None,
            topmost_only: false,
            action: None
        })
        .unwrap_err()
        .code(),
        "invalid_input",
        "no index before the first scan"
    );
    assert_eq!(scan(e).state, ScanRunState::Done);
    let parsed = e.search_parse(SearchParseRequest { query: "ext:tmp,msi".into() }).unwrap();
    assert!(parsed.errors.is_empty());
    let o = e
        .search_run(SearchRunRequest {
            rule: parsed.rule.unwrap(),
            title: Some("Leftovers".into()),
            topmost_only: false,
            action: None,
        })
        .unwrap();
    assert!(o.id.0.starts_with("custom:"));
    assert_eq!(o.candidates, 2);
    assert_eq!(o.selected, 2);
    // Same title replaces the card.
    let again = e
        .search_run(SearchRunRequest {
            rule: mj_filters::parse("ext:tmp").unwrap(),
            title: Some("Leftovers".into()),
            topmost_only: false,
            action: None,
        })
        .unwrap();
    assert_eq!(again.id, o.id);
    assert_eq!(again.candidates, 1);
    e.plan_add_step(PlanAddStepRequest { opportunity_id: o.id.clone(), action: None, params: None }).unwrap();
    // Media conditions without metadata are unknown: nothing matches, it is reported.
    let media: mj_core::Rule = serde_json::from_str(
        r#"{"cond":{"field":"media","cond":{"kind":"number","field":"width","cmp":"ge","value":1}}}"#,
    )
    .unwrap();
    let m = e
        .search_run(SearchRunRequest { rule: media, title: Some("Media".into()), topmost_only: false, action: None })
        .unwrap();
    assert_eq!(m.candidates, 0);
    let d = e.step_detail(OpportunityRequest { opportunity_id: m.id }).unwrap();
    assert!(d.diagnostics.skipped.get("media_without_metadata").copied().unwrap_or(0) > 0, "{:?}", d.diagnostics);

    // Parameters re-run the detector.
    let r = e
        .step_params_set(StepParamsRequest {
            opportunity_id: oid("det:old_installers"),
            params: mj_core::ParamValues::new().with("min_age_days", mj_core::ParamValue::Number(1000.0)),
            preset_id: None,
        })
        .unwrap();
    assert_eq!(r.summary.candidates, 0);
    assert!(!r.lossy);
    let err = e
        .step_params_set(StepParamsRequest {
            opportunity_id: oid("det:old_installers"),
            params: mj_core::ParamValues::new().with("nope", mj_core::ParamValue::Bool(true)),
            preset_id: None,
        })
        .unwrap_err();
    assert_eq!(err.code(), "invalid_input");

    // Preview: text and folders.
    let rows = e.candidates_page(page_req(&o.id.0, 0, 10, CandidateSortKey::Path, false)).unwrap().rows;
    let p = e.candidate_preview(CandidatePreviewRequest { candidate_id: rows[0].candidate.id }).unwrap();
    assert_eq!(p.kind, "none", "binary temp file has no preview");
    let dev = e.candidates_page(page_req("det:dev_artefacts", 0, 10, CandidateSortKey::Path, false)).unwrap().rows;
    assert_eq!(
        e.candidate_preview(CandidatePreviewRequest { candidate_id: dev[0].candidate.id }).unwrap().kind,
        "none"
    );

    // Disk usage and breakdowns.
    let b = e.breakdown(BreakdownRequest { by: BreakdownBy::Extension, path: None }).unwrap();
    assert!(b.rows.iter().any(|r| r.key == "bin"));
    let age = e.breakdown(BreakdownRequest { by: BreakdownBy::Age, path: None }).unwrap();
    assert!(age.rows.iter().any(|r| r.key == "1-2y"), "{:?}", age.rows);
    assert!(e.breakdown(BreakdownRequest { by: BreakdownBy::Type, path: None }).is_ok());
    let tree = e.disk_usage_tree(DiskUsageRequest { path: None, depth: 2, min_fraction: Some(0.0) }).unwrap();
    assert_eq!(tree.children.len(), 1);
}

#[test]
fn exports_history_restore_and_quarantine() {
    let f = engine_for(standard_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let out = TempDir::new("engine-exports").unwrap();
    let dest = out.join("results.csv");
    let r = e
        .results_export(ResultsExportRequest {
            opportunity_id: None,
            selected_only: true,
            format: mj_core::ExportFormat::Csv,
            dest: dest.clone(),
            overwrite: false,
        })
        .unwrap();
    assert_eq!(r.path, dest);
    assert!(r.rows >= 5);
    let text = std::fs::read_to_string(&dest).unwrap();
    assert!(text.starts_with('\u{feff}'));
    assert!(text.contains("report.tmp"));
    assert!(!text.contains("signed.pdf"), "locked rows are not selected");
    let r2 = e
        .results_export(ResultsExportRequest {
            opportunity_id: Some(oid("det:temp_files")),
            selected_only: false,
            format: mj_core::ExportFormat::Html,
            dest: dest.clone(),
            overwrite: false,
        })
        .unwrap();
    assert_ne!(r2.path, dest, "never overwrites");
    assert!(r2.path.to_string_lossy().contains("(1)"));
    assert_eq!(r2.rows, 1);
    // W6-A D-6A4: with `overwrite` (the user confirmed in the OS save dialog) the file at
    // `dest` is replaced, and only then. A request without the field keeps it.
    let before = std::fs::read(&dest).unwrap();
    let r3 = e
        .results_export(ResultsExportRequest {
            opportunity_id: Some(oid("det:temp_files")),
            selected_only: false,
            format: mj_core::ExportFormat::Csv,
            dest: dest.clone(),
            overwrite: true,
        })
        .unwrap();
    assert_eq!(r3.path, dest, "replaced in place");
    assert_eq!(r3.rows, 1);
    let after = std::fs::read(&dest).unwrap();
    assert_ne!(after, before);
    assert_eq!(after.len() as u64, r3.bytes);
    let json = serde_json::json!({ "format": "csv", "dest": dest });
    let parsed: ResultsExportRequest = serde_json::from_value(json).unwrap();
    assert!(!parsed.overwrite, "never overwriting is the default");
    let json = serde_json::json!({ "kind": "audit", "format": "csv", "dest": dest });
    assert!(!serde_json::from_value::<ReportExportRequest>(json).unwrap().overwrite);
    let r4 = e
        .report_export(ReportExportRequest {
            run_id: None,
            kind: ReportExportKind::Audit,
            format: mj_core::ExportFormat::Csv,
            dest: dest.clone(),
            overwrite: true,
        })
        .unwrap();
    assert_eq!(r4.path, dest, "report exports replace too");
    assert!(std::fs::read_to_string(&dest).unwrap().contains("entry_id"), "the audit header replaced the results");
    let leftovers: Vec<_> = std::fs::read_dir(dest.parent().unwrap())
        .unwrap()
        .map(|x| x.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".mjtmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    for id in ["det:temp_files", "det:old_installers"] {
        e.plan_add_step(PlanAddStepRequest {
            opportunity_id: oid(id),
            action: Some(Action::Quarantine { purge_after_days: 30 }),
            params: None,
        })
        .unwrap();
    }
    let started = e.plan_apply(PlanApplyRequest::default()).unwrap();
    wait_jobs(e);
    assert!(!exists(&f.tree.join("work/report.tmp")));
    let q = e.quarantine_info().unwrap();
    assert_eq!(q.items, 2);
    let report = e.report_get(RunRequest { run_id: started.run_id }).unwrap();
    assert_eq!(report.totals.succeeded, 2);
    for (kind, format, name) in [
        (ReportExportKind::Report, mj_core::ExportFormat::Csv, "report.csv"),
        (ReportExportKind::Report, mj_core::ExportFormat::Html, "report.html"),
        (ReportExportKind::Audit, mj_core::ExportFormat::Csv, "audit.csv"),
    ] {
        let r = e
            .report_export(ReportExportRequest {
                run_id: Some(started.run_id),
                kind,
                format,
                dest: out.join(name),
                overwrite: false,
            })
            .unwrap();
        assert_eq!(r.rows, 2, "{name}");
        assert!(r.bytes > 0);
    }
    let audit = std::fs::read_to_string(out.join("audit.csv")).unwrap();
    assert!(audit.contains("quarantine"));
    let all = e
        .report_export(ReportExportRequest {
            run_id: None,
            kind: ReportExportKind::Audit,
            format: mj_core::ExportFormat::Csv,
            dest: out.join("all.csv"),
            overwrite: false,
        })
        .unwrap();
    assert!(all.rows >= 2);
    assert!(e
        .report_export(ReportExportRequest {
            run_id: None,
            kind: ReportExportKind::Audit,
            format: mj_core::ExportFormat::Html,
            dest: out.join("x.html"),
            overwrite: false,
        })
        .is_err());

    // Restore one entry.
    let detail = e.history_run_detail(RunDetailRequest { run_id: started.run_id, offset: 0, limit: 100 }).unwrap();
    let entry = detail.entries.iter().find(|x| x.original.ends_with("report.tmp")).unwrap();
    let job = e.restore_entries(RestoreEntriesRequest { entry_ids: vec![entry.id] }).unwrap();
    let jobs = wait_jobs(e);
    assert_eq!(jobs.iter().find(|j| j.id == job.id).unwrap().state, mj_core::JobState::Completed);
    assert!(exists(&f.tree.join("work/report.tmp")));
    assert_eq!(e.quarantine_info().unwrap().items, 1);
    assert!(e.restore_entries(RestoreEntriesRequest { entry_ids: vec![entry.id] }).is_err(), "already restored");
    // Purge: the remaining quarantined item is deleted for good and no longer undoable.
    let job = e.quarantine_purge(QuarantinePurgeRequest { older_than_days: Some(0) }).unwrap();
    let jobs = wait_jobs(e);
    assert_eq!(jobs.iter().find(|j| j.id == job.id).unwrap().state, mj_core::JobState::Completed);
    assert_eq!(e.quarantine_info().unwrap().items, 0);
    assert!(e.undo_run(RunRequest { run_id: started.run_id }).is_err());
    assert!(!exists(&f.tree.join("downloads/setup-1.0.msi")));
}

#[test]
fn queued_jobs_can_be_reordered_and_cancelled() {
    let f = engine_for(standard_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    for id in ["det:temp_files", "det:old_installers", "det:dev_artefacts"] {
        e.plan_add_step(PlanAddStepRequest { opportunity_id: oid(id), action: None, params: None }).unwrap();
    }
    let started = e.plan_apply(PlanApplyRequest::default()).unwrap();
    assert_eq!(e.plan_apply(PlanApplyRequest::default()).map(|_| ()).unwrap_err().code(), "conflict");
    let ids: Vec<_> = started.jobs.iter().map(|j| j.id).collect();
    // Cancel the last queued job; it may already have run on a fast machine.
    let c = e.job_cancel(JobRequest { job_id: ids[2] }).unwrap();
    let jobs = wait_jobs(e);
    let last = jobs.iter().find(|j| j.id == ids[2]).unwrap();
    if c.state == mj_core::JobState::Cancelled {
        assert_eq!(last.state, mj_core::JobState::Cancelled);
        assert!(last.started.is_none());
        assert!(exists(&f.tree.join("proj/node_modules")), "a cancelled queued job never runs");
    }
    assert!(e.jobs_reorder(JobsReorderRequest { job_ids: vec![ids[0]] }).is_err(), "finished jobs are not queued");
    assert!(e.job_pause(JobRequest { job_id: mj_core::JobId(u64::MAX >> 12) }).is_err());
    let names: Vec<_> = f.events.events().iter().map(|e| e.name()).collect();
    assert!(names.contains(&"job://state") && names.contains(&"job://done"));
}
