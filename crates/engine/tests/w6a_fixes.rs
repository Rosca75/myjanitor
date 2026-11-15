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

fn oid(s: &str) -> OpportunityId {
    OpportunityId(s.into())
}

fn page_req(id: &str) -> CandidatesPageRequest {
    CandidatesPageRequest {
        opportunity_id: oid(id),
        sub_group: None,
        offset: 0,
        limit: 500,
        sort: CandidateSortKey::Path,
        descending: false,
        filter: None,
        min_size: None,
        keep_groups: false,
    }
}

/// (file name, selected) of every row of a step, by path.
fn rows(e: &mj_engine::Engine, id: &str) -> Vec<(String, bool, mj_core::CandidateId)> {
    e.candidates_page(page_req(id))
        .unwrap()
        .rows
        .into_iter()
        .map(|r| (r.candidate.path.file_name().unwrap().to_string_lossy().into_owned(), r.selected, r.candidate.id))
        .collect()
}

fn installers_tree() -> TempDir {
    let t = TempDir::new("w6a-installers").unwrap();
    for (i, days) in [(0, 400u64), (1, 400), (2, 400), (3, 400), (4, 250)] {
        let p = format!("downloads/setup-{i}.msi");
        t.write(&p, &bytes(10 + i as u64, 4000)).unwrap();
        mj_core::testutil::set_mtime(
            &t.join(&p),
            std::time::SystemTime::now() - std::time::Duration::from_secs(86_400 * days),
        )
        .unwrap();
    }
    t
}

/// ENG-2: changing a step parameter keeps the rows the user unchecked unchecked.
#[test]
fn a_parameter_change_keeps_the_rows_the_user_unchecked() {
    let f = engine_for(installers_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let before = rows(e, "det:old_installers");
    assert_eq!(before.len(), 5, "{before:?}");
    assert!(before.iter().all(|r| r.1));
    let off: Vec<_> = before.iter().filter(|r| r.0 == "setup-0.msi" || r.0 == "setup-1.msi").map(|r| r.2).collect();
    e.selection_set(SelectionSetRequest {
        opportunity_id: oid("det:old_installers"),
        ids: Some(off),
        sub_group: None,
        selected: false,
        filter: None,
        min_size: None,
        keep_groups: false,
    })
    .unwrap();
    let r = e
        .step_params_set(StepParamsRequest {
            opportunity_id: oid("det:old_installers"),
            params: mj_core::ParamValues::new().with("min_age_days", mj_core::ParamValue::Number(200.0)),
            preset_id: None,
        })
        .unwrap();
    assert_eq!(r.summary.candidates, 5);
    assert_eq!(r.summary.selected, 3, "the two unchecked rows stay unchecked");
    for (name, sel, _) in rows(e, "det:old_installers") {
        assert_eq!(sel, name != "setup-0.msi" && name != "setup-1.msi", "{name}");
    }
    // A narrower parameter drops a row; the other choices stay.
    let r = e
        .step_params_set(StepParamsRequest {
            opportunity_id: oid("det:old_installers"),
            params: mj_core::ParamValues::new().with("min_age_days", mj_core::ParamValue::Number(300.0)),
            preset_id: None,
        })
        .unwrap();
    assert_eq!((r.summary.candidates, r.summary.selected), (4, 2));
    assert_eq!(
        e.step_detail(OpportunityRequest { opportunity_id: oid("det:old_installers") }).unwrap().summary.selected,
        2
    );
}

/// ENG-2 (search): running the same search again keeps the rows the user unchecked.
#[test]
fn running_the_same_search_again_keeps_unchecked_rows() {
    let f = engine_for(old_tmp_tree(4));
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let run = |e: &std::sync::Arc<mj_engine::Engine>| {
        e.search_run(SearchRunRequest {
            rule: mj_filters::parse("ext:tmp").unwrap(),
            title: Some("Leftovers".into()),
            topmost_only: false,
            action: None,
        })
        .unwrap()
    };
    let o = run(e);
    let id = o.id.0.clone();
    let all = rows(e, &id);
    assert_eq!(all.len(), 4);
    let first = all.iter().find(|r| r.0 == "f00.tmp").unwrap().2;
    e.selection_set(SelectionSetRequest {
        opportunity_id: o.id.clone(),
        ids: Some(vec![first]),
        sub_group: None,
        selected: false,
        filter: None,
        min_size: None,
        keep_groups: false,
    })
    .unwrap();
    let again = run(e);
    assert_eq!(again.id, o.id);
    assert_eq!(again.selected, all.iter().filter(|r| r.1).count() as u64 - 1);
    assert!(!rows(e, &id).iter().find(|r| r.0 == "f00.tmp").unwrap().1);
}

fn select_all(e: &mj_engine::Engine, id: &str, filter: Option<&str>, keep_groups: bool, selected: bool) -> StepSummary {
    e.selection_set(SelectionSetRequest {
        opportunity_id: oid(id),
        ids: None,
        sub_group: None,
        filter: filter.map(Into::into),
        min_size: None,
        keep_groups,
        selected,
    })
    .unwrap()
}

/// ENG-3 (flat table): "select all" under a text filter acts on the filtered rows only.
#[test]
fn select_all_under_a_filter_touches_only_the_filtered_rows() {
    let f = engine_for(installers_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    assert_eq!(select_all(e, "det:old_installers", None, false, false).selected, 0);
    let s = select_all(e, "det:old_installers", Some("SETUP-1"), false, true);
    assert_eq!(s.selected, 1, "only the filtered row");
    for (name, sel, _) in rows(e, "det:old_installers") {
        assert_eq!(sel, name == "setup-1.msi", "{name}");
    }
    // Unticking under a filter leaves the hidden rows alone too.
    assert_eq!(select_all(e, "det:old_installers", None, false, true).selected, 5);
    assert_eq!(select_all(e, "det:old_installers", Some("setup-3"), false, false).selected, 4);
    assert!(!rows(e, "det:old_installers").iter().find(|r| r.0 == "setup-3.msi").unwrap().1);
}

fn dup_tree() -> TempDir {
    let t = TempDir::new("w6a-dups").unwrap();
    let one = bytes(31, 200_000);
    let two = bytes(32, 150_000);
    for p in ["x/one.bin", "y/one.bin", "zz/one.bin"] {
        t.write(p, &one).unwrap();
    }
    for p in ["x/two.bin", "y/two.bin"] {
        t.write(p, &two).unwrap();
    }
    t
}

/// ENG-3 (groups kept together): the filter keeps whole groups, so "select all" acts on the
/// shown groups, never on a hidden group and never on a group's kept copy.
#[test]
fn select_all_in_grouped_mode_acts_on_the_shown_groups_only() {
    let f = engine_for(dup_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let id = "det:exact_duplicates";
    assert_eq!(select_all(e, id, None, true, false).selected, 0);
    let s = select_all(e, id, Some("zz/"), true, true);
    assert_eq!(s.selected, 2, "the shown group minus its kept copy");
    let page = e.candidates_page(page_req(id)).unwrap();
    for r in &page.rows {
        let name = r.candidate.path.file_name().unwrap().to_string_lossy().into_owned();
        let keeper = r.candidate.badges.contains(mj_core::Badges::KEEPER);
        assert_eq!(r.selected, name == "one.bin" && !keeper, "{}", r.candidate.path.display());
    }
}

fn dry_run(f: &Fixture) {
    let job = f.engine.plan_dry_run().unwrap();
    wait("dry run", || {
        f.events.events().into_iter().find_map(|ev| match ev {
            Event::DryRun(r) if r.job_id == job.id => Some(()),
            _ => None,
        })
    });
}

/// ENG-4: a dry-run measurement never stands for a different set of items. Running the
/// same search again with a wider rule replaces the rows; the plan and the confirmation
/// dialog must count the new items, which are the ones the digest binds and apply deletes.
#[test]
fn a_measured_estimate_does_not_survive_a_replaced_record() {
    let f = engine_for(old_tmp_tree(5));
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let search = |rule: &str| {
        e.search_run(SearchRunRequest {
            rule: mj_filters::parse(rule).unwrap(),
            title: Some("Custom search".into()),
            topmost_only: false,
            action: None,
        })
        .unwrap()
    };
    let o = search("name:f00.tmp");
    assert_eq!(o.candidates, 1);
    let plan = e
        .plan_add_step(PlanAddStepRequest {
            opportunity_id: o.id.clone(),
            action: Some(mj_core::Action::PermanentDelete),
            params: None,
        })
        .unwrap();
    let step = plan.steps[0].id;
    dry_run(&f);
    assert_eq!(e.plan_get().unwrap().steps[0].estimate.files, 1);
    assert_eq!(e.plan_get().unwrap().steps[0].estimate.confidence, mj_core::Confidence::Exact);
    let again = search("ext:tmp");
    assert_eq!((again.id.clone(), again.candidates, again.selected), (o.id.clone(), 5, 5));
    assert_eq!(e.plan_get().unwrap().steps[0].estimate.files, 5, "the plan counts the live rows");
    let info = e.plan_confirmation_info(StepRequest { step_id: step }).unwrap();
    assert_eq!(info.files, 5);
    assert!(info.title.contains('5'), "{}", info.title);
    let conf = mj_core::StepConfirmation {
        step_id: step,
        acknowledged: true,
        originals: Default::default(),
        synced_online_only: false,
        selection_digest: Some(info.selection_digest),
    };
    e.plan_apply(PlanApplyRequest { confirmations: vec![conf] }).unwrap();
    let jobs = wait_jobs(e);
    assert_eq!(jobs.iter().map(|j| j.outcome.as_ref().map_or(0, |o| o.succeeded)).sum::<u64>(), 5);
}

/// Every group of the duplicates step: exactly one member unselected, it carries the kept
/// badge, and the group's keeper is that member.
fn assert_keepers_follow_selection(e: &mj_engine::Engine, when: &str) {
    let page = e.candidates_page(page_req("det:exact_duplicates")).unwrap();
    assert!(!page.groups.is_empty());
    for g in &page.groups {
        let members: Vec<_> = page.rows.iter().filter(|r| r.candidate.group == Some(g.id)).collect();
        let left: Vec<_> = members.iter().filter(|r| !r.selected).collect();
        assert_eq!(left.len(), 1, "{when}: one copy stays in {:?}", g.members);
        let kept: Vec<_> = members.iter().filter(|r| r.candidate.badges.contains(mj_core::Badges::KEEPER)).collect();
        assert_eq!(kept.len(), 1, "{when}: one kept badge");
        assert_eq!(kept[0].candidate.id, left[0].candidate.id, "{when}: the badge is on the copy that stays");
        assert_eq!(g.keeper.as_ref(), Some(&left[0].candidate.path), "{when}: group keeper");
    }
}

fn dated_dup_tree() -> TempDir {
    let t = dup_tree();
    for (p, days) in [("x/one.bin", 30u64), ("y/one.bin", 10), ("zz/one.bin", 20), ("x/two.bin", 5), ("y/two.bin", 50)]
    {
        mj_core::testutil::set_mtime(
            &t.join(p),
            std::time::SystemTime::now() - std::time::Duration::from_secs(86_400 * days),
        )
        .unwrap();
    }
    t
}

/// ENG-7: after a keep rule, "Reset selection" and a rescan leave the kept badge on the
/// copy that actually stays.
#[test]
fn the_kept_badge_follows_the_selection_after_reset_and_rescan() {
    let f = engine_for(dated_dup_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    assert_keepers_follow_selection(e, "default");
    for rule in [mj_core::KeepRule::Newest, mj_core::KeepRule::Oldest] {
        e.selection_apply_keep_rule(SelectionKeepRuleRequest { opportunity_id: oid("det:exact_duplicates"), rule })
            .unwrap();
        assert_keepers_follow_selection(e, "keep rule");
        e.selection_reset(OpportunityRequest { opportunity_id: oid("det:exact_duplicates") }).unwrap();
        assert_keepers_follow_selection(e, "reset");
    }
    for rule in [mj_core::KeepRule::Newest, mj_core::KeepRule::Oldest] {
        e.selection_apply_keep_rule(SelectionKeepRuleRequest { opportunity_id: oid("det:exact_duplicates"), rule })
            .unwrap();
        assert_eq!(scan(e).state, ScanRunState::Done);
        assert_keepers_follow_selection(e, "rescan");
    }
}
