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

fn ack(step_id: mj_core::StepId) -> mj_core::StepConfirmation {
    mj_core::StepConfirmation {
        step_id,
        acknowledged: true,
        originals: mj_core::OriginalsPolicy::RecycleBin,
        synced_online_only: false,
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
        let confirmations = plan.steps.iter().map(|s| ack(s.id)).collect();
        e.plan_apply(PlanApplyRequest { confirmations }).unwrap();
        wait_jobs(e);
        let left = [f.tree.join("alpha/a.bin"), f.tree.join("beta/a.bin")].iter().filter(|p| exists(p)).count();
        assert!(left >= 1, "folder_first={folder_first}: every copy is gone");
        assert!(exists(&keeper.candidate.path), "folder_first={folder_first}: the kept copy is gone");
    }
}
