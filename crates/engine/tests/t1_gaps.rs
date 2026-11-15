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
