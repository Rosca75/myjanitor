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

/// CLN-ACT-04: an item quarantined with a retention that has passed is purged
/// automatically, at the next start-up and after each run, each item by its own step's
/// retention. ACT-9: a retention of 0 days is refused (it would be an unconfirmed permanent
/// delete), so the retention is made to pass by back-dating the journal record.
#[test]
fn cln_act_04_expired_quarantine_items_are_purged_after_the_run() {
    let t = TempDir::new("t1-purge").unwrap();
    let data = bytes(3, 40_000);
    t.write("a/x.bin", &data).unwrap();
    t.write("b/x.bin", &data).unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let step = |days| PlanAddStepRequest {
        opportunity_id: oid("det:exact_duplicates"),
        action: Some(mj_core::Action::Quarantine { purge_after_days: days }),
        params: None,
    };
    let err = e.plan_add_step(step(0)).unwrap_err();
    assert_eq!(err.info().code, "invalid_input", "{err}");
    e.plan_add_step(step(1)).unwrap();
    e.plan_apply(PlanApplyRequest::default()).unwrap();
    wait_jobs(e);
    assert_eq!(e.quarantine_info().unwrap().items, 1, "held for its retention, not purged at once");
    let left = ["a/x.bin", "b/x.bin"].iter().filter(|p| exists(&f.tree.join(p))).count();
    assert_eq!(left, 1, "the kept copy stays");

    // Two days later (back-dated record), the next start-up purges it.
    let Fixture { tree, app, engine, .. } = f;
    drop(engine);
    let j = mj_actions::Journal::open(&app.join("journal")).unwrap();
    for mut r in j.all_entries() {
        if r.action == mj_core::ActionKind::Quarantine && r.state == mj_core::JournalState::Done {
            r.at = r.at.add_days(-2);
            j.append(&r).unwrap();
        }
    }
    drop(j);
    let e2 = mj_engine::Engine::new(config(&app, Default::default())).unwrap();
    assert_eq!(e2.quarantine_info().unwrap().items, 0, "expired, purged at start-up");
    let left = ["a/x.bin", "b/x.bin"].iter().filter(|p| exists(&tree.join(p))).count();
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
            overwrite: false,
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
            overwrite: false,
        })
        .unwrap();
    let bytes = std::fs::read(&rep.path).unwrap();
    assert!(mj_formats::pdf::document::Document::load(&bytes).is_ok());
    assert!(mj_formats::pdf::document::contains(&bytes, b"MyJanitor report"));
}

fn ooxml(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut w = mj_formats::zip::ZipWriter::new(Vec::new());
    let ct = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#;
    w.add("[Content_Types].xml", ct.as_bytes(), 6, 0x6000, 0x5821).unwrap();
    for (n, d) in parts {
        w.add(n, d.as_bytes(), 6, 0x6000, 0x5821).unwrap();
    }
    w.finish().unwrap()
}

/// CLN-CNT-01: content search reads PDF, Word, Excel and PowerPoint text.
#[test]
fn cln_cnt_01_content_search_reads_documents() {
    let t = TempDir::new("t1-content").unwrap();
    t.write(
        "a.docx",
        &ooxml(&[(
            "word/document.xml",
            "<w:document><w:body><w:p><w:r><w:t>the zebra contract</w:t></w:r></w:p></w:body></w:document>",
        )]),
    )
    .unwrap();
    t.write("b.xlsx", &ooxml(&[("xl/sharedStrings.xml", "<sst><si><t>zebra budget</t></si></sst>")])).unwrap();
    t.write(
        "c.pptx",
        &ooxml(&[("ppt/slides/slide1.xml", "<p:sld><a:p><a:r><a:t>zebra slide</a:t></a:r></a:p></p:sld>")]),
    )
    .unwrap();
    let pdf = mj_formats::pdf::table::table_pdf("zebra report", "", &["x"], &[1.0], &[]);
    t.write("d.pdf", &pdf.bytes).unwrap();
    t.write("e.docx", &ooxml(&[("word/document.xml", "<w:document><w:t>nothing here</w:t></w:document>")])).unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let parsed = e.search_parse(SearchParseRequest { query: "content:zebra".into() }).unwrap();
    let opp = e
        .search_run(SearchRunRequest { rule: parsed.rule.unwrap(), title: None, topmost_only: false, action: None })
        .unwrap();
    assert_eq!(opp.count, 4, "{opp:?}");
}
