//! The "You can free up" invariant (W6-B), on a mixed generated tree: duplicates, a
//! temp file, developer artefacts, an old installer that also has a duplicate, files the
//! optimisers can shrink (one of them duplicated), stale files and an inactive folder.
//!
//! For one generation, the same bytes must come out of every surface:
//!
//! - `summary.total` = the sum of the cards' `saving` = the sum of its three buckets;
//! - a file selected in several opportunities (or kept by one and removed by another, or
//!   lying inside a folder another one removes) is counted once;
//! - with every card in the plan, the plan total, the per-drive figures and (for
//!   removals, whose sizes are exact) the dry run say the same as the hero;
//! - an action that frees nothing (review only, copy, light copy) counts nothing, and a
//!   removal counts the whole file whatever the detector estimated for its own action.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use common::fixtures;
use common::*;
use mj_core::testutil::{set_mtime, TempDir};
use mj_core::{Action, Opportunity, OpportunityId};
use mj_engine::api::*;
use mj_engine::Engine;

fn oid(s: &str) -> OpportunityId {
    OpportunityId(s.into())
}

fn old(t: &TempDir, rel: &str, data: &[u8], days: u64) {
    let p = t.write(rel, data).unwrap();
    set_mtime(&p, SystemTime::now() - Duration::from_secs(days * 86_400)).unwrap();
}

/// The standard tree plus: a second copy of the old installer (so the installer is a kept
/// duplicate *and* an old installer), optimisable files with one exact copy of the PNG
/// (so the copy is a duplicate *and* a PNG to recompress), and old data for the
/// review-only cards.
fn mixed_tree() -> TempDir {
    let t = standard_tree();
    old(&t, "backup/setup-1.0.msi", &bytes(4, 40_000), 390);
    t.write("img/a.png", &fixtures::png()).unwrap();
    t.write("img/copy/a.png", &fixtures::png()).unwrap();
    t.write("img/b.jpg", &fixtures::jpeg()).unwrap();
    t.write("books/c.epub", &fixtures::epub()).unwrap();
    t.write("decks/d.pptx", &fixtures::pptx(false)).unwrap();
    t.write("docs/e.pdf", &fixtures::pdf()).unwrap();
    old(&t, "archive2019/big.dat", &bytes(21, 2_500_000), 900);
    old(&t, "archive2019/notes.txt", b"old notes", 900);
    old(&t, "ClientA/report.dat", &bytes(22, 1_500_000), 800);
    t
}

fn rows(e: &Engine, id: &str) -> Vec<CandidateRow> {
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

fn list(e: &Engine) -> OpportunitiesList {
    e.opportunities_list(OpportunitiesRequest::default()).unwrap()
}

fn available(l: &OpportunitiesList) -> Vec<&Opportunity> {
    l.items.iter().filter(|o| o.availability.is_available()).collect()
}

fn select(e: &Engine, id: &str, ids: Option<Vec<mj_core::CandidateId>>, selected: bool) -> StepSummary {
    e.selection_set(SelectionSetRequest { opportunity_id: oid(id), ids, sub_group: None, selected }).unwrap()
}

fn add(e: &Engine, id: &str, action: Option<Action>) -> mj_core::Plan {
    e.plan_add_step(PlanAddStepRequest { opportunity_id: oid(id), action, params: None }).unwrap()
}

fn dry_run(f: &Fixture) -> DryRunReport {
    let job = f.engine.plan_dry_run().unwrap();
    wait("dry run", || {
        f.events.events().into_iter().find_map(|ev| match ev {
            Event::DryRun(r) if r.job_id == job.id => Some(r),
            _ => None,
        })
    })
}

/// What the selected rows of every available card would free if each row stood alone,
/// keyed by path: `(card, saving)` per listing.
fn selected_by_path(e: &Engine, l: &OpportunitiesList) -> BTreeMap<PathBuf, Vec<(String, u64)>> {
    let mut m: BTreeMap<PathBuf, Vec<(String, u64)>> = BTreeMap::new();
    for o in available(l) {
        for r in rows(e, &o.id.0).into_iter().filter(|r| r.selected) {
            m.entry(r.candidate.path.clone()).or_default().push((o.id.0.clone(), r.candidate.saving()));
        }
    }
    m
}

#[test]
fn hero_is_the_sum_of_the_cards_and_counts_every_file_once() {
    let f = engine_for(mixed_tree());
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let l = list(e);
    let s = l.summary;

    // The hero is exactly what the cards show, and its buckets add up.
    let cards: u64 = available(&l).iter().map(|o| o.saving).sum();
    assert_eq!(s.total, cards, "hero total vs sum of the cards");
    assert_eq!(s.total, s.reversible + s.opt_in + s.review_only, "{s:?}");
    // What is only listed for review is reported next to the total, never in it.
    assert_eq!(s.to_review, available(&l).iter().map(|o| o.to_review).sum::<u64>());
    assert!(s.to_review > 0 && s.review_opportunities > 0, "the review-only cards list old data: {s:?}");

    // The two overlaps of the tree are really there.
    let by_path = selected_by_path(e, &l);
    let copy = &by_path[&f.tree.join("img/copy/a.png")];
    assert_eq!(copy.len(), 2, "the PNG copy is a duplicate and a PNG to recompress: {copy:?}");
    let png_saving = copy.iter().find(|(id, _)| id == "opt:png_lossless").unwrap().1;
    let dup_saving = copy.iter().find(|(id, _)| id == "det:exact_duplicates").unwrap().1;
    assert!(dup_saving > png_saving && png_saving > 0);
    let kept = f.tree.join("downloads/setup-1.0.msi");
    let installer = &by_path[&kept];
    assert_eq!(installer.len(), 1);
    assert!(
        rows(e, "det:exact_duplicates")
            .iter()
            .any(|r| r.candidate.path == kept && r.candidate.badges.contains(mj_core::Badges::KEEPER)),
        "the old installer is the copy the duplicates card keeps"
    );

    // Every file once: the copy counts for the card that frees most (the removal), and
    // the installer the duplicates card keeps is not counted as removed by another card.
    let standalone: u64 = by_path.values().flatten().map(|(_, s)| s).sum();
    let expected = standalone - png_saving - installer[0].1;
    assert_eq!(s.total, expected, "standalone {standalone}, PNG overlap {png_saving}, kept copy {}", installer[0].1);

    // The cards say where the difference went.
    let card = |id: &str| available(&l).into_iter().find(|o| o.id.0 == id).unwrap().clone();
    assert_eq!(
        card("det:exact_duplicates").saving,
        by_path.values().flatten().filter(|(id, _)| id == "det:exact_duplicates").map(|(_, s)| s).sum::<u64>()
    );
    assert_eq!(card("det:old_installers").saving, 0);
    assert_eq!(card("det:old_installers").selected, 1, "the row stays ticked");
}

#[test]
fn summary_and_counts_do_not_depend_on_the_tab_or_the_sort() {
    let f = engine_for(mixed_tree());
    let e = &f.engine;
    scan(e);
    let base = list(e);
    let c = base.counts;
    assert_eq!(c.all, c.clean + c.optimise + c.archive + c.review_only, "{c:?}");
    assert_eq!(c.all as usize, base.items.len());
    for tab in [
        OpportunityTab::Clean,
        OpportunityTab::Optimise,
        OpportunityTab::Archive,
        OpportunityTab::ReviewOnly,
        OpportunityTab::All,
    ] {
        for sort in [OpportunitySort::Saving, OpportunitySort::Count, OpportunitySort::Safety, OpportunitySort::Name] {
            let l = e.opportunities_list(OpportunitiesRequest { tab, sort }).unwrap();
            assert_eq!(l.summary, base.summary, "{tab:?} {sort:?}");
            assert_eq!(l.counts, base.counts, "{tab:?} {sort:?}");
            for o in &l.items {
                let same = base.items.iter().find(|b| b.id == o.id).unwrap();
                assert_eq!((o.saving, o.size_before, o.count), (same.saving, same.size_before, same.count));
            }
        }
    }
}

#[test]
fn plan_total_drive_bars_and_dry_run_agree_with_the_hero() {
    let f = engine_for(mixed_tree());
    let e = &f.engine;
    scan(e);
    let before = list(e);
    for o in available(&before).into_iter().filter(|o| o.selected > 0) {
        add(e, &o.id.0, None);
    }
    let plan = e.plan_get().unwrap();
    let l = list(e);
    assert_eq!(l.summary, before.summary, "adding cards to the plan with their default action changes nothing");
    assert_eq!(plan.estimate.saving(), l.summary.total, "plan total vs hero");
    for s in &plan.steps {
        let card = l.items.iter().find(|o| o.id == s.opportunity).unwrap();
        assert_eq!(s.estimate.saving(), card.saving, "{}: plan step vs card", s.title);
    }
    // Per-drive bars: the plan's bytes, on the volume of the fixture tree.
    let drives = e.drives_list().unwrap();
    if drives.iter().any(|d| mj_core::pathutil::is_within(&d.volume.mount, f.tree.path())) {
        assert_eq!(drives.iter().map(|d| d.plan_saving).sum::<u64>(), plan.estimate.saving());
        for d in &drives {
            assert_eq!(d.free_after_plan, d.volume.free + d.plan_saving);
        }
    }
    // Dry run: removals are exact in both, so they must agree step by step; a file two
    // steps list is measured once.
    let estimates: BTreeMap<_, _> =
        plan.steps.iter().map(|s| (s.id, (s.title.clone(), s.action.clone(), s.estimate))).collect();
    let report = dry_run(&f);
    let mut removal_steps = 0;
    for d in &report.steps {
        let (title, action, est) = &estimates[&d.step_id];
        assert_eq!(d.estimate.files, est.files, "{title}: files");
        if matches!(action, Action::Trash) {
            removal_steps += 1;
            assert_eq!(d.estimate.saving(), est.saving(), "{title}: dry run vs estimate");
        }
    }
    assert!(removal_steps >= 4, "{removal_steps}");
    // The plan now shows the measured figures, and its total is still the sum of its steps.
    let measured = e.plan_get().unwrap();
    assert_eq!(measured.estimate.saving(), report.total.saving());
    assert_eq!(measured.estimate.saving(), measured.steps.iter().map(|s| s.estimate.saving()).sum::<u64>());
    // The copy of the PNG is removed by the duplicates step: the PNG step measures one file.
    let png = report
        .steps
        .iter()
        .find(|d| {
            d.title
                == estimates
                    .values()
                    .find(|(_, a, _)| {
                        matches!(a, Action::Optimise { optimiser: mj_core::OptimiserId::PngLossless, .. })
                    })
                    .unwrap()
                    .0
        })
        .unwrap();
    assert_eq!(png.estimate.files, 1, "{png:?}");
}

#[test]
fn a_group_whose_every_copy_is_selected_counts_nothing_anywhere() {
    let f = engine_for(mixed_tree());
    let e = &f.engine;
    scan(e);
    let dup = "det:exact_duplicates";
    let before = list(e);
    let card_before = before.items.iter().find(|o| o.id.0 == dup).unwrap().saving;
    // Tick the kept copy of the a.bin pair too: apply keeps one copy, so the pair is skipped.
    let pair: Vec<CandidateRow> = rows(e, dup).into_iter().filter(|r| r.candidate.path.ends_with("a.bin")).collect();
    assert_eq!(pair.len(), 2);
    let keeper = pair.iter().find(|r| !r.selected).unwrap();
    let removed = pair.iter().find(|r| r.selected).unwrap();
    let summary = select(e, dup, Some(vec![keeper.candidate.id]), true);
    let l = list(e);
    let card = l.items.iter().find(|o| o.id.0 == dup).unwrap();
    assert_eq!(card.saving, card_before - removed.candidate.saving(), "the pair no longer counts");
    assert_eq!(summary.saving, card.saving, "step summary vs card");
    assert_eq!(l.summary.total, before.summary.total - removed.candidate.saving());
    let plan = add(e, dup, None);
    assert_eq!(plan.estimate.saving(), card.saving, "plan step vs card");
}

#[test]
fn an_action_that_frees_nothing_counts_nothing_and_a_removal_counts_the_whole_file() {
    let f = engine_for(mixed_tree());
    let e = &f.engine;
    scan(e);
    let before = list(e);
    let stale = "det:stale_files";
    let card = before.items.iter().find(|o| o.id.0 == stale).expect("stale files card");
    assert_eq!((card.saving, card.selected), (0, 0), "review-only cards pre-select nothing");
    let big = rows(e, stale).into_iter().find(|r| r.candidate.path.ends_with("big.dat")).unwrap();
    assert!(big.candidate.size >= 2_500_000);
    select(e, stale, Some(vec![big.candidate.id]), true);

    // Ticked, but the card's action is "review only": nothing is freed, nothing is counted.
    let ticked = list(e);
    assert_eq!(ticked.summary.total, before.summary.total);
    let plan = add(e, stale, None);
    assert_eq!(plan.estimate.saving(), 0, "a review-only step changes nothing");
    let step = plan.steps[0].id;

    // The same step with a removal: the whole file counts (the detector's own estimate,
    // made for archiving, claimed no saving), in the plan, on the card and in the hero.
    let plan =
        e.plan_update_step(PlanUpdateStepRequest { step_id: step, action: Some(Action::Trash), params: None }).unwrap();
    assert_eq!(plan.estimate.saving(), big.candidate.size);
    let l = list(e);
    assert_eq!(l.items.iter().find(|o| o.id.0 == stale).unwrap().saving, big.candidate.size);
    assert_eq!(l.summary.total, before.summary.total + big.candidate.size);
    assert_eq!(l.summary.review_only, big.candidate.size, "bucketed by the card's class: {:?}", l.summary);
    let report = dry_run(&f);
    assert_eq!(report.total.saving(), plan.estimate.saving(), "dry run vs estimate");

    // A copy frees nothing.
    let dest = TempDir::new("w6b-copy-dest").unwrap();
    let plan = e
        .plan_update_step(PlanUpdateStepRequest {
            step_id: step,
            action: Some(Action::Copy { dest: dest.path().to_path_buf(), keep_structure: false }),
            params: None,
        })
        .unwrap();
    assert_eq!(plan.estimate.saving(), 0);
    assert_eq!(list(e).summary.total, before.summary.total);
}

#[test]
fn a_light_copy_frees_nothing() {
    let f = engine_for(mixed_tree());
    let e = &f.engine;
    scan(e);
    let id = "opt:power_point";
    let card = list(e).items.into_iter().find(|o| o.id.0 == id).expect("Office card");
    assert!(card.saving > 0);
    let mut action = card.default_action.clone();
    let Action::Optimise { output, .. } = &mut action else { panic!("{action:?}") };
    *output = mj_core::OutputMode::LightCopy { suffix: "_light".into() };
    let plan = add(e, id, Some(action));
    assert_eq!(plan.estimate.saving(), 0, "the original stays: a light copy adds a file, it frees nothing");
    assert_eq!(plan.estimate.files, 1);
    let report = dry_run(&f);
    assert_eq!(report.total.saving(), 0, "{report:?}");
    assert_eq!(list(e).items.into_iter().find(|o| o.id.0 == id).unwrap().saving, 0, "the card follows its plan step");
}

#[test]
fn a_safety_setting_change_moves_the_hero_and_the_cards_together() {
    let f = engine_for(mixed_tree());
    let e = &f.engine;
    let mut settings = e.settings_get().unwrap();
    settings.safety.skip_signed = false;
    e.settings_set(SettingsSetRequest { settings: settings.clone() }).unwrap();
    scan(e);
    let dup = "det:exact_duplicates";
    let signed = rows(e, dup)
        .into_iter()
        .find(|r| r.candidate.path.ends_with("downloads/signed.pdf"))
        .expect("the signed duplicate is listed");
    assert!(signed.actionable && !signed.selected, "signed rows are never pre-selected: {signed:?}");
    select(e, dup, Some(vec![signed.candidate.id]), true);
    let with = list(e);
    // Signed files are skipped again: the row is locked, in the hero and on the card.
    settings.safety.skip_signed = true;
    e.settings_set(SettingsSetRequest { settings }).unwrap();
    let l = list(e);
    assert_eq!(l.summary.total, with.summary.total - signed.candidate.saving());
    assert_eq!(l.summary.total, available(&l).iter().map(|o| o.saving).sum::<u64>(), "hero vs cards");
}

/// RF-8: the temp-folder rule of the app caches and the temporary-files detector both list
/// an old `.tmp` in `AppData/Local/Temp`. It is counted once, and applying a plan that holds
/// both cards removes it once: the step that does not count it leaves it alone instead of
/// failing on a file the other step already removed.
#[test]
fn a_file_two_steps_list_is_counted_and_removed_once() {
    let t = standard_tree();
    old(&t, "u/AppData/Local/Temp/setup.tmp", &bytes(30, 70_000), 5);
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let l = list(e);
    let tmp = f.tree.join("u/AppData/Local/Temp/setup.tmp");
    let by_path = selected_by_path(e, &l);
    let listed: Vec<String> = by_path[&tmp].iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(listed.len(), 2, "both detectors list the file: {listed:?}");
    assert_eq!(l.summary.total, available(&l).iter().map(|o| o.saving).sum::<u64>());
    let standalone: u64 = by_path.values().flatten().map(|(_, s)| s).sum();
    assert_eq!(l.summary.total, standalone - by_path[&tmp][0].1, "the file counts once");
    for id in &listed {
        add(e, id, None);
    }
    let plan = e.plan_get().unwrap();
    let counted: u64 = plan.steps.iter().map(|s| s.estimate.saving()).sum();
    assert_eq!(counted, plan.estimate.saving());
    assert_eq!(
        plan.steps.iter().filter(|s| s.estimate.files > 0).count(),
        1,
        "only one step counts the file: {:?}",
        plan.steps
    );
    let started = e.plan_apply(PlanApplyRequest::default()).unwrap();
    let jobs = wait_jobs(e);
    for j in jobs.iter().filter(|j| j.run_id == Some(started.run_id)) {
        assert_eq!(j.state, mj_core::JobState::Completed, "{j:?}");
    }
    assert!(!exists(&tmp));
}
