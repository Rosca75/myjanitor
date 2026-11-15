//! Engine-level regression tests for `docs/review/adversarial-review-2.md`. Each test
//! drives the real `Engine` on a generated tree inside a `TempDir` (app folders in
//! another `TempDir`, Trash routed to the test's quarantine). Names start with the
//! finding id.

mod common;

use std::time::{Duration, SystemTime};

use common::*;
use mj_core::testutil::{set_mtime, TempDir};
use mj_core::{Badges, OpportunityId};
use mj_engine::api::*;

fn oid(s: &str) -> OpportunityId {
    OpportunityId(s.into())
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

fn select(e: &mj_engine::Engine, id: &str, ids: Option<Vec<mj_core::CandidateId>>, selected: bool) -> StepSummary {
    e.selection_set(SelectionSetRequest { opportunity_id: oid(id), ids, sub_group: None, selected }).unwrap()
}

/// The dialog's answer for one step, bound to what the dialog showed (G-04).
fn ack(e: &mj_engine::Engine, step_id: mj_core::StepId) -> mj_core::StepConfirmation {
    mj_core::StepConfirmation {
        step_id,
        acknowledged: true,
        originals: mj_core::OriginalsPolicy::RecycleBin,
        synced_online_only: false,
        selection_digest: Some(e.plan_confirmation_info(StepRequest { step_id }).unwrap().selection_digest),
    }
}

fn old(t: &TempDir, rel: &str, data: &[u8], days: u64) {
    let p = t.write(rel, data).unwrap();
    set_mtime(&p, SystemTime::now() - Duration::from_secs(days * 86_400)).unwrap();
}

/// G-03: step 1 removes a duplicate (keeping the copy in `alpha/`), step 2 removes the
/// inactive folder that holds the kept copy. Whatever the order, one copy stays in place (Trash goes to the test quarantine).
#[test]
fn g03_a_later_step_never_removes_the_copy_an_earlier_step_kept() {
    for folder_first in [false, true] {
        let t = TempDir::new("g03-tree").unwrap();
        let data = bytes(9, 120_000);
        old(&t, "alpha/a.bin", &data, 800);
        old(&t, "alpha/notes.txt", b"notes", 800);
        old(&t, "beta/a.bin", &data, 700);
        old(&t, "beta/other.txt", b"other", 700);
        let f = engine_for(t);
        let e = &f.engine;
        scan(e);
        let dup = "det:exact_duplicates";
        let keeper =
            rows(e, dup).into_iter().find(|r| r.candidate.badges.contains(Badges::KEEPER)).expect("a kept copy");
        let keep_dir = keeper.candidate.path.parent().unwrap().to_path_buf();
        let inactive = "det:inactive_folders";
        let folder = rows(e, inactive).into_iter().find(|r| r.candidate.path == keep_dir).expect("folder row");
        select(e, inactive, Some(vec![folder.candidate.id]), true);
        let add = |id: &str| {
            e.plan_add_step(PlanAddStepRequest {
                opportunity_id: oid(id),
                action: Some(mj_core::Action::Trash),
                params: None,
            })
            .unwrap()
        };
        let plan = if folder_first {
            add(inactive);
            add(dup)
        } else {
            add(dup);
            add(inactive)
        };
        let confirmations = plan.steps.iter().map(|s| ack(e, s.id)).collect();
        e.plan_apply(PlanApplyRequest { confirmations }).unwrap();
        wait_jobs(e);
        let left = [f.tree.join("alpha/a.bin"), f.tree.join("beta/a.bin")].iter().filter(|p| exists(p)).count();
        assert!(left >= 1, "folder_first={folder_first}: every copy is gone");
        assert!(exists(&keeper.candidate.path), "folder_first={folder_first}: the kept copy is gone");
    }
}

/// G-04: the user unchecks one duplicate of a permanent-delete step and confirms; a rescan
/// finishes before the apply. The unchecked file must survive (the exclusion is carried
/// over by path), and a confirmation given before the selection changed is refused.
#[test]
fn g04_rescan_keeps_the_users_exclusions_and_stale_confirmations_are_refused() {
    let t = TempDir::new("g04-tree").unwrap();
    let data = bytes(11, 90_000);
    old(&t, "a/x.bin", &data, 30);
    old(&t, "b/x.bin", &data, 20);
    old(&t, "c/x.bin", &data, 10);
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let dup = "det:exact_duplicates";
    let r = rows(e, dup);
    let spare = r.iter().find(|r| r.selected).expect("a pre-selected duplicate").candidate.clone();
    select(e, dup, Some(vec![spare.id]), false);
    let plan = e
        .plan_add_step(PlanAddStepRequest {
            opportunity_id: oid(dup),
            action: Some(mj_core::Action::PermanentDelete),
            params: None,
        })
        .unwrap();
    let step = plan.steps[0].id;
    let confirmed = ack(e, step);
    // A scan finishes while the dialog is open.
    scan(e);
    assert!(
        rows(e, dup).iter().any(|r| r.candidate.path == spare.path && !r.selected),
        "the user's exclusion survives the rescan"
    );
    // The user changes the selection after confirming: the old confirmation is stale.
    let other = rows(e, dup).into_iter().find(|r| r.selected).unwrap().candidate;
    select(e, dup, Some(vec![other.id]), false);
    let err = e.plan_apply(PlanApplyRequest { confirmations: vec![confirmed.clone()] }).unwrap_err();
    assert_eq!(err.code(), "confirmation_required", "{err:?}");
    select(e, dup, Some(vec![other.id]), true);
    // Same selection as confirmed (after the rescan): accepted, and the unchecked row stays.
    e.plan_apply(PlanApplyRequest { confirmations: vec![confirmed] }).unwrap();
    wait_jobs(e);
    assert!(exists(&spare.path), "the file the user unchecked was deleted");
    let left = ["a/x.bin", "b/x.bin", "c/x.bin"].iter().filter(|p| exists(&f.tree.join(p))).count();
    assert_eq!(left, 2, "the kept copy and the unchecked one remain");
}
