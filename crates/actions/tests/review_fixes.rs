//! Regression tests for the findings of `docs/review/adversarial-review-1.md` that the
//! action engine owns. Each test reproduces the reviewed scenario on a fixture tree inside
//! its own `TempDir` and asserts the safe behaviour. Names start with the finding id.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use mj_actions::{Guard, ProtectedPaths, Services};
use mj_core::{
    Action, ArchiveFormat, ArchiveJob, Archiver, ExternalTool, ItemStatus, JournalState, OriginalsPolicy, RunKind,
};

/// An archiver that is never reached (the refusals happen before).
struct NoArchiver;

impl Archiver for NoArchiver {
    fn formats(&self, _: &[ExternalTool]) -> Vec<ArchiveFormat> {
        vec![ArchiveFormat::Zip]
    }
    fn create(&self, _: &ArchiveJob<'_>) -> mj_core::Result<u64> {
        panic!("the archiver must not run for a refused item");
    }
    fn verify(&self, _: &Path, _: &[PathBuf], _: &Path, _: &mj_core::CancelToken) -> mj_core::Result<()> {
        Ok(())
    }
}

/// Every folder action the engine has, against `dir`.
fn folder_actions(f: &Fixture) -> Vec<Action> {
    vec![
        Action::Trash,
        Action::Quarantine { purge_after_days: 30 },
        Action::PermanentDelete,
        Action::Move { dest: f.t.join("moved"), keep_structure: false },
        Action::Archive {
            format: ArchiveFormat::Zip,
            level: 6,
            dest: None,
            originals: OriginalsPolicy::Quarantine,
            keep_sources: false,
        },
        Action::Dehydrate,
    ]
}

/// Run every folder action on `dir` and assert each one is skipped with `code` and the
/// tree is untouched.
fn assert_every_folder_action_refused(f: &Fixture, dir: &Path, code: &str, owned: bool) {
    let before = snapshot(&f.root());
    let archiver = NoArchiver;
    let services = Services { optimisers: &[], archiver: Some(&archiver), tools: &[] };
    for action in folder_actions(f) {
        let mut item = f.item(dir, action.clone());
        item.owned_tree = owned;
        let run = f.new_run(RunKind::Apply);
        let out = mj_actions::execute_with(&f.ctx(run, services), &[item], &confirmed()).unwrap();
        assert_eq!(out.succeeded, 0, "{action:?} acted: {out:?}");
        assert_eq!(out.skipped, 1, "{action:?}: {out:?}");
        let err = out.errors[0].error.as_ref().unwrap();
        assert_eq!(err.code, code, "{action:?}: {err:?}");
        assert_eq!(snapshot(&f.root()), before, "{action:?} changed the tree");
        assert!(!f.t.join("moved").exists());
    }
}

#[test]
fn f01_folder_holding_a_user_protected_folder_is_refused_for_every_action() {
    let mut f = Fixture::new("f01-user");
    let proj = f.root().join("old-project");
    f.write("old-project/contracts/signed-contract.pdf", b"%PDF keep me");
    f.write("old-project/notes.txt", b"notes");
    f.guard = Guard::new(
        vec![f.root()],
        ProtectedPaths::empty().with_app_dirs(&[f.qdir.clone()]).with_user_folders(&[proj.join("contracts")]),
    );
    assert_every_folder_action_refused(&f, &proj, "protected", false);
}

#[test]
fn f01_folder_holding_a_user_pattern_is_refused() {
    let mut f = Fixture::new("f01-pattern");
    let proj = f.root().join("clients");
    f.write("clients/acme/Invoices/2024.pdf", b"%PDF");
    f.guard = Guard::new(
        vec![f.root()],
        ProtectedPaths::empty().with_app_dirs(&[f.qdir.clone()]).with_patterns(&["**/Invoices/**".into()]),
    );
    assert_every_folder_action_refused(&f, &proj, "protected", false);
}

#[test]
fn f01_project_folder_holding_git_or_credentials_is_refused() {
    let mut f = Fixture::new("f01-vcs");
    f.guard = Guard::new(vec![f.root()], ProtectedPaths::new(&[]).with_app_dirs(&[f.qdir.clone()]));
    f.write("project/src/main.rs", b"fn main() {}");
    f.write("project/.git/config", b"[core]");
    assert_every_folder_action_refused(&f, &f.root().join("project"), "protected", false);
    f.write("home-copy/.ssh/id_ed25519", b"-----BEGIN OPENSSH PRIVATE KEY-----");
    assert_every_folder_action_refused(&f, &f.root().join("home-copy"), "protected", false);
    f.write("cfg/.aws/credentials", b"[default]");
    assert_every_folder_action_refused(&f, &f.root().join("cfg"), "protected", false);
    // Even a regenerable artefact folder may not take a credential store with it.
    f.write("node_modules/evil/.ssh/id_rsa", b"key");
    assert_every_folder_action_refused(&f, &f.root().join("node_modules"), "protected", true);
}

#[test]
fn f01_owned_artefact_folder_may_hold_vcs_metadata() {
    let mut f = Fixture::new("f01-owned");
    f.guard = Guard::new(vec![f.root()], ProtectedPaths::new(&[]).with_app_dirs(&[f.qdir.clone()]));
    f.write("node_modules/dep/index.js", b"module.exports = 1");
    f.write("node_modules/gitdep/.git/HEAD", b"ref: refs/heads/main");
    let dir = f.root().join("node_modules");
    // Not owned (a generic folder action): refused.
    assert_every_folder_action_refused(&f, &dir, "protected", false);
    // Owned by the dev-artefacts detector: trashed as a whole, and undone exactly.
    let before = snapshot(&f.root());
    let mut item = f.item(&dir, Action::Quarantine { purge_after_days: 30 });
    item.owned_tree = true;
    let (run, out) = f.apply(&[item], &opts());
    assert_eq!(out.succeeded, 1, "{out:?}");
    assert!(!dir.exists());
    assert_eq!(f.undo_run(run).succeeded, 1);
    assert_eq!(snapshot(&f.root()), before);
}

#[test]
fn f01_folder_holding_an_app_folder_is_refused_even_lexically() {
    let mut f = Fixture::new("f01-app");
    f.write("share/other-app/data.db", b"db");
    f.write("share/myjanitor/journal/entries-2026-11.jsonl", b"{}");
    let app = f.root().join("share/myjanitor");
    f.guard = Guard::new(vec![f.root()], ProtectedPaths::empty().with_app_dirs(&[f.qdir.clone(), app.clone()]));
    assert_every_folder_action_refused(&f, &f.root().join("share"), "protected", false);
    // The lexical rule also covers an app folder that does not exist (yet).
    let ghost = f.root().join("cache-parent/myjanitor-cache");
    f.write("cache-parent/x.txt", b"x");
    f.guard = Guard::new(vec![f.root()], ProtectedPaths::empty().with_app_dirs(&[f.qdir.clone(), ghost]));
    assert_every_folder_action_refused(&f, &f.root().join("cache-parent"), "protected", false);
}

#[test]
fn f01_dry_run_reports_the_refusal() {
    let mut f = Fixture::new("f01-dry");
    f.guard = Guard::new(vec![f.root()], ProtectedPaths::new(&[]).with_app_dirs(&[f.qdir.clone()]));
    f.write("project/.git/config", b"[core]");
    let item = f.item(&f.root().join("project"), Action::Trash);
    let r = mj_actions::dry_run(&f.ctx(mj_core::RunId(0), no_services()), &[item]).unwrap();
    assert_eq!(r.estimate.files, 0);
    assert_eq!(r.skipped.len(), 1);
    assert_eq!(r.skipped[0].status, ItemStatus::Skipped);
    assert_eq!(r.skipped[0].error.as_ref().unwrap().code, "protected");
    let _ = JournalState::Skipped;
}

fn keep_of(f: &Fixture, path: &Path, identical: bool) -> mj_core::KeepCopy {
    let md = mj_platform::ScanPlatform::metadata(&f.platform, path).unwrap();
    mj_core::KeepCopy { path: path.to_path_buf(), expected: md.stamp(), expected_id: md.file_id, identical }
}

#[test]
fn f02_engine_refuses_to_remove_a_copy_whose_kept_copy_is_gone_or_different() {
    for action in [Action::Trash, Action::Quarantine { purge_after_days: 30 }, Action::PermanentDelete] {
        let f = Fixture::new("f02");
        let a = f.write("a/photo.jpg", &[9u8; 4000]);
        let b = f.write("b/photo.jpg", &[9u8; 4000]);
        // 1. The kept copy is itself the item: refused.
        let mut item = f.item(&a, action.clone());
        item.keep = Some(keep_of(&f, &a, true));
        let (_, out) = f.apply(&[item], &confirmed());
        assert_eq!((out.succeeded, out.skipped), (0, 1), "{action:?}: {out:?}");
        // 2. Both copies in one batch, each keeping the other: the second is refused
        //    because its kept copy is gone by then.
        let mut ia = f.item(&a, action.clone());
        ia.keep = Some(keep_of(&f, &b, true));
        let mut ib = f.item(&b, action.clone());
        ib.keep = Some(keep_of(&f, &a, true));
        let (_, out) = f.apply(&[ia, ib], &confirmed());
        assert_eq!((out.succeeded, out.skipped), (1, 1), "{action:?}: {out:?}");
        assert_eq!(out.errors[0].error.as_ref().unwrap().code, "conflict");
        assert!(a.exists() != b.exists(), "{action:?}: exactly one copy survives");
    }
}

#[test]
fn f02_engine_verifies_the_kept_copy_content_right_before_removing() {
    let f = Fixture::new("f02-content");
    let a = f.write("a/doc.bin", &[1u8; 4000]);
    let b = f.write("b/doc.bin", &[1u8; 4000]);
    let mut item = f.item(&b, Action::PermanentDelete);
    item.keep = Some(keep_of(&f, &a, true));
    // The kept copy was edited after the scan (same size): the duplicate is no longer a
    // duplicate and must stay.
    std::fs::write(&a, [2u8; 4000]).unwrap();
    let (_, out) = f.apply(std::slice::from_ref(&item), &confirmed());
    assert_eq!(out.skipped, 1, "{out:?}");
    assert!(b.exists());
    // A hard link to the item is not a second copy.
    std::fs::remove_file(&a).unwrap();
    std::fs::hard_link(&b, &a).unwrap();
    let mut item = f.item(&b, Action::PermanentDelete);
    item.keep = Some(keep_of(&f, &a, true));
    let (_, out) = f.apply(&[item], &confirmed());
    assert_eq!(out.skipped, 1, "{out:?}");
    assert!(b.exists());
    // The kept copy is fine: removal proceeds.
    std::fs::remove_file(&a).unwrap();
    std::fs::write(&a, [1u8; 4000]).unwrap();
    let mut item = f.item(&b, Action::PermanentDelete);
    item.keep = Some(keep_of(&f, &a, true));
    let (_, out) = f.apply(&[item], &confirmed());
    assert_eq!(out.succeeded, 1, "{out:?}");
    assert!(!b.exists() && a.exists());
}

#[test]
fn f02_non_identical_group_keeps_a_copy_that_still_matches_its_scan() {
    let f = Fixture::new("f02-similar");
    let a = f.write("a/img.jpg", &[1u8; 4000]);
    let b = f.write("b/img.jpg", &[3u8; 5000]);
    let mut item = f.item(&b, Action::Trash);
    item.keep = Some(keep_of(&f, &a, false));
    std::fs::write(&a, [1u8; 10]).unwrap();
    let (_, out) = f.apply(&[item], &opts());
    assert_eq!(out.skipped, 1, "{out:?}");
    assert!(b.exists());
}

/// An optimiser that writes its intermediate result to a FIXED name in `job.temp_dir`
/// (as several real optimisers did), waits for a second job to do the same, then copies
/// that intermediate file to its output. Run concurrently with one shared temp dir, the
/// two jobs exchanged files before F-03.
struct FixedScratch {
    barrier: std::sync::Barrier,
}

impl mj_core::Optimiser for FixedScratch {
    fn info(&self) -> mj_core::OptimiserInfo {
        mj_core::OptimiserInfo {
            id: mj_core::OptimiserId::PngLossless,
            title: "fixed".into(),
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
        let data = std::fs::read(job.src).unwrap();
        let scratch = job.temp_dir.join("stage1.bin");
        std::fs::write(&scratch, &data[..data.len() / 2]).unwrap();
        self.barrier.wait();
        std::fs::copy(&scratch, job.dst).unwrap();
        let _ = std::fs::remove_file(&scratch);
        Ok(mj_core::OptimiseOutput {
            bytes_before: data.len() as u64,
            bytes_after: (data.len() / 2) as u64,
            ..mj_core::OptimiseOutput::default()
        })
    }
    fn analyse(&self, _: &mj_core::DetectContext<'_>) -> mj_core::Result<mj_core::DetectorOutput> {
        Ok(mj_core::DetectorOutput::default())
    }
}

fn optimise_action() -> Action {
    Action::Optimise {
        optimiser: mj_core::OptimiserId::PngLossless,
        params: mj_core::ParamValues::default(),
        output: mj_core::OutputMode::Replace,
        originals: OriginalsPolicy::Quarantine,
        keep_if_not_smaller: true,
    }
}

#[test]
fn f03_concurrent_apply_and_dry_run_never_share_a_scratch_file() {
    let f = Fixture::new("f03");
    let a = f.write("a.bin", &[b'A'; 4000]);
    let b = f.write("b.bin", &[b'B'; 4000]);
    let opt: Vec<Box<dyn mj_core::Optimiser>> = vec![Box::new(FixedScratch { barrier: std::sync::Barrier::new(2) })];
    let services = Services { optimisers: &opt, archiver: None, tools: &[] };
    let (ia, ib) = (f.item(&a, optimise_action()), f.item(&b, optimise_action()));
    let run = f.new_run(RunKind::Apply);
    let (applied, dry) = std::thread::scope(|s| {
        let h1 = s.spawn(|| mj_actions::execute_with(&f.ctx(run, services), std::slice::from_ref(&ia), &opts()));
        let h2 = s.spawn(|| mj_actions::dry_run(&f.ctx(mj_core::RunId(0), services), std::slice::from_ref(&ib)));
        (h1.join().unwrap().unwrap(), h2.join().unwrap().unwrap())
    });
    assert_eq!(applied.succeeded, 1, "{applied:?}");
    assert_eq!(dry.estimate.files, 1, "{dry:?}");
    assert_eq!(std::fs::read(&a).unwrap(), vec![b'A'; 2000], "a's output is a's own content");
    assert_eq!(std::fs::read(&b).unwrap(), vec![b'B'; 4000], "the dry run changed nothing");
    assert_eq!(std::fs::read_dir(&f.tmp).map(|d| d.count()).unwrap_or(0), 0, "scratch folders were removed");
}

/// An optimiser that "writes" its output as a hard link to the original.
struct LinkBack;

impl mj_core::Optimiser for LinkBack {
    fn info(&self) -> mj_core::OptimiserInfo {
        FixedScratch { barrier: std::sync::Barrier::new(1) }.info()
    }
    fn availability(&self, _: &[ExternalTool]) -> mj_core::Availability {
        mj_core::Availability::Available
    }
    fn applies_to(&self, _: &mj_core::FileEntry) -> bool {
        true
    }
    fn estimate(&self, e: &mj_core::FileEntry, p: &mj_core::ParamValues) -> mj_core::Result<mj_core::Estimate> {
        FixedScratch { barrier: std::sync::Barrier::new(1) }.estimate(e, p)
    }
    fn optimise(&self, job: &mj_core::OptimiseJob<'_>) -> mj_core::Result<mj_core::OptimiseOutput> {
        std::fs::hard_link(job.src, job.dst).unwrap();
        Ok(mj_core::OptimiseOutput { privacy: true, ..mj_core::OptimiseOutput::default() })
    }
    fn analyse(&self, _: &mj_core::DetectContext<'_>) -> mj_core::Result<mj_core::DetectorOutput> {
        Ok(mj_core::DetectorOutput::default())
    }
}

#[test]
fn f03_an_output_that_is_not_a_new_file_never_replaces_the_original() {
    let f = Fixture::new("f03-link");
    let a = f.write("a.bin", &[b'A'; 4000]);
    let before = snapshot(&f.root());
    let opt: Vec<Box<dyn mj_core::Optimiser>> = vec![Box::new(LinkBack)];
    let services = Services { optimisers: &opt, archiver: None, tools: &[] };
    let run = f.new_run(RunKind::Apply);
    let out = mj_actions::execute_with(&f.ctx(run, services), &[f.item(&a, optimise_action())], &opts()).unwrap();
    assert_eq!(out.succeeded, 0, "{out:?}");
    assert_eq!(snapshot(&f.root()), before);
}

#[test]
fn f04_folder_that_changed_after_the_scan_is_refused_for_every_action() {
    let f = Fixture::new("f04");
    let dir = f.t.mkdir("root/was-empty").unwrap();
    let scanned = folder_actions(&f).into_iter().map(|a| f.item(&dir, a)).collect::<Vec<_>>();
    // After the scan the user saves a document into the folder.
    let doc = f.write("was-empty/thesis.docx", b"two years of work");
    let archiver = NoArchiver;
    let services = Services { optimisers: &[], archiver: Some(&archiver), tools: &[] };
    for item in scanned {
        let run = f.new_run(RunKind::Apply);
        let out = mj_actions::execute_with(&f.ctx(run, services), std::slice::from_ref(&item), &confirmed()).unwrap();
        assert_eq!((out.succeeded, out.skipped), (0, 1), "{:?}: {out:?}", item.action);
        assert_eq!(out.errors[0].error.as_ref().unwrap().code, "changed");
        assert_eq!(std::fs::read(&doc).unwrap(), b"two years of work");
    }
}

#[test]
fn f04_same_size_edit_inside_a_folder_and_missing_stamp_are_refused() {
    let f = Fixture::new("f04-edit");
    f.write("cache/a.bin", &[1u8; 100]);
    let dir = f.root().join("cache");
    let item = f.item(&dir, Action::PermanentDelete);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(dir.join("a.bin"), [2u8; 100]).unwrap();
    let (_, out) = f.apply(&[item], &confirmed());
    assert_eq!(out.skipped, 1, "{out:?}");
    // A folder item that carries no scan stamp of its subtree is never acted on.
    let mut item = f.item(&dir, Action::PermanentDelete);
    item.expected_tree = None;
    let (_, out) = f.apply(&[item], &confirmed());
    assert_eq!(out.skipped, 1, "{out:?}");
    assert!(dir.join("a.bin").exists());
    // Unchanged: removed.
    let (_, out) = f.apply(&[f.item(&dir, Action::PermanentDelete)], &confirmed());
    assert_eq!(out.succeeded, 1, "{out:?}");
}

/// An archiver that adds a file to the source folder while archiving (the user saving
/// a document during a long archive run).
struct AddsDuringArchive;

impl Archiver for AddsDuringArchive {
    fn formats(&self, _: &[ExternalTool]) -> Vec<ArchiveFormat> {
        vec![ArchiveFormat::Zip]
    }
    fn create(&self, job: &ArchiveJob<'_>) -> mj_core::Result<u64> {
        std::fs::write(job.dst, b"archive").unwrap();
        std::fs::write(job.sources[0].join("new.txt"), b"saved meanwhile").unwrap();
        Ok(7)
    }
    fn verify(&self, _: &Path, _: &[PathBuf], _: &Path, _: &mj_core::CancelToken) -> mj_core::Result<()> {
        Ok(())
    }
}

#[test]
fn f04_archive_keeps_sources_that_changed_while_archiving() {
    let f = Fixture::new("f04-arc");
    f.write("proj/a.txt", b"a");
    let dir = f.root().join("proj");
    let archiver = AddsDuringArchive;
    let services = Services { optimisers: &[], archiver: Some(&archiver), tools: &[] };
    let item = f.item(
        &dir,
        Action::Archive {
            format: ArchiveFormat::Zip,
            level: 6,
            dest: None,
            originals: OriginalsPolicy::Delete,
            keep_sources: false,
        },
    );
    let run = f.new_run(RunKind::Apply);
    let out = mj_actions::execute_with(&f.ctx(run, services), &[item], &confirmed()).unwrap();
    assert_eq!(out.succeeded, 0, "{out:?}");
    assert!(dir.join("new.txt").exists() && dir.join("a.txt").exists(), "the sources were kept");
    assert!(!f.root().join("proj.zip").exists(), "the stale archive was removed");
}

struct Halve;

impl mj_core::Optimiser for Halve {
    fn info(&self) -> mj_core::OptimiserInfo {
        FixedScratch { barrier: std::sync::Barrier::new(1) }.info()
    }
    fn availability(&self, _: &[ExternalTool]) -> mj_core::Availability {
        mj_core::Availability::Available
    }
    fn applies_to(&self, _: &mj_core::FileEntry) -> bool {
        true
    }
    fn estimate(&self, e: &mj_core::FileEntry, p: &mj_core::ParamValues) -> mj_core::Result<mj_core::Estimate> {
        FixedScratch { barrier: std::sync::Barrier::new(1) }.estimate(e, p)
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

fn optimise_to(originals: OriginalsPolicy) -> Action {
    Action::Optimise {
        optimiser: mj_core::OptimiserId::PngLossless,
        params: mj_core::ParamValues::default(),
        output: mj_core::OutputMode::Replace,
        originals,
        keep_if_not_smaller: true,
    }
}

#[test]
fn f05_backup_folder_is_guarded_like_a_destination() {
    let mut f = Fixture::new("f05");
    let protected_dir = f.t.mkdir("protected").unwrap();
    f.guard =
        Guard::new(vec![f.root()], ProtectedPaths::empty().with_app_dirs(&[f.qdir.clone(), protected_dir.clone()]));
    let file = f.write("photo.bin", &[b'p'; 4000]);
    let opt: Vec<Box<dyn mj_core::Optimiser>> = vec![Box::new(Halve)];
    let services = Services { optimisers: &opt, archiver: None, tools: &[] };
    std::fs::create_dir_all(f.t.join("elsewhere")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&protected_dir, f.t.join("elsewhere/link")).unwrap();
    let mut bad =
        vec![protected_dir.join("x"), f.qdir.join("backups"), f.tmp.join("b"), f.root().join("photo.bin/sub")];
    if cfg!(unix) {
        bad.push(f.t.join("elsewhere/link/x"));
    }
    for folder in bad {
        let before = snapshot(&f.root());
        let item = f.item(&file, optimise_to(OriginalsPolicy::BackupFolder { folder: folder.clone() }));
        let run = f.new_run(RunKind::Apply);
        let out = mj_actions::execute_with(&f.ctx(run, services), &[item], &opts()).unwrap();
        assert_eq!((out.succeeded, out.skipped), (0, 1), "{}: {out:?}", folder.display());
        assert_eq!(snapshot(&f.root()), before);
    }
    assert_eq!(std::fs::read_dir(&protected_dir).unwrap().count(), 0, "nothing written into the protected folder");
    // A plain folder elsewhere is fine.
    let item = f.item(&file, optimise_to(OriginalsPolicy::BackupFolder { folder: f.t.join("backup") }));
    let run = f.new_run(RunKind::Apply);
    let out = mj_actions::execute_with(&f.ctx(run, services), &[item], &opts()).unwrap();
    assert_eq!(out.succeeded, 1, "{out:?}");
    assert!(f.t.join("backup/photo.bin").exists());
}

#[test]
fn f05_purge_never_deletes_something_that_is_not_a_quarantine_slot() {
    let f = Fixture::new("f05-purge");
    let file = f.write("photo.bin", &[b'p'; 4000]);
    let opt: Vec<Box<dyn mj_core::Optimiser>> = vec![Box::new(Halve)];
    let services = Services { optimisers: &opt, archiver: None, tools: &[] };
    // An entry whose retained copy lies below the quarantine folder but is not its slot
    // (as a backup folder inside the quarantine produced before this fix).
    let run = f.new_run(RunKind::Apply);
    let out = mj_actions::execute_with(
        &f.ctx(run, services),
        &[f.item(&file, optimise_to(OriginalsPolicy::BackupFolder { folder: f.t.join("bk") }))],
        &opts(),
    )
    .unwrap();
    assert_eq!(out.succeeded, 1);
    let mut e = f.journal.entries_of(run).pop().unwrap();
    let foreign = f.t.write("q/backups/photo.bin", &[b'p'; 4000]).unwrap();
    e.retained_at = Some(foreign.clone());
    f.journal.append(&e).unwrap();
    let freed =
        mj_actions::quarantine::purge(&f.journal, &f.qdir, mj_core::Timestamp::now().add_days(400), 30).unwrap();
    assert_eq!(freed, 0);
    assert!(foreign.exists(), "the backup below the quarantine folder survived the purge");
}

#[test]
fn f06_undo_never_discards_a_same_size_edit_of_the_output() {
    for originals in [OriginalsPolicy::Quarantine, OriginalsPolicy::BackupFolder { folder: PathBuf::new() }] {
        let f = Fixture::new("f06");
        let originals = match originals {
            OriginalsPolicy::BackupFolder { .. } => OriginalsPolicy::BackupFolder { folder: f.t.join("bk") },
            o => o,
        };
        let file = f.write("doc.bin", &[b'a'; 4000]);
        let opt: Vec<Box<dyn mj_core::Optimiser>> = vec![Box::new(Halve)];
        let services = Services { optimisers: &opt, archiver: None, tools: &[] };
        let run = f.new_run(RunKind::Apply);
        let out =
            mj_actions::execute_with(&f.ctx(run, services), &[f.item(&file, optimise_to(originals))], &opts()).unwrap();
        assert_eq!(out.succeeded, 1, "{out:?}");
        // The user keeps working on the optimised file: same size, new content.
        std::fs::write(&file, [b'Z'; 2000]).unwrap();
        let undo = f.undo_run(run);
        assert_eq!(undo.failed, 1, "{undo:?}");
        assert_eq!(undo.errors[0].error.as_ref().unwrap().code, "conflict");
        assert_eq!(std::fs::read(&file).unwrap(), vec![b'Z'; 2000], "the user's edit is still there");
        // Unchanged output: the undo works.
        std::fs::write(&file, [b'a'; 2000]).unwrap();
        let undo = f.undo_run(run);
        assert_eq!(undo.succeeded, 1, "{undo:?}");
        assert_eq!(std::fs::read(&file).unwrap(), vec![b'a'; 4000]);
    }
}

#[test]
fn f07_hardlink_needs_confirmation_and_undo_after_an_edit_refuses_without_damage() {
    assert_eq!(Action::Hardlink.safety(false), mj_core::SafetyClass::LossyOptIn);
    assert_eq!(Action::Reflink.safety(false), mj_core::SafetyClass::Reversible);
    let f = Fixture::new("f07");
    let keeper = f.write("a/keep.txt", b"version 1 of the text");
    let dup = f.write("b/copy.txt", b"version 1 of the text");
    let mut item = f.item(&dup, Action::Hardlink);
    item.link_source = Some(keeper.clone());
    // Unconfirmed (a schedule, a caller that skipped the dialog): refused.
    let (_, out) = f.apply(std::slice::from_ref(&item), &opts());
    assert_eq!(out.skipped, 1);
    assert_eq!(out.errors[0].error.as_ref().unwrap().code, "confirmation_required");
    // Confirmed: linked; an untouched pair separates again on undo.
    let (run, out) = f.apply(std::slice::from_ref(&item), &confirmed());
    assert_eq!(out.succeeded, 1, "{out:?}");
    // Documented behaviour: after an edit through either name both names change and the
    // undo refuses (conflict) without touching either file.
    std::fs::write(&keeper, b"version 2, edited later").unwrap();
    assert_eq!(std::fs::read(&dup).unwrap(), b"version 2, edited later");
    let undo = f.undo_run(run);
    assert_eq!(undo.failed, 1);
    assert_eq!(undo.errors[0].error.as_ref().unwrap().code, "conflict");
    assert_eq!(std::fs::read(&keeper).unwrap(), b"version 2, edited later");
    assert_eq!(std::fs::read(&dup).unwrap(), b"version 2, edited later");
}
