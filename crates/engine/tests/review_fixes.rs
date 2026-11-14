//! Engine-level regression tests for `docs/review/adversarial-review-1.md`. Each test
//! drives the real `Engine` on a generated tree inside a `TempDir` (app folders in
//! another `TempDir`). Names start with the finding id.

mod common;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::{Badges, OpportunityId};
use mj_engine::api::*;

fn oid(s: &str) -> OpportunityId {
    OpportunityId(s.into())
}

fn select(e: &mj_engine::Engine, id: &str, ids: Option<Vec<mj_core::CandidateId>>, selected: bool) -> StepSummary {
    e.selection_set(SelectionSetRequest { opportunity_id: oid(id), ids, sub_group: None, selected }).unwrap()
}

fn rows(e: &mj_engine::Engine, id: &str) -> Vec<CandidateRow> {
    e.candidates_page(CandidatesPageRequest {
        opportunity_id: oid(id),
        sub_group: None,
        offset: 0,
        limit: 500,
        sort: CandidateSortKey::Path,
        descending: false,
        filter: None,
    })
    .unwrap()
    .rows
}

fn pair_tree() -> TempDir {
    let t = TempDir::new("f02-tree").unwrap();
    let data = bytes(7, 200_000);
    t.write("photos/a.bin", &data).unwrap();
    t.write("downloads/a.bin", &data).unwrap();
    t
}

#[test]
fn f02_select_all_never_selects_the_kept_copy() {
    let f = engine_for(pair_tree());
    let e = &f.engine;
    scan(e);
    let id = "det:exact_duplicates";
    let s = select(e, id, None, true);
    assert_eq!(s.selected, 1, "the header 'Select all' leaves the keeper unchecked");
    let r = rows(e, id);
    assert!(r.iter().all(|r| r.selected != r.candidate.badges.contains(Badges::KEEPER)));
    e.plan_add_step(PlanAddStepRequest { opportunity_id: oid(id), action: None, params: None }).unwrap();
    e.plan_apply(PlanApplyRequest::default()).unwrap();
    wait_jobs(e);
    let left = [path(&f.tree, "photos/a.bin"), path(&f.tree, "downloads/a.bin")].iter().filter(|p| exists(p)).count();
    assert_eq!(left, 1, "the duplicate went, the kept copy stayed");
}

#[test]
fn f02_apply_keeps_one_copy_even_when_every_copy_is_selected() {
    for action in [mj_core::Action::Trash, mj_core::Action::PermanentDelete] {
        let f = engine_for(pair_tree());
        let e = &f.engine;
        scan(e);
        let id = "det:exact_duplicates";
        // An explicit pick of every row, keeper included.
        let all: Vec<_> = rows(e, id).iter().map(|r| r.candidate.id).collect();
        assert_eq!(select(e, id, Some(all), true).selected, 2);
        let plan = e
            .plan_add_step(PlanAddStepRequest { opportunity_id: oid(id), action: Some(action.clone()), params: None })
            .unwrap();
        assert!(
            plan.checklist.iter().any(|c| c.code == "groups_every_copy_selected"),
            "the checklist warns: {:?}",
            plan.checklist
        );
        let step_id = plan.steps[0].id;
        let confirmations = vec![mj_core::StepConfirmation {
            step_id,
            acknowledged: true,
            originals: mj_core::OriginalsPolicy::RecycleBin,
            synced_online_only: false,
        }];
        e.plan_apply(PlanApplyRequest { confirmations }).unwrap();
        wait_jobs(e);
        let left =
            [path(&f.tree, "photos/a.bin"), path(&f.tree, "downloads/a.bin")].iter().filter(|p| exists(p)).count();
        assert_eq!(left, 2, "{action:?}: no copy of the group may be removed when all are selected");
    }
}

#[test]
fn f04_folder_that_gained_a_file_after_the_scan_is_not_deleted() {
    let t = TempDir::new("f04-tree").unwrap();
    t.mkdir("old/was-empty").unwrap();
    t.write("old/keep.txt", b"x").unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let id = "det:empty_folders";
    assert!(rows(e, id).iter().any(|r| r.candidate.path.ends_with("was-empty")), "{:?}", rows(e, id));
    // After the scan the user saves a document into the "empty" folder.
    let doc = f.tree.write("old/was-empty/thesis.docx", b"two years of work").unwrap();
    let plan = e
        .plan_add_step(PlanAddStepRequest {
            opportunity_id: oid(id),
            action: Some(mj_core::Action::PermanentDelete),
            params: None,
        })
        .unwrap();
    let confirmations = vec![mj_core::StepConfirmation {
        step_id: plan.steps[0].id,
        acknowledged: true,
        originals: mj_core::OriginalsPolicy::RecycleBin,
        synced_online_only: false,
    }];
    e.plan_apply(PlanApplyRequest { confirmations }).unwrap();
    wait_jobs(e);
    assert_eq!(std::fs::read(&doc).unwrap(), b"two years of work");
}

#[test]
fn f08_unreadable_journal_blocks_every_acting_command_but_not_reading() {
    let app = TempDir::new("f08-app").unwrap();
    // A month file the journal cannot read (a folder in its place).
    std::fs::create_dir_all(app.join("journal/entries-2026-11.jsonl")).unwrap();
    let tree = pair_tree();
    let events = std::sync::Arc::new(TimedSink::default());
    let e = mj_engine::Engine::new(config(&app, events)).unwrap();
    e.scope_add_root(ScopePathRequest { path: tree.path().to_path_buf() }).unwrap();
    assert!(!e.app_info().unwrap().journal_persistent);
    scan(&e);
    let id = "det:exact_duplicates";
    assert!(!rows(&e, id).is_empty(), "reading still works");
    e.plan_add_step(PlanAddStepRequest { opportunity_id: oid(id), action: None, params: None }).unwrap();
    let err = e.plan_apply(PlanApplyRequest::default()).unwrap_err();
    assert_eq!(err.code(), "conflict", "{err}");
    assert!(err.to_string().contains("journal"), "{err}");
    assert_eq!(e.quarantine_purge(QuarantinePurgeRequest { older_than_days: Some(0) }).unwrap_err().code(), "conflict");
    assert_eq!(e.undo_last().unwrap_err().code(), "conflict");
    assert!(exists(&path(&tree, "photos/a.bin")) && exists(&path(&tree, "downloads/a.bin")));
}
