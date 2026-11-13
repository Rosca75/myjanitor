//! Crash recovery (dangling `intent` records), the on-disk journal across restarts,
//! and quarantine purge with a caller-supplied "now".

mod common;

use std::path::{Path, PathBuf};

use common::*;
use mj_actions::journal::ENTRIES_PER_RUN;
use mj_core::{
    Action, ActionKind, EntryId, FileStamp, JobId, JournalEntry, JournalState, RunId, RunKind, TargetKind, Timestamp,
};

fn intent(run: RunId, idx: u64, action: ActionKind, original: &Path, stamp: FileStamp) -> JournalEntry {
    JournalEntry {
        id: EntryId(run.0 * ENTRIES_PER_RUN + idx),
        run_id: run,
        job_id: JobId(1),
        step_id: None,
        at: Timestamp::now(),
        state: JournalState::Intent,
        action,
        target: TargetKind::File,
        original: original.to_path_buf(),
        original_stamp: stamp,
        original_sha256: None,
        bytes_before: stamp.size,
        bytes_after: stamp.size,
        retained_at: None,
        trash: None,
        created: vec![],
        link_source: None,
        reversible: false,
        undo_deadline: None,
        lossy: false,
        error: None,
    }
}

fn stamp(p: &Path) -> FileStamp {
    mj_actions::fsops::stamp_of(p).unwrap()
}

#[test]
fn dangling_intents_are_settled_by_inspection() {
    let f = Fixture::new("recover");
    let run = f.new_run(RunKind::Apply);
    let a = f.write("a.txt", b"not moved");
    let b = f.write("b.txt", b"moved");
    let c = f.write("c.txt", b"both");
    let keeper = f.write("k.bin", b"same content");
    let dup = f.write("d.bin", b"same content");
    let dup2 = f.write("d2.bin", b"same content");
    let gone = f.write("gone.txt", b"deleted");
    let copied = f.write("copied.txt", b"copy me");

    let slot = |i: u64, name: &str| {
        mj_actions::quarantine::slot(&f.qdir, run, EntryId(run.0 * ENTRIES_PER_RUN + i), name.as_ref())
    };
    // 0: quarantine, crash before the move.
    let mut e0 = intent(run, 0, ActionKind::Quarantine, &a, stamp(&a));
    e0.retained_at = Some(slot(0, "a.txt"));
    // 1: quarantine, crash after the move.
    let mut e1 = intent(run, 1, ActionKind::Quarantine, &b, stamp(&b));
    e1.retained_at = Some(slot(1, "b.txt"));
    std::fs::create_dir_all(slot(1, "b.txt").parent().unwrap()).unwrap();
    std::fs::rename(&b, slot(1, "b.txt")).unwrap();
    // 2: quarantine across volumes, crash between copy and delete: both exist.
    let mut e2 = intent(run, 2, ActionKind::Quarantine, &c, stamp(&c));
    e2.retained_at = Some(slot(2, "c.txt"));
    std::fs::create_dir_all(slot(2, "c.txt").parent().unwrap()).unwrap();
    std::fs::copy(&c, slot(2, "c.txt")).unwrap();
    // 3: hard link made under the temp name, rename not done.
    let mut e3 = intent(run, 3, ActionKind::Hardlink, &dup, stamp(&dup));
    let temp3 = mj_actions::engine::temp_beside(&dup, e3.id, false);
    std::fs::hard_link(&keeper, &temp3).unwrap();
    e3.created = vec![temp3.clone()];
    e3.link_source = Some(keeper.clone());
    // 4: hard link completed (renamed over the duplicate), done record missing.
    let mut e4 = intent(run, 4, ActionKind::Hardlink, &dup2, stamp(&dup2));
    let temp4 = mj_actions::engine::temp_beside(&dup2, e4.id, false);
    std::fs::hard_link(&keeper, &temp4).unwrap();
    std::fs::rename(&temp4, &dup2).unwrap();
    e4.created = vec![temp4];
    e4.link_source = Some(keeper.clone());
    // 5: permanent delete that happened.
    let e5 = intent(run, 5, ActionKind::PermanentDelete, &gone, stamp(&gone));
    std::fs::remove_file(&gone).unwrap();
    // 6: a complete copy.
    let mut e6 = intent(run, 6, ActionKind::Copy, &copied, stamp(&copied));
    let copy_dest = f.t.join("copies/copied.txt");
    e6.created = vec![copy_dest.clone()];
    std::fs::create_dir_all(copy_dest.parent().unwrap()).unwrap();
    std::fs::copy(&copied, &copy_dest).unwrap();
    // 7: plain trash, nothing happened.
    let e7 = intent(run, 7, ActionKind::Trash, &copied, stamp(&copied));
    for e in [&e0, &e1, &e2, &e3, &e4, &e5, &e6, &e7] {
        f.journal.append(e).unwrap();
    }

    // "Restart": reopen the journal from disk and recover.
    let j = mj_actions::Journal::open(&f.t.join("journal")).unwrap();
    assert_eq!(j.pending_intents().len(), 8);
    let mut ctx = f.ctx(run, no_services());
    ctx.journal = &j;
    let changed = mj_actions::undo::recover(&ctx).unwrap();
    assert_eq!(changed.len(), 8);
    assert!(j.pending_intents().is_empty());
    let st = |e: &JournalEntry| j.entry(e.id).unwrap();
    assert_eq!(st(&e0).state, JournalState::Failed);
    assert!(a.exists());
    assert_eq!(st(&e1).state, JournalState::Done);
    assert!(st(&e1).reversible);
    assert_eq!(st(&e2).state, JournalState::Failed);
    assert!(c.exists() && slot(2, "c.txt").exists(), "recovery never deletes user data");
    assert!(st(&e2).error.unwrap().message.contains("incomplete copy"));
    assert_eq!(st(&e3).state, JournalState::Failed);
    assert!(!temp3.exists(), "our own temporary link name is removed");
    assert_eq!(std::fs::read(&dup).unwrap(), b"same content");
    assert_eq!(st(&e4).state, JournalState::Done);
    assert!(st(&e4).created.is_empty());
    assert_eq!(st(&e5).state, JournalState::Done);
    assert!(!st(&e5).reversible);
    assert_eq!(st(&e6).state, JournalState::Done);
    assert_eq!(st(&e7).state, JournalState::Failed);

    // The recovered quarantine entry can be undone.
    let u = mj_actions::undo::start_undo_run(&j, Some(run), "Undo", Timestamp::now()).unwrap();
    let mut uctx = f.ctx(u.run_id, no_services());
    uctx.journal = &j;
    let out = mj_actions::undo::undo(&uctx, &[e1.id, e6.id]).unwrap();
    assert_eq!(out.succeeded, 2, "{:?}", out.errors);
    assert_eq!(std::fs::read(&b).unwrap(), b"moved");
    assert!(!copy_dest.exists());
}

#[test]
fn interrupted_undo_is_settled() {
    let f = Fixture::new("recover-undo");
    let p = f.write("a.txt", b"alpha");
    let (run, out) = f.apply(&[f.item(&p, Action::Quarantine { purge_after_days: 30 })], &opts());
    assert_eq!(out.succeeded, 1);
    let src = f.journal.entries_of(run)[0].clone();
    // Simulate an undo that restored the file and then crashed before its records.
    let u = mj_actions::undo::start_undo_run(&f.journal, Some(run), "Undo", Timestamp::now()).unwrap();
    let mut ue = src.clone();
    ue.id = EntryId(u.run_id.0 * ENTRIES_PER_RUN);
    ue.run_id = u.run_id;
    ue.state = JournalState::Intent;
    ue.reversible = false;
    f.journal.append(&ue).unwrap();
    std::fs::rename(src.retained_at.as_ref().unwrap(), &p).unwrap();
    mj_actions::undo::recover(&f.ctx(u.run_id, no_services())).unwrap();
    assert_eq!(f.journal.entry(src.id).unwrap().state, JournalState::Undone);
    assert_eq!(f.journal.entry(ue.id).unwrap().state, JournalState::Done);

    // An undo intent where nothing happened leaves the source undoable.
    let f = Fixture::new("recover-undo2");
    let p = f.write("a.txt", b"alpha");
    let (run, _) = f.apply(&[f.item(&p, Action::Quarantine { purge_after_days: 30 })], &opts());
    let src = f.journal.entries_of(run)[0].clone();
    let u = mj_actions::undo::start_undo_run(&f.journal, Some(run), "Undo", Timestamp::now()).unwrap();
    let mut ue = src.clone();
    ue.id = EntryId(u.run_id.0 * ENTRIES_PER_RUN);
    ue.run_id = u.run_id;
    ue.state = JournalState::Intent;
    f.journal.append(&ue).unwrap();
    mj_actions::undo::recover(&f.ctx(u.run_id, no_services())).unwrap();
    assert_eq!(f.journal.entry(src.id).unwrap().state, JournalState::Done);
    assert_eq!(f.journal.entry(ue.id).unwrap().state, JournalState::Failed);
    assert_eq!(mj_actions::undo::undoable_entries(&f.journal, run, Timestamp::now()), vec![src.id]);
}

#[test]
fn journal_survives_restart_and_runs_are_listed() {
    let f = Fixture::new("persist");
    tree3(&f);
    let (run, _) = f.apply(&[f.item(&f.root().join("x.txt"), Action::Trash)], &opts());
    let j = mj_actions::Journal::open(&f.t.join("journal")).unwrap();
    let e = &j.entries_of(run)[0];
    assert_eq!(e.state, JournalState::Done);
    assert!(e.trash.as_ref().unwrap().info.as_ref().unwrap().exists());
    assert!(j.runs().iter().any(|r| r.run_id == run));
    // Every intent line precedes its done line on disk.
    let file = std::fs::read_dir(f.t.join("journal"))
        .unwrap()
        .flatten()
        .find(|d| d.file_name().to_string_lossy().starts_with("entries-"))
        .unwrap()
        .path();
    let text = std::fs::read_to_string(file).unwrap();
    let id = format!("\"id\":{}", e.id.0);
    let lines: Vec<&str> = text.lines().filter(|l| l.contains(&id)).collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("\"state\":\"intent\"") && lines[1].contains("\"state\":\"done\""));
}

fn tree3(f: &Fixture) {
    f.write("x.txt", b"x");
}

#[test]
fn quarantine_purge_uses_the_callers_now() {
    let f = Fixture::new("purge");
    let a = f.write("a.bin", &[1u8; 1000]);
    let d = f.write("d/b.bin", &[2u8; 500]);
    let (run, out) = f.apply(
        &[
            f.item(&a, Action::Quarantine { purge_after_days: 30 }),
            f.item(d.parent().unwrap(), Action::Quarantine { purge_after_days: 30 }),
        ],
        &opts(),
    );
    assert_eq!(out.succeeded, 2);
    let done_at = f.journal.entries_of(run).iter().map(|e| e.at).max().unwrap();
    let s = mj_actions::quarantine::summary(&f.journal, &f.qdir);
    assert_eq!((s.items, s.bytes), (2, 1500));
    assert_eq!(mj_actions::quarantine::purge(&f.journal, &f.qdir, done_at.add_days(29), 30).unwrap(), 0);
    assert_eq!(mj_actions::quarantine::summary(&f.journal, &f.qdir).items, 2);
    let freed = mj_actions::quarantine::purge(&f.journal, &f.qdir, done_at.add_days(31), 30).unwrap();
    assert_eq!(freed, 1500);
    assert!(f.journal.entries_of(run).iter().all(|e| e.state == JournalState::Purged && !e.reversible));
    assert!(std::fs::read_dir(&f.qdir).unwrap().next().is_none(), "slots, manifests and run folders removed");
    assert!(mj_actions::undo::undoable_entries(&f.journal, run, Timestamp::now()).is_empty());
    // A purge never deletes outside the quarantine, even with a forged journal entry.
    let outside: PathBuf = f.write("keep.txt", b"keep");
    let mut forged = f.journal.entries_of(run)[0].clone();
    forged.id = EntryId(forged.id.0 + 10);
    forged.state = JournalState::Done;
    forged.retained_at = Some(f.qdir.join("..").join("root").join("keep.txt"));
    f.journal.append(&forged).unwrap();
    mj_actions::quarantine::purge(&f.journal, &f.qdir, done_at.add_days(400), 30).unwrap();
    assert!(outside.exists());
}
