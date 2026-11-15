//! Regression tests for the review findings fixed in W6-A (third task): entry ids on the
//! wire, selection kept across parameter changes, "select all" under a filter, the lossy
//! flag of lossy-by-nature steps, the confirmation figures, keep-rule badges after a reset
//! and the undo window of quarantined items.

mod common;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::{JobState, OpportunityId};
use mj_engine::api::*;
use serde_json::Value;

/// What a webview does to a JSON response: every number becomes an IEEE double.
fn through_js(v: Value) -> Value {
    match v {
        Value::Number(n) => serde_json::Number::from_f64(n.as_f64().unwrap()).map(Value::Number).unwrap_or(Value::Null),
        Value::Array(a) => Value::Array(a.into_iter().map(through_js).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, through_js(v))).collect()),
        other => other,
    }
}

fn old_tmp_tree(n: usize) -> TempDir {
    let t = TempDir::new("w6a-ids").unwrap();
    for i in 0..n {
        let p = format!("work/f{i:02}.tmp");
        t.write(&p, &bytes(i as u64, 2000)).unwrap();
        mj_core::testutil::set_mtime(
            &t.join(&p),
            std::time::SystemTime::now() - std::time::Duration::from_secs(86_400 * 3),
        )
        .unwrap();
    }
    t
}

/// IPC-1 / ENG-1 / ACT-2: History's per-entry Restore restores exactly the clicked entry
/// after the ids went through a JavaScript number round trip.
#[test]
fn restore_of_one_entry_survives_the_webview_number_round_trip() {
    let f = engine_for(old_tmp_tree(6));
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
    assert_eq!(detail.entries.len(), 6);
    let seen = through_js(serde_json::to_value(&detail).unwrap());
    let ids: Vec<&Value> = seen["entries"].as_array().unwrap().iter().map(|x| &x["id"]).collect();
    let distinct: std::collections::BTreeSet<String> = ids.iter().map(|v| v.to_string()).collect();
    assert_eq!(distinct.len(), 6, "every entry keeps its own id in the webview: {ids:?}");
    // The UI sends back the id it holds for the 6th row.
    let pick = 5;
    let req: RestoreEntriesRequest =
        serde_json::from_value(serde_json::json!({ "entry_ids": [ids[pick].clone()] })).unwrap();
    let target = detail.entries[pick].original.clone();
    assert!(!exists(&target));
    e.restore_entries(req).unwrap();
    wait_jobs(e);
    assert!(exists(&target), "the clicked entry is restored");
    for (i, x) in detail.entries.iter().enumerate() {
        if i != pick {
            assert!(!exists(&x.original), "entry {i} was not asked for");
        }
    }
}
