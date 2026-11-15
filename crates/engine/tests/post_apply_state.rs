//! State after an apply (W6-A review findings ENG-11 and ENG-12): cards, groups,
//! re-analyses and searches after files were changed, and custom-search cards after a
//! rescan.

mod common;

use common::*;
use mj_core::testutil::{set_mtime, TempDir};
use mj_core::{OpportunityId, ScanScope};
use mj_engine::api::*;

fn oid(s: &str) -> OpportunityId {
    OpportunityId(s.into())
}

fn old(t: &TempDir, rel: &str, seed: u64, days: u64) {
    let p = t.write(rel, &bytes(seed, 4000)).unwrap();
    set_mtime(&p, std::time::SystemTime::now() - std::time::Duration::from_secs(86_400 * days)).unwrap();
}

/// Two old installers and two pairs of exact duplicates.
fn tree() -> TempDir {
    let t = TempDir::new("post-apply").unwrap();
    old(&t, "downloads/setup-0.msi", 10, 400);
    old(&t, "downloads/setup-1.msi", 11, 400);
    for (name, seed) in [("x.bin", 20), ("y.bin", 21)] {
        old(&t, &format!("a/{name}"), seed, 30);
        old(&t, &format!("b/{name}"), seed, 3);
    }
    t
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

/// Row paths of a card, relative to the scanned folder.
fn names(e: &mj_engine::Engine, id: &str) -> Vec<String> {
    let root = e.scope_get().unwrap().roots[0].clone();
    let page = e.candidates_page(page_req(id)).unwrap();
    let rel = |p: &std::path::Path| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
    page.rows.into_iter().map(|r| rel(&r.candidate.path)).collect()
}

fn search(e: &std::sync::Arc<mj_engine::Engine>, query: &str, title: &str) -> mj_core::Opportunity {
    e.search_run(SearchRunRequest {
        rule: mj_filters::parse(query).unwrap(),
        title: Some(title.into()),
        topmost_only: false,
        action: None,
    })
    .unwrap()
}

fn apply(e: &std::sync::Arc<mj_engine::Engine>, id: &str) {
    e.plan_add_step(PlanAddStepRequest { opportunity_id: oid(id), action: None, params: None }).unwrap();
    e.plan_apply(PlanApplyRequest::default()).unwrap();
    let jobs = wait_jobs(e);
    assert!(jobs.iter().all(|j| j.state == mj_core::JobState::Completed), "{jobs:?}");
}

/// ENG-11: an applied duplicate card leaves no kept-only groups behind, and neither a
/// search nor a re-analysis on the unchanged index brings removed files back.
#[test]
fn applied_items_stay_gone_until_the_next_scan() {
    let f = engine_for(tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    assert_eq!(names(e, "det:exact_duplicates").len(), 4);
    apply(e, "det:exact_duplicates");
    assert!(!exists(&f.tree.join("b/x.bin")) && exists(&f.tree.join("a/x.bin")));
    assert!(opp(e, "det:exact_duplicates").is_none(), "no card of lone kept copies");
    assert!(names(e, "det:exact_duplicates").is_empty());
    // A search on the index of the last scan does not list the removed copies.
    let bins = search(e, "ext:bin", "Bins").id.0;
    assert_eq!(names(e, &bins), vec!["a/x.bin", "a/y.bin"]);
    // A re-analysis with new parameters does not either.
    let unchecked = e.candidates_page(CandidatesPageRequest {
        opportunity_id: oid("det:old_installers"),
        sub_group: None,
        offset: 0,
        limit: 10,
        sort: CandidateSortKey::Path,
        descending: false,
        filter: None,
        min_size: None,
        keep_groups: false,
    });
    let one = unchecked.unwrap().rows.into_iter().find(|r| r.candidate.path.ends_with("setup-1.msi")).unwrap();
    e.selection_set(SelectionSetRequest {
        opportunity_id: oid("det:old_installers"),
        ids: Some(vec![one.candidate.id]),
        sub_group: None,
        selected: false,
        filter: None,
        min_size: None,
        keep_groups: false,
    })
    .unwrap();
    apply(e, "det:old_installers");
    assert!(!exists(&f.tree.join("downloads/setup-0.msi")));
    assert_eq!(names(e, "det:old_installers"), vec!["downloads/setup-1.msi"]);
    let r = e
        .step_params_set(StepParamsRequest {
            opportunity_id: oid("det:old_installers"),
            params: mj_core::ParamValues::new().with("min_age_days", mj_core::ParamValue::Number(200.0)),
            preset_id: None,
        })
        .unwrap();
    assert_eq!(r.summary.candidates, 1, "the removed installer is not a candidate again");
    assert_eq!(names(e, "det:old_installers"), vec!["downloads/setup-1.msi"]);
    // The next scan sees the disk as it is.
    assert_eq!(scan(e).state, ScanRunState::Done);
    assert_eq!(names(e, "det:old_installers"), vec!["downloads/setup-1.msi"]);
    let bins = search(e, "ext:bin", "Bins").id.0;
    assert_eq!(names(e, &bins), vec!["a/x.bin", "a/y.bin"]);
}

/// ENG-11: when a group's copies are removed by another card, the duplicate group that is
/// left with one member leaves the duplicate card.
#[test]
fn a_group_emptied_by_another_card_leaves_the_duplicate_card() {
    let f = engine_for(tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let one = search(e, "name:x.bin", "One copy").id;
    assert_eq!(names(e, &one.0), vec!["a/x.bin", "b/x.bin"]);
    let page = e.candidates_page(page_req(&one.0)).unwrap();
    let keep = page.rows.iter().find(|r| r.candidate.path.ends_with("a/x.bin")).unwrap().candidate.id;
    e.selection_set(SelectionSetRequest {
        opportunity_id: one.clone(),
        ids: Some(vec![keep]),
        sub_group: None,
        selected: false,
        filter: None,
        min_size: None,
        keep_groups: false,
    })
    .unwrap();
    apply(e, &one.0);
    assert!(!exists(&f.tree.join("b/x.bin")));
    assert_eq!(names(e, "det:exact_duplicates"), vec!["a/y.bin", "b/y.bin"]);
}

/// ENG-12: a custom search card and its plan step survive a rescan (the rule is run again
/// on the new index, the rows the user unchecked stay unchecked).
#[test]
fn a_custom_search_card_and_its_step_survive_a_rescan() {
    let f = engine_for(tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    search(e, "ext:msi", "Installers");
    let rows = e
        .candidates_page(CandidatesPageRequest {
            opportunity_id: oid("custom:1"),
            sub_group: None,
            offset: 0,
            limit: 10,
            sort: CandidateSortKey::Path,
            descending: false,
            filter: None,
            min_size: None,
            keep_groups: false,
        })
        .unwrap()
        .rows;
    let first = rows.iter().find(|r| r.candidate.path.ends_with("setup-0.msi")).unwrap().candidate.id;
    e.selection_set(SelectionSetRequest {
        opportunity_id: oid("custom:1"),
        ids: Some(vec![first]),
        sub_group: None,
        selected: false,
        filter: None,
        min_size: None,
        keep_groups: false,
    })
    .unwrap();
    e.plan_add_step(PlanAddStepRequest { opportunity_id: oid("custom:1"), action: None, params: None }).unwrap();
    // A new installer appears before the rescan.
    old(&f.tree, "downloads/setup-2.msi", 12, 400);
    assert_eq!(scan(e).state, ScanRunState::Done);
    let card = opp(e, "custom:1").expect("the custom card survives the rescan");
    assert_eq!(card.title, "Installers");
    assert_eq!(names(e, "custom:1").len(), 3, "the rule ran on the new index");
    let plan = e.plan_get().unwrap();
    assert!(plan.steps.iter().any(|s| s.opportunity.0 == "custom:1"), "{plan:?}");
    let page = e
        .candidates_page(CandidatesPageRequest {
            opportunity_id: oid("custom:1"),
            sub_group: None,
            offset: 0,
            limit: 10,
            sort: CandidateSortKey::Path,
            descending: false,
            filter: None,
            min_size: None,
            keep_groups: false,
        })
        .unwrap();
    for r in page.rows {
        assert_eq!(r.selected, !r.candidate.path.ends_with("setup-0.msi"), "{:?}", r.candidate.path);
    }
}

/// ENG-12: a schedule cannot hold a custom-search step (its rule is not stored with it):
/// both the save and "Save as weekly schedule" say so instead of saving a step that
/// never finds anything.
#[test]
fn a_custom_search_step_cannot_be_scheduled() {
    let f = engine_for(tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    search(e, "ext:msi", "Installers");
    e.plan_add_step(PlanAddStepRequest { opportunity_id: oid("custom:1"), action: None, params: None }).unwrap();
    let err = e
        .schedule_from_plan(ScheduleFromPlanRequest {
            name: "Weekly".into(),
            cadence: mj_core::Cadence::Weekly { weekday: 1, hour: 9, minute: 13 },
        })
        .unwrap_err();
    assert_eq!(err.code(), "invalid_input");
    assert!(err.to_string().contains("custom search"), "{err}");
    let schedule = mj_core::Schedule {
        id: String::new(),
        name: "Direct".into(),
        enabled: true,
        cadence: mj_core::Cadence::Weekly { weekday: 1, hour: 9, minute: 13 },
        scope: ScanScope { roots: vec![f.tree.path().to_path_buf()], ..Default::default() },
        steps: vec![mj_core::ScheduledStep {
            opportunity: oid("custom:1"),
            action: mj_core::Action::Trash,
            params: Default::default(),
        }],
        last_run: None,
        next_run: None,
    };
    assert!(e.schedule_save(ScheduleSaveRequest { schedule }).unwrap_err().to_string().contains("custom search"));
}
