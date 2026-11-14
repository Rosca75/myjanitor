//! Seams between workstreams found by the integration pass (W4-INT): settings that feed
//! detectors, the restore pre-check, platform capabilities and per-step warnings.

mod common;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::{JobState, OpportunityId};
use mj_engine::api::*;

fn paths(e: &mj_engine::Engine, id: &str) -> Vec<std::path::PathBuf> {
    e.candidates_page(CandidatesPageRequest {
        opportunity_id: OpportunityId(id.into()),
        sub_group: None,
        offset: 0,
        limit: 500,
        sort: CandidateSortKey::Path,
        descending: false,
        filter: None,
    })
    .map(|p| p.rows.into_iter().map(|r| r.candidate.path).collect())
    .unwrap_or_default()
}

#[test]
fn kept_languages_setting_reaches_the_language_pack_detector() {
    let t = TempDir::new("engine-langs").unwrap();
    for code in ["en-US", "fr", "de", "ja"] {
        t.write(format!("app/locales/{code}.pak"), &bytes(code.len() as u64, 4096)).unwrap();
    }
    let f = engine_for(t);
    let e = &f.engine;
    let mut s = e.settings_get().unwrap();
    s.general.kept_languages = vec![" FR ".into(), "de".into()];
    let saved = e.settings_set(SettingsSetRequest { settings: s }).unwrap();
    assert_eq!(saved.general.kept_languages, vec!["fr".to_string(), "de".to_string()], "normalised");
    assert_eq!(scan(e).state, ScanRunState::Done);
    let found = paths(e, "det:language_packs");
    let names: Vec<String> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["ja.pak".to_string()], "fr, de (setting) and en (always) are kept");
    // A bad code is refused with the field named, and nothing is stored.
    let mut bad = e.settings_get().unwrap();
    bad.general.kept_languages = vec!["../x".into()];
    let err = e.settings_set(SettingsSetRequest { settings: bad }).unwrap_err();
    assert!(err.to_string().contains("kept_languages"), "{err}");
    assert_eq!(e.settings_get().unwrap().general.kept_languages, vec!["fr".to_string(), "de".to_string()]);
}

#[test]
fn restore_precheck_reports_an_occupied_original_location() {
    let t = TempDir::new("engine-occupied").unwrap();
    t.write("work/a.tmp", &bytes(1, 3000)).unwrap();
    t.write("work/b.tmp", &bytes(2, 3000)).unwrap();
    for p in ["work/a.tmp", "work/b.tmp"] {
        mj_core::testutil::set_mtime(
            &t.join(p),
            std::time::SystemTime::now() - std::time::Duration::from_secs(86_400 * 3),
        )
        .unwrap();
    }
    let f = engine_for(t);
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    e.plan_add_step(PlanAddStepRequest {
        opportunity_id: OpportunityId("det:temp_files".into()),
        action: None,
        params: None,
    })
    .unwrap();
    let started = e.plan_apply(PlanApplyRequest::default()).unwrap();
    for j in wait_jobs(e) {
        assert_eq!(j.state, JobState::Completed, "{j:?}");
    }
    let detail = e.history_run_detail(RunDetailRequest { run_id: started.run_id, offset: 0, limit: 100 }).unwrap();
    assert!(detail.occupied.is_empty());
    // The user puts something new where a.tmp was.
    std::fs::write(f.tree.join("work/a.tmp"), b"new content").unwrap();
    let detail = e.history_run_detail(RunDetailRequest { run_id: started.run_id, offset: 0, limit: 100 }).unwrap();
    let a = detail.entries.iter().find(|x| x.original == f.tree.join("work/a.tmp")).unwrap();
    assert_eq!(detail.occupied, vec![a.id]);
    // The pre-check agrees with what a restore does: refused, the new file is untouched.
    let job = e.restore_entries(RestoreEntriesRequest { entry_ids: vec![a.id] }).unwrap();
    let jobs = wait_jobs(e);
    let out = jobs.iter().find(|j| j.id == job.id).unwrap().outcome.clone().unwrap();
    assert_eq!(out.succeeded, 0, "{out:?}");
    assert_eq!(std::fs::read(f.tree.join("work/a.tmp")).unwrap(), b"new content");
}

#[test]
fn app_info_lists_platform_capabilities() {
    let app = TempDir::new("engine-caps").unwrap();
    let e = mj_engine::Engine::new(config(&app, Default::default())).unwrap();
    let caps = e.app_info().unwrap().capabilities;
    assert!(caps.contains(&"hardlinks".to_string()));
    if cfg!(windows) {
        assert!(caps.contains(&"cloud_dehydrate".to_string()) && caps.contains(&"wof_compression".to_string()));
    } else {
        assert!(caps.contains(&"btrfs_compression".to_string()) && !caps.contains(&"cloud_dehydrate".to_string()));
    }
}
