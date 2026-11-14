//! Optimise steps end to end through the engine facade, on generated fixtures: scan →
//! optimiser cards → plan → dry run → apply → undo, in replace mode (original to the
//! test's quarantine, undo restores the exact bytes) and light-copy mode (`_light`
//! beside, original untouched, undo removes the copy); keep-if-not-smaller, labelled
//! Office files, and the lossy confirmation gate (real ffmpeg when on PATH, else skipped).

mod common;

use common::fixtures;
use common::*;
use mj_core::testutil::{snapshot, TempDir};
use mj_core::{Action, JobState, OptimiserId, OutputMode};
use mj_engine::api::*;

fn card_id(id: OptimiserId) -> String {
    format!("opt:{}", serde_json::to_value(id).unwrap().as_str().unwrap())
}

fn rows(e: &mj_engine::Engine, id: &str) -> Vec<CandidateRow> {
    e.candidates_page(CandidatesPageRequest {
        opportunity_id: mj_core::OpportunityId(id.into()),
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

/// The five lossless optimisers and the file each one should pick up.
const CASES: [(OptimiserId, &str); 5] = [
    (OptimiserId::PngLossless, "img/a.png"),
    (OptimiserId::JpegLossless, "img/b.jpg"),
    (OptimiserId::ZipRepack, "books/c.epub"),
    (OptimiserId::PowerPoint, "decks/d.pptx"),
    (OptimiserId::PdfLossless, "docs/e.pdf"),
];

fn optimise_tree() -> TempDir {
    let t = TempDir::new("engine-opt-tree").unwrap();
    t.write("img/a.png", &fixtures::png()).unwrap();
    t.write("img/b.jpg", &fixtures::jpeg()).unwrap();
    t.write("books/c.epub", &fixtures::epub()).unwrap();
    t.write("decks/d.pptx", &fixtures::pptx(false)).unwrap();
    t.write("decks/labelled.pptx", &fixtures::pptx(true)).unwrap();
    t.write("docs/e.pdf", &fixtures::pdf()).unwrap();
    t
}

/// Scan, check every optimiser card lists its file (and only that one is selectable),
/// add one step per card with `action` derived from the card's default by `edit`.
fn plan_cases(f: &Fixture, edit: &dyn Fn(&mut Action)) {
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    for (id, rel) in CASES {
        let cid = card_id(id);
        let o = opp(e, &cid).unwrap_or_else(|| panic!("card {cid} missing"));
        assert!(o.availability.is_available(), "{cid}: {:?}", o.availability);
        let r = rows(e, &cid);
        let target = f.tree.join(rel);
        let row = r.iter().find(|r| r.candidate.path == target).unwrap_or_else(|| panic!("{rel} not in {cid}: {r:?}"));
        assert!(row.selected && row.actionable, "{rel}: {row:?}");
        let mut action = o.default_action.clone();
        edit(&mut action);
        e.plan_add_step(PlanAddStepRequest { opportunity_id: o.id.clone(), action: Some(action), params: None })
            .unwrap();
    }
    // The labelled deck is listed but locked: never selectable, never acted on.
    let ppt = rows(e, &card_id(OptimiserId::PowerPoint));
    let labelled =
        ppt.iter().find(|r| r.candidate.path == f.tree.join("decks/labelled.pptx")).expect("labelled deck listed");
    assert!(labelled.candidate.badges.contains(mj_core::Badges::LABELLED), "{labelled:?}");
    assert!(!labelled.actionable && !labelled.selected && labelled.blocked_reason.is_some(), "{labelled:?}");
    let plan = e.plan_get().unwrap();
    assert!(plan.checklist.iter().any(|c| c.code == "skipped_confidential"), "{:?}", plan.checklist);
}

fn dry_run(f: &Fixture) -> DryRunReport {
    let e = &f.engine;
    let job = e.plan_dry_run().unwrap();
    wait("dry run", || {
        f.events.events().into_iter().find_map(|ev| match ev {
            Event::DryRun(r) if r.job_id == job.id => Some(r),
            _ => None,
        })
    })
}

fn apply(f: &Fixture) -> mj_core::RunId {
    let e = &f.engine;
    let started = e.plan_apply(PlanApplyRequest::default()).unwrap();
    assert_eq!(started.jobs.len(), CASES.len());
    for j in wait_jobs(e).iter().filter(|j| j.run_id == Some(started.run_id)) {
        assert_eq!(j.state, JobState::Completed, "{j:?}");
        let o = j.outcome.as_ref().unwrap();
        assert_eq!((o.succeeded, o.failed, o.skipped), (1, 0, 0), "{}: {o:?}", j.title);
    }
    started.run_id
}

fn undo(f: &Fixture, run_id: mj_core::RunId) {
    let e = &f.engine;
    let job = e.undo_run(RunRequest { run_id }).unwrap();
    let jobs = wait_jobs(e);
    let u = jobs.iter().find(|j| j.id == job.id).unwrap();
    assert_eq!(u.state, JobState::Completed, "{u:?}");
    assert_eq!(u.outcome.as_ref().unwrap().failed, 0, "{u:?}");
}

#[test]
fn replace_mode_keeps_originals_restorable_and_undo_restores_exact_bytes() {
    let f = engine_for(optimise_tree());
    let before = snapshot(f.tree.path()).unwrap();
    let originals: Vec<Vec<u8>> = CASES.iter().map(|(_, rel)| std::fs::read(f.tree.join(rel)).unwrap()).collect();
    plan_cases(&f, &|_| {});

    let report = dry_run(&f);
    assert!(report.error.is_none(), "{report:?}");
    assert_eq!(report.total.confidence, mj_core::Confidence::Exact);
    assert_eq!(report.total.skipped, 0, "{report:?}");
    assert!(report.total.saving() > 0);
    assert_eq!(snapshot(f.tree.path()).unwrap(), before, "a dry run changes nothing");

    let run = apply(&f);
    for ((_, rel), orig) in CASES.iter().zip(&originals) {
        let now = std::fs::read(f.tree.join(rel)).unwrap();
        assert!(now.len() < orig.len(), "{rel} not smaller ({} vs {})", now.len(), orig.len());
    }
    assert_eq!(std::fs::read(f.tree.join("decks/labelled.pptx")).unwrap(), fixtures::pptx(true));
    let q = f.engine.quarantine_info().unwrap();
    assert_eq!(q.items, CASES.len() as u64, "originals held in the test quarantine: {q:?}");
    let history = f.engine.history_list(PageRequest::default()).unwrap();
    assert!(history.runs[0].undoable);
    let saved = f.engine.opportunities_list(OpportunitiesRequest::default()).unwrap().summary.reclaimed;
    assert!(saved > 0);

    undo(&f, run);
    assert_eq!(snapshot(f.tree.path()).unwrap(), before, "undo restores every original byte for byte");
    for ((_, rel), orig) in CASES.iter().zip(&originals) {
        assert_eq!(&std::fs::read(f.tree.join(rel)).unwrap(), orig, "{rel}");
    }
    assert_eq!(f.engine.quarantine_info().unwrap().items, 0);
}

#[test]
fn light_copy_mode_writes_beside_and_undo_removes_the_copies() {
    let f = engine_for(optimise_tree());
    let before = snapshot(f.tree.path()).unwrap();
    plan_cases(&f, &|a| {
        if let Action::Optimise { output, .. } = a {
            *output = OutputMode::LightCopy { suffix: "_light".into() };
        }
    });
    let plan = f.engine.plan_get().unwrap();
    assert!(plan.steps.iter().all(|s| !s.safety.requires_confirmation()));
    let run = apply(&f);
    for (_, rel) in CASES {
        let p = f.tree.join(rel);
        let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
        let ext = p.extension().unwrap().to_string_lossy().into_owned();
        let light = p.with_file_name(format!("{stem}_light.{ext}"));
        assert!(light.is_file(), "{} missing", light.display());
        assert!(std::fs::metadata(&light).unwrap().len() < std::fs::metadata(&p).unwrap().len());
    }
    // Originals untouched; nothing went to the quarantine.
    let now = snapshot(f.tree.path()).unwrap();
    for entry in &before {
        assert!(now.contains(entry), "{} changed", entry.0);
    }
    assert_eq!(f.engine.quarantine_info().unwrap().items, 0);
    undo(&f, run);
    assert_eq!(snapshot(f.tree.path()).unwrap(), before, "undo removes the light copies");
}

#[test]
fn keep_if_not_smaller_leaves_the_tree_unchanged() {
    // Entries already deflated at the strongest level: the cheap estimate promises a few
    // percent, the real repack wins nothing, so the item is skipped and nothing changes.
    let t = TempDir::new("engine-opt-tight").unwrap();
    let body = fixtures::prose(21, 40_000);
    let mut w = mj_formats::zip::ZipWriter::new(Vec::new());
    w.add("mimetype", b"application/epub+zip", 0, 0x6000, 0x5821).unwrap();
    w.add("OEBPS/ch1.xhtml", body.as_bytes(), 10, 0x6000, 0x5821).unwrap();
    t.write("tight.epub", &w.finish().unwrap()).unwrap();
    let f = engine_for(t);
    let before = snapshot(f.tree.path()).unwrap();
    assert_eq!(scan(&f.engine).state, ScanRunState::Done);
    let cid = card_id(OptimiserId::ZipRepack);
    let o = opp(&f.engine, &cid).expect("listed on the estimate");
    f.engine.plan_add_step(PlanAddStepRequest { opportunity_id: o.id, action: None, params: None }).unwrap();
    let report = dry_run(&f);
    assert_eq!(report.total.skipped, 1, "{report:?}");
    assert_eq!(report.total.saving(), 0);
    let started = f.engine.plan_apply(PlanApplyRequest::default()).unwrap();
    let jobs = wait_jobs(&f.engine);
    let j = jobs.iter().find(|j| j.run_id == Some(started.run_id)).unwrap();
    let out = j.outcome.as_ref().unwrap();
    assert_eq!((out.succeeded, out.skipped), (0, 1), "{out:?}");
    assert!(out.errors[0].error.as_ref().unwrap().message.contains("not smaller"), "{out:?}");
    assert_eq!(snapshot(f.tree.path()).unwrap(), before);
    assert_eq!(f.engine.quarantine_info().unwrap().items, 0);
    // No temporary output left behind anywhere in the tree.
    assert!(!snapshot(f.tree.path()).unwrap().iter().any(|(p, _, _)| p.contains(".mj-")));
}

fn ffmpeg_on_path() -> bool {
    std::process::Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn lossy_step_is_refused_without_confirmation_and_runs_with_it() {
    if !ffmpeg_on_path() {
        eprintln!("ffmpeg not on PATH: the lossy gate is covered by mj-actions' fake optimiser tests");
        return;
    }
    let t = TempDir::new("engine-opt-audio").unwrap();
    let wav = fixtures::wav(12);
    t.write("rec/meeting.wav", &wav).unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    let before = snapshot(f.tree.path()).unwrap();
    assert_eq!(scan(e).state, ScanRunState::Done);
    let cid = card_id(OptimiserId::Audio);
    let o = opp(e, &cid).unwrap_or_else(|| {
        panic!(
            "{cid} missing: {:?}",
            e.opportunities_list(OpportunitiesRequest::default())
                .unwrap()
                .items
                .iter()
                .map(|o| (&o.id, &o.availability))
                .collect::<Vec<_>>()
        )
    });
    let speech = o.presets.iter().find(|p| p.id == "speech").expect("speech preset").clone();
    let plan = e
        .plan_add_step(PlanAddStepRequest {
            opportunity_id: o.id.clone(),
            action: None,
            params: Some(speech.values.clone()),
        })
        .unwrap();
    let step = &plan.steps[0];
    assert_eq!(step.safety, mj_core::SafetyClass::LossyOptIn, "{step:?}");
    // The optimiser receives the preset's values (not the card defaults).
    match &step.action {
        Action::Optimise { params, .. } => {
            for (k, v) in &speech.values.0 {
                assert_eq!(params.0.get(k), Some(v), "{k}");
            }
        }
        a => panic!("{a:?}"),
    }
    let err = e.plan_apply(PlanApplyRequest::default()).unwrap_err();
    assert_eq!(err.code(), "confirmation_required");
    assert!(e.jobs_list().unwrap().is_empty(), "nothing ran");
    assert_eq!(snapshot(f.tree.path()).unwrap(), before);

    let conf = mj_core::StepConfirmation {
        step_id: step.id,
        acknowledged: true,
        originals: Default::default(),
        synced_online_only: false,
    };
    let started = e.plan_apply(PlanApplyRequest { confirmations: vec![conf] }).unwrap();
    let jobs = wait_jobs(e);
    let j = jobs.iter().find(|j| j.run_id == Some(started.run_id)).unwrap();
    assert_eq!(j.outcome.as_ref().unwrap().succeeded, 1, "{j:?}");
    // The conversion is named with its own extension; the original went to the quarantine.
    let wav_path = f.tree.join("rec/meeting.wav");
    let opus = f.tree.join("rec/meeting.opus");
    assert!(!exists(&wav_path), "the original was retained");
    assert!(opus.is_file(), "converted file named .opus");
    assert!(std::fs::metadata(&opus).unwrap().len() < wav.len() as u64);
    assert_eq!(&std::fs::read(&opus).unwrap()[..4], b"OggS");
    let entry = e.history_run_detail(RunDetailRequest { run_id: started.run_id, offset: 0, limit: 10 }).unwrap();
    assert!(entry.entries.iter().any(|x| x.lossy), "{entry:?}");
    undo(&f, started.run_id);
    assert_eq!(snapshot(f.tree.path()).unwrap(), before, "undo brings the WAV back and removes the Opus file");
}
