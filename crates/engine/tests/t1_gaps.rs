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
