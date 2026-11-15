//! Regression tests for the round-1 review findings of the action core (W6-B). Each test
//! reproduces the reviewed scenario on a fixture tree inside its own `TempDir`; names start
//! with the finding id.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use mj_actions::Services;
use mj_core::{Action, ArchiveFormat, ArchiveJob, Archiver, ExternalTool, OriginalsPolicy, RunKind};

/// Writes a stand-in archive and accepts it on verification.
struct FakeArchiver;

impl Archiver for FakeArchiver {
    fn formats(&self, _: &[ExternalTool]) -> Vec<ArchiveFormat> {
        vec![ArchiveFormat::Zip]
    }
    fn create(&self, job: &ArchiveJob<'_>) -> mj_core::Result<u64> {
        std::fs::write(job.dst, b"complete verified archive").unwrap();
        Ok(25)
    }
    fn verify(&self, _: &Path, _: &[PathBuf], _: &Path, _: &mj_core::CancelToken) -> mj_core::Result<()> {
        Ok(())
    }
}

fn archive_removing_sources() -> Action {
    Action::Archive {
        format: ArchiveFormat::Zip,
        level: 6,
        dest: None,
        originals: OriginalsPolicy::RecycleBin,
        keep_sources: false,
    }
}

/// ACT-1: the removal of an archived folder fails after part of it is gone: the archive,
/// now the only complete copy, is kept and the error names it.
#[test]
fn act1_archive_is_kept_when_the_sources_were_partly_removed() {
    let mut f = Fixture::new("act1-partial");
    f.write("proj/a.txt", b"a");
    f.write("proj/b.txt", b"b");
    f.platform.partial_trash = true;
    let dir = f.root().join("proj");
    let services = Services { optimisers: &[], archiver: Some(&FakeArchiver), tools: &[] };
    let run = f.new_run(RunKind::Apply);
    let item = f.item(&dir, archive_removing_sources());
    let out = mj_actions::execute_with(&f.ctx(run, services), &[item], &confirmed()).unwrap();
    assert_eq!(out.succeeded, 0, "{out:?}");
    let zip = f.root().join("proj.zip");
    assert!(zip.exists(), "the only complete copy was deleted");
    let err = out.errors[0].error.as_ref().unwrap();
    assert!(err.message.contains("proj.zip"), "{err:?}");
    let e = f.journal.entries_of(run).pop().unwrap();
    assert!(e.error.as_ref().unwrap().message.contains("proj.zip"), "{e:?}");
}

/// ACT-1 control: a removal that fails without touching the source still removes the
/// archive (nothing was lost, the run leaves nothing behind).
#[test]
fn act1_archive_is_removed_when_the_sources_are_untouched() {
    let mut f = Fixture::new("act1-whole");
    f.write("proj/a.txt", b"a");
    f.platform.fail_trash_at = Some(1);
    let dir = f.root().join("proj");
    let before = snapshot(&dir);
    let services = Services { optimisers: &[], archiver: Some(&FakeArchiver), tools: &[] };
    let run = f.new_run(RunKind::Apply);
    let out =
        mj_actions::execute_with(&f.ctx(run, services), &[f.item(&dir, archive_removing_sources())], &confirmed())
            .unwrap();
    assert_eq!(out.succeeded, 0, "{out:?}");
    assert!(!f.root().join("proj.zip").exists());
    assert_eq!(snapshot(&dir), before);
}

/// ACT-4: undoing a file copy whose original is gone (or changed) keeps the copy: it may be
/// the only one left.
#[test]
fn act4_undo_of_a_copy_keeps_it_when_the_original_is_gone() {
    let f = Fixture::new("act4");
    let a = f.write("a.txt", b"only data");
    let b = f.write("b.txt", b"still here");
    let c = f.write("c.txt", b"will change");
    let dest = f.t.join("backup");
    let items: Vec<_> =
        [&a, &b, &c].iter().map(|p| f.item(p, Action::Copy { dest: dest.clone(), keep_structure: false })).collect();
    let (run, out) = f.apply(&items, &opts());
    assert_eq!(out.succeeded, 3, "{out:?}");
    std::fs::remove_file(&a).unwrap();
    std::fs::write(&c, b"changed later").unwrap();
    let undo = f.undo_run(run);
    assert_eq!(undo.succeeded, 1, "{undo:?}");
    assert_eq!(undo.errors.len(), 2, "{undo:?}");
    for e in &undo.errors {
        assert_eq!(e.error.as_ref().unwrap().code, "conflict", "{e:?}");
    }
    assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"only data");
    assert_eq!(std::fs::read(dest.join("c.txt")).unwrap(), b"will change");
    assert!(!dest.join("b.txt").exists(), "a copy of an unchanged original is still removed");
}

/// Writes the first half of the source.
struct Halve;

impl mj_core::Optimiser for Halve {
    fn info(&self) -> mj_core::OptimiserInfo {
        mj_core::OptimiserInfo {
            id: mj_core::OptimiserId::PngLossless,
            title: "halve".into(),
            description: String::new(),
            category: mj_core::Category::Optimise,
            safety: mj_core::SafetyClass::Reversible,
            default_action: mj_core::ActionKind::Optimise,
            available_actions: vec![],
            presets: vec![],
            settings: vec![],
            platform: mj_core::PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec![],
            unit: String::new(),
        }
    }
    fn availability(&self, _: &[ExternalTool]) -> mj_core::Availability {
        mj_core::Availability::Available
    }
    fn applies_to(&self, _: &mj_core::FileEntry) -> bool {
        true
    }
    fn estimate(&self, e: &mj_core::FileEntry, _: &mj_core::ParamValues) -> mj_core::Result<mj_core::Estimate> {
        Ok(mj_core::Estimate { files: 1, bytes_before: e.size, bytes_after: e.size / 2, ..Default::default() })
    }
    fn optimise(&self, job: &mj_core::OptimiseJob<'_>) -> mj_core::Result<mj_core::OptimiseOutput> {
        let d = std::fs::read(job.src).unwrap();
        std::fs::write(job.dst, &d[..d.len() / 2]).unwrap();
        Ok(mj_core::OptimiseOutput { bytes_before: d.len() as u64, ..mj_core::OptimiseOutput::default() })
    }
    fn analyse(&self, _: &mj_core::DetectContext<'_>) -> mj_core::Result<mj_core::DetectorOutput> {
        Ok(mj_core::DetectorOutput::default())
    }
}

fn make_read_only(p: &Path) {
    let mut perm = std::fs::metadata(p).unwrap().permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(p, perm).unwrap();
}

fn mtime(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p).unwrap().modified().unwrap()
}

/// ACT-3: a read-only file can be optimised (output read-only too, mtime kept), and a
/// read-only duplicate replaced by a hard link separates again on undo. On Linux this only
/// fails for a non-root user (root ignores the mode); on Windows the read-only attribute
/// refuses a writable handle to anyone, so the Windows CI job checks it unconditionally.
#[test]
fn act3_read_only_files_can_be_optimised_and_unlinked() {
    let f = Fixture::new("act3");
    let photo = f.write("photo.png", b"0123456789abcdefghij");
    make_read_only(&photo);
    let before = mtime(&photo);
    let optimisers: Vec<Box<dyn mj_core::Optimiser>> = vec![Box::new(Halve)];
    let services = Services { optimisers: &optimisers, archiver: None, tools: &[] };
    let action = Action::Optimise {
        optimiser: mj_core::OptimiserId::PngLossless,
        params: mj_core::ParamValues::default(),
        output: mj_core::OutputMode::Replace,
        originals: OriginalsPolicy::Quarantine,
        keep_if_not_smaller: true,
    };
    let run = f.new_run(RunKind::Apply);
    let out = mj_actions::execute_with(&f.ctx(run, services), &[f.item(&photo, action)], &confirmed()).unwrap();
    assert_eq!(out.succeeded, 1, "{out:?}");
    assert_eq!(std::fs::read(&photo).unwrap(), b"0123456789");
    assert!(std::fs::metadata(&photo).unwrap().permissions().readonly(), "permissions preserved");
    assert_eq!(mtime(&photo), before, "mtime preserved");

    let keeper = f.write("a/keep.txt", b"same text in both");
    let dup = f.write("b/copy.txt", b"same text in both");
    make_read_only(&dup);
    let mut item = f.item(&dup, Action::Hardlink);
    item.link_source = Some(keeper.clone());
    let (run, out) = f.apply(std::slice::from_ref(&item), &confirmed());
    assert_eq!(out.succeeded, 1, "{out:?}");
    let undo = f.undo_run(run);
    assert_eq!(undo.succeeded, 1, "{undo:?}");
    assert_eq!(std::fs::read(&dup).unwrap(), b"same text in both");
    assert!(std::fs::metadata(&dup).unwrap().permissions().readonly(), "mode restored");
    // Separate again: an edit of the keeper no longer shows through the duplicate.
    let mut perm = std::fs::metadata(&keeper).unwrap().permissions();
    perm.set_readonly(false);
    let _ = std::fs::set_permissions(&keeper, perm);
    std::fs::write(&keeper, b"edited").unwrap();
    assert_eq!(std::fs::read(&dup).unwrap(), b"same text in both");
}
