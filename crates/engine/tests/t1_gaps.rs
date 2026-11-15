//! Engine tests for the T1 gaps closed in wave 5 (CLN-ACT-04, -05, -02, -06, -11, AUT-05..07,
//! CLN-DET-08, CLN-FLT-06). Every test runs on its own `TempDir` trees.

mod common;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::OpportunityId;
use mj_engine::api::*;

fn oid(s: &str) -> OpportunityId {
    OpportunityId(s.into())
}

/// CLN-ACT-04: an item quarantined with a retention that has already passed is purged
/// automatically once the run completes, and at the next start-up for older items.
#[test]
fn cln_act_04_expired_quarantine_items_are_purged_after_the_run() {
    let t = TempDir::new("t1-purge").unwrap();
    let data = bytes(3, 40_000);
    t.write("a/x.bin", &data).unwrap();
    t.write("b/x.bin", &data).unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    e.plan_add_step(PlanAddStepRequest {
        opportunity_id: oid("det:exact_duplicates"),
        action: Some(mj_core::Action::Quarantine { purge_after_days: 0 }),
        params: None,
    })
    .unwrap();
    e.plan_apply(PlanApplyRequest::default()).unwrap();
    wait_jobs(e);
    assert_eq!(e.quarantine_info().unwrap().items, 0, "expired at once, purged after the run");
    let left = ["a/x.bin", "b/x.bin"].iter().filter(|p| exists(&f.tree.join(p))).count();
    assert_eq!(left, 1, "the kept copy stays");
}

/// CLN-ACT-02: `app_info` names the route removals really take; the Windows Recycle Bin
/// setting never changes a route fixed by the caller (the tests' own quarantine), and on
/// Linux it is ignored.
#[test]
fn cln_act_02_app_info_names_the_real_trash_route() {
    let f = engine_for(TempDir::new("t1-route").unwrap());
    let e = &f.engine;
    assert_eq!(e.app_info().unwrap().trash_route, "quarantine");
    let mut s = e.settings_get().unwrap();
    s.general.windows_recycle_bin = true;
    e.settings_set(SettingsSetRequest { settings: s }).unwrap();
    assert_eq!(e.app_info().unwrap().trash_route, "quarantine", "a fixed route is kept");
}

/// AUT-07: the temp folder may not lie inside a scanned location.
#[test]
fn aut_07_temp_dir_inside_a_scanned_location_is_refused() {
    let tree = TempDir::new("t1-temp").unwrap();
    let inside = tree.mkdir("work/tmp").unwrap();
    let outside = TempDir::new("t1-temp-out").unwrap();
    let f = engine_for(tree);
    let e = &f.engine;
    let mut s = e.settings_get().unwrap();
    s.performance.temp_dir = Some(inside);
    let err = e.settings_set(SettingsSetRequest { settings: s.clone() }).unwrap_err();
    assert!(err.to_string().contains("inside a scanned location"), "{err}");
    s.performance.temp_dir = Some(outside.path().to_path_buf());
    e.settings_set(SettingsSetRequest { settings: s }).unwrap();
    assert_eq!(e.temp_dir(), outside.path());
}

/// CLN-ACT-05: Copy keeps the folder structure below the scan root by default, leaves the
/// originals in place, and a destination inside the scanned location is refused at once.
#[test]
fn cln_act_05_copy_keeps_the_structure_and_checks_the_destination() {
    let t = TempDir::new("t1-copy").unwrap();
    let data = bytes(5, 30_000);
    t.write("a/x.bin", &data).unwrap();
    t.write("b/deep/x.bin", &data).unwrap();
    let out = TempDir::new("t1-copy-out").unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let id = oid("det:exact_duplicates");
    let inside = f.tree.join("a");
    let err = e
        .plan_add_step(PlanAddStepRequest {
            opportunity_id: id.clone(),
            action: Some(mj_core::Action::Copy { dest: inside, keep_structure: true }),
            params: None,
        })
        .unwrap_err();
    assert!(err.to_string().contains("inside the scanned location"), "{err}");
    e.plan_add_step(PlanAddStepRequest {
        opportunity_id: id,
        action: Some(mj_core::Action::Copy { dest: out.path().to_path_buf(), keep_structure: true }),
        params: None,
    })
    .unwrap();
    e.plan_apply(PlanApplyRequest::default()).unwrap();
    wait_jobs(e);
    assert!(exists(&f.tree.join("a/x.bin")) && exists(&f.tree.join("b/deep/x.bin")), "originals stay");
    let copied = ["a/x.bin", "b/deep/x.bin"].iter().filter(|p| exists(&out.join(p))).count();
    assert_eq!(copied, 1, "the selected copy landed under its relative path");
}

/// CLN-ACT-11: results and run reports export as a valid PDF the in-house reader parses.
#[test]
fn cln_act_11_results_and_reports_export_as_pdf() {
    let t = TempDir::new("t1-pdf").unwrap();
    let data = bytes(8, 20_000);
    t.write("a/x (1).bin", &data).unwrap();
    t.write("b/x.bin", &data).unwrap();
    let out = TempDir::new("t1-pdf-out").unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let r = e
        .results_export(ResultsExportRequest {
            opportunity_id: Some(oid("det:exact_duplicates")),
            selected_only: false,
            format: mj_core::ExportFormat::Pdf,
            dest: out.join("results.pdf"),
        })
        .unwrap();
    assert_eq!(r.rows, 2);
    let bytes = std::fs::read(&r.path).unwrap();
    let doc = mj_formats::pdf::document::Document::load(&bytes).expect("valid PDF");
    assert_eq!(doc.pages().unwrap().len(), 1);
    assert!(mj_formats::pdf::document::contains(&bytes, b"x \\(1\\).bin"));
    e.plan_add_step(PlanAddStepRequest {
        opportunity_id: oid("det:exact_duplicates"),
        action: Some(mj_core::Action::Trash),
        params: None,
    })
    .unwrap();
    let started = e.plan_apply(PlanApplyRequest::default()).unwrap();
    wait_jobs(e);
    let rep = e
        .report_export(ReportExportRequest {
            run_id: Some(started.run_id),
            kind: ReportExportKind::Report,
            format: mj_core::ExportFormat::Pdf,
            dest: out.join("report.pdf"),
        })
        .unwrap();
    let bytes = std::fs::read(&rep.path).unwrap();
    assert!(mj_formats::pdf::document::Document::load(&bytes).is_ok());
    assert!(mj_formats::pdf::document::contains(&bytes, b"MyJanitor report"));
}
