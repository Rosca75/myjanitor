//! End to end over a generated fixture tree: scan → opportunities → plan → dry run →
//! apply (reversible) → history → undo, plus the refusal paths and the rescan cache.

mod common;

use common::*;
use mj_core::testutil::snapshot;
use mj_core::{Action, JobState};
use mj_engine::api::*;

const CARDS: [&str; 5] =
    ["det:exact_duplicates", "det:temp_files", "det:dev_artefacts", "det:empty_folders", "det:old_installers"];

fn page(e: &mj_engine::Engine, id: &str) -> CandidatesPage {
    e.candidates_page(CandidatesPageRequest {
        opportunity_id: mj_core::OpportunityId(id.into()),
        sub_group: None,
        offset: 0,
        limit: 500,
        sort: CandidateSortKey::Path,
        descending: false,
        filter: None,
        min_size: None,
        keep_groups: false,
    })
    .unwrap()
}

#[cfg(unix)]
fn allocated(p: &std::path::Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(p).unwrap().blocks() * 512
}

#[test]
fn scan_plan_dry_run_apply_and_undo_restore_the_tree() {
    let f = engine_for(standard_tree());
    let e = &f.engine;
    let before = snapshot(f.tree.path()).unwrap();
    let status = scan(e);
    assert_eq!(status.state, ScanRunState::Done, "{:?}", status.error);

    // Opportunities: the expected cards with the selected sizes.
    for id in CARDS {
        let o = opp(e, id).unwrap_or_else(|| panic!("card {id} missing"));
        assert!(o.availability.is_available());
        assert!(o.selected >= 1, "{id}");
        let sum: u64 = page(e, id).rows.iter().filter(|r| r.selected).map(|r| r.candidate.saving()).sum();
        assert_eq!(o.saving, sum, "{id}");
    }
    #[cfg(unix)]
    {
        assert_eq!(opp(e, "det:temp_files").unwrap().saving, allocated(&f.tree.join("work/report.tmp")));
        assert_eq!(opp(e, "det:exact_duplicates").unwrap().saving, allocated(&f.tree.join("downloads/a.bin")));
        assert_eq!(opp(e, "det:old_installers").unwrap().saving, allocated(&f.tree.join("downloads/setup-1.0.msi")));
    }
    let list = e.opportunities_list(OpportunitiesRequest::default()).unwrap();
    let card_total: u64 = CARDS.iter().map(|id| opp(e, id).unwrap().saving).sum();
    assert_eq!(list.summary.total, card_total);
    assert_eq!(list.summary.reversible, card_total);
    // Disabled optimiser cards are listed with a reason, after the available ones.
    let first_disabled = list.items.iter().position(|o| !o.availability.is_available()).unwrap();
    assert!(list.items[first_disabled..].iter().all(|o| !o.availability.is_available()));

    // Plan: add the five steps; the estimate is the sum.
    for id in CARDS {
        e.plan_add_step(PlanAddStepRequest {
            opportunity_id: mj_core::OpportunityId(id.into()),
            action: None,
            params: None,
        })
        .unwrap();
    }
    let plan = e.plan_get().unwrap();
    assert_eq!(plan.steps.len(), 5);
    assert_eq!(plan.estimate.saving(), card_total);
    assert!(plan.checklist.iter().any(|c| c.code == "skipped_confidential"), "{:?}", plan.checklist);
    assert!(plan.steps.iter().all(|s| !s.safety.requires_confirmation()));

    // Dry run: exact and equal to the estimate; nothing changed.
    let job = e.plan_dry_run().unwrap();
    let report = wait("dry run", || {
        f.events.events().into_iter().find_map(|ev| match ev {
            Event::DryRun(r) if r.job_id == job.id => Some(r),
            _ => None,
        })
    });
    assert_eq!(report.total.saving(), card_total, "{report:?}");
    assert_eq!(report.total.confidence, mj_core::Confidence::Exact);
    assert_eq!(report.total.skipped, 0, "{report:?}");
    assert_eq!(snapshot(f.tree.path()).unwrap(), before);
    assert_eq!(e.plan_get().unwrap().estimate.confidence, mj_core::Confidence::Exact);

    // Apply (reversible, no confirmation needed).
    let started = e.plan_apply(PlanApplyRequest::default()).unwrap();
    assert_eq!(started.jobs.len(), 5);
    let jobs = wait_jobs(e);
    for j in jobs.iter().filter(|j| j.run_id == Some(started.run_id)) {
        assert_eq!(j.state, JobState::Completed, "{j:?}");
    }
    for gone in ["downloads/a.bin", "work/report.tmp", "proj/node_modules", "old", "downloads/setup-1.0.msi"] {
        assert!(!exists(&f.tree.join(gone)), "{gone} still there");
    }
    for kept in
        ["photos/a.bin", "contracts/signed.pdf", "downloads/signed.pdf", "proj/package.json", "photos/link-to-a"]
    {
        // The fixture link is made on Unix only (see `standard_tree`).
        if kept.ends_with("link-to-a") && !cfg!(unix) {
            continue;
        }
        assert!(exists(&f.tree.join(kept)), "{kept} removed");
    }
    // Items went to the test's quarantine (Trash route), never to the real Trash.
    assert!(std::fs::read_dir(f.app.join("quarantine")).unwrap().next().is_some());
    let list = e.opportunities_list(OpportunitiesRequest::default()).unwrap();
    assert_eq!(list.summary.reclaimed, card_total);
    assert!(e.plan_get().unwrap().steps.is_empty(), "applied steps leave the plan");
    assert!(opp(e, "det:temp_files").is_none(), "applied items leave their cards");
    // Journal persisted.
    let journal_files: Vec<_> = std::fs::read_dir(f.app.join("journal")).unwrap().flatten().collect();
    assert!(journal_files.iter().any(|d| d.file_name().to_string_lossy().starts_with("entries-")));
    let history = e.history_list(PageRequest::default()).unwrap();
    assert_eq!(history.runs[0].run_id, started.run_id);
    assert!(history.runs[0].undoable);
    assert_eq!(history.runs[0].done, 5);
    let report = e.report_last().unwrap().unwrap();
    assert_eq!(report.run_id, started.run_id);
    assert_eq!(report.totals.succeeded, 5);
    assert_eq!(report.sections.len(), 5);
    assert!(f.events.events().iter().any(|ev| matches!(ev, Event::HistoryChanged(_))));

    // A fresh engine over the same folders sees the persisted history.
    {
        let e2 = mj_engine::Engine::new(config(&f.app, Default::default())).unwrap();
        assert_eq!(e2.history_list(PageRequest::default()).unwrap().runs[0].run_id, started.run_id);
        assert!(e2.app_info().unwrap().journal_persistent);
    }

    // Undo the run: the tree is back to its initial state (paths and contents).
    let undo = e.undo_run(RunRequest { run_id: started.run_id }).unwrap();
    let jobs = wait_jobs(e);
    let u = jobs.iter().find(|j| j.id == undo.id).unwrap();
    assert_eq!(u.state, JobState::Completed, "{u:?}");
    assert_eq!(snapshot(f.tree.path()).unwrap(), before);
    let history = e.history_list(PageRequest::default()).unwrap();
    assert!(history.runs.iter().any(|r| r.kind == mj_core::RunKind::Undo));
    let run = history.runs.iter().find(|r| r.run_id == started.run_id).unwrap();
    assert_eq!(run.undone, 5);
    assert!(!run.undoable);
    assert!(e.undo_last().is_err(), "nothing left to undo");
    assert_eq!(e.quarantine_info().unwrap().items, 0);
}

#[test]
fn permanent_step_needs_confirmation_and_signed_files_are_skipped() {
    let f = engine_for(standard_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let temp = mj_core::OpportunityId("det:temp_files".into());
    let plan = e
        .plan_add_step(PlanAddStepRequest {
            opportunity_id: temp.clone(),
            action: Some(Action::PermanentDelete),
            params: None,
        })
        .unwrap();
    let step = plan.steps[0].id;
    assert_eq!(plan.steps[0].safety, mj_core::SafetyClass::Permanent);
    let err = e.plan_apply(PlanApplyRequest::default()).unwrap_err();
    assert_eq!(err.code(), "confirmation_required");
    // An unticked acknowledgement is no confirmation.
    let unticked = mj_core::StepConfirmation {
        step_id: step,
        acknowledged: false,
        originals: Default::default(),
        synced_online_only: false,
        selection_digest: None,
    };
    let err = e.plan_apply(PlanApplyRequest { confirmations: vec![unticked] }).unwrap_err();
    assert_eq!(err.code(), "confirmation_required");
    assert!(e.jobs_list().unwrap().is_empty(), "nothing ran");
    assert!(exists(&f.tree.join("work/report.tmp")));
    let info = e.plan_confirmation_info(StepRequest { step_id: step }).unwrap();
    assert!(info.permanent);
    assert!(info.originals_options.is_empty());
    e.plan_clear().unwrap();

    // Signed duplicates: locked, never selectable, reported, untouched by apply.
    let dups = mj_core::OpportunityId("det:exact_duplicates".into());
    let rows = page(e, "det:exact_duplicates").rows;
    let signed: Vec<_> = rows.iter().filter(|r| r.candidate.badges.contains(mj_core::Badges::SIGNED)).collect();
    assert_eq!(signed.len(), 2);
    assert!(signed.iter().all(|r| !r.actionable && !r.selected && r.blocked_reason.is_some()));
    let ids: Vec<_> = signed.iter().map(|r| r.candidate.id).collect();
    e.selection_set(SelectionSetRequest {
        opportunity_id: dups.clone(),
        ids: Some(ids),
        sub_group: None,
        selected: true,
    })
    .unwrap();
    assert!(page(e, "det:exact_duplicates")
        .rows
        .iter()
        .filter(|r| r.candidate.badges.contains(mj_core::Badges::SIGNED))
        .all(|r| !r.selected));
    e.plan_add_step(PlanAddStepRequest { opportunity_id: dups, action: None, params: None }).unwrap();
    let plan = e.plan_get().unwrap();
    assert!(plan.checklist.iter().any(|c| c.code == "skipped_confidential"));
    let job = e.plan_dry_run().unwrap();
    let report = wait("dry run", || {
        f.events.events().into_iter().find_map(|ev| match ev {
            Event::DryRun(r) if r.job_id == job.id => Some(r),
            _ => None,
        })
    });
    assert!(report.steps[0].warnings.iter().any(|w| w.contains("signed")), "{report:?}");
    e.plan_apply(PlanApplyRequest::default()).unwrap();
    wait_jobs(e);
    assert!(exists(&f.tree.join("downloads/signed.pdf")));
    assert!(exists(&f.tree.join("contracts/signed.pdf")));
    assert!(!exists(&f.tree.join("downloads/a.bin")));
}

#[test]
fn rescan_uses_and_saves_the_cache() {
    let f = engine_for(standard_tree());
    let e = &f.engine;
    let first = scan(e);
    assert_eq!(first.state, ScanRunState::Done);
    let cache = f.app.join("cache").join("scan-cache.bin");
    assert!(cache.is_file(), "cache saved after the scan");
    let info = e.cache_info().unwrap();
    assert!(info.entries > 0);
    let second = scan(e);
    assert_eq!(second.state, ScanRunState::Done);
    assert!(second.generation > first.generation);
    let stats = second.stats.unwrap();
    assert!(stats.cache_hits > 0, "{stats:?}");
    assert!(second.cached);
    assert!(e.opportunities_list(OpportunitiesRequest::default()).unwrap().cached);
    // A fresh engine loads the saved cache.
    let e2 = mj_engine::Engine::new(config(&f.app, Default::default())).unwrap();
    assert!(e2.cache_info().unwrap().entries > 0);
}
