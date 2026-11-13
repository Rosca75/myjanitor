//! Action engine: every reversible action is undone back to the exact tree it started
//! from (paths, sizes, content hashes, file mtimes), guard refusals never touch
//! anything, mid-batch failures leave a consistent and undoable journal.
//! Every test works inside its own `TempDir` fixture.

mod common;

use std::path::Path;

use common::*;
use mj_actions::{Services, TrashRoute};
use mj_core::{
    Action, ArchiveFormat, ArchiveJob, Archiver, Availability, Category, DetectContext, DetectorOutput, Estimate,
    ExternalTool, FileEntry, ItemStatus, JournalState, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId,
    OptimiserInfo, OriginalsPolicy, OutputMode, ParamValues, PlatformSupport, Result, RunKind, SafetyClass, Timestamp,
};

/// Writes the first half of the input; optionally lossy or panicking.
struct HalfOptimiser {
    lossy: bool,
    panic_on: Option<&'static str>,
    /// Write the input unchanged instead (never smaller).
    same: bool,
}

fn half(lossy: bool, panic_on: Option<&'static str>) -> HalfOptimiser {
    HalfOptimiser { lossy, panic_on, same: false }
}

fn same_size() -> HalfOptimiser {
    HalfOptimiser { lossy: false, panic_on: None, same: true }
}

impl Optimiser for HalfOptimiser {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::PngLossless,
            title: "half".into(),
            description: String::new(),
            category: Category::Optimise,
            safety: SafetyClass::Reversible,
            default_action: mj_core::ActionKind::Optimise,
            available_actions: vec![],
            presets: vec![],
            settings: vec![],
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec![],
            unit: String::new(),
        }
    }
    fn availability(&self, _: &[ExternalTool]) -> Availability {
        Availability::Available
    }
    fn applies_to(&self, _: &FileEntry) -> bool {
        true
    }
    fn estimate(&self, e: &FileEntry, _: &ParamValues) -> Result<Estimate> {
        Ok(Estimate { files: 1, bytes_before: e.size, bytes_after: e.size / 2, ..Default::default() })
    }
    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        if let Some(p) = self.panic_on {
            if job.src.to_string_lossy().contains(p) {
                panic!("hostile file");
            }
        }
        let data = std::fs::read(job.src).unwrap();
        let keep = if self.same { data.len() } else { data.len() / 2 };
        std::fs::write(job.dst, &data[..keep]).unwrap();
        Ok(OptimiseOutput {
            bytes_before: data.len() as u64,
            bytes_after: (data.len() / 2) as u64,
            lossy: self.lossy,
            skipped: None,
            notes: vec![],
        })
    }
    fn analyse(&self, _: &DetectContext<'_>) -> Result<DetectorOutput> {
        Ok(DetectorOutput::default())
    }
}

/// "Archives" by concatenating the bytes of the source file.
struct CatArchiver;

impl Archiver for CatArchiver {
    fn formats(&self, _: &[ExternalTool]) -> Vec<ArchiveFormat> {
        vec![ArchiveFormat::Zip]
    }
    fn create(&self, job: &ArchiveJob<'_>) -> Result<u64> {
        let data = std::fs::read(&job.sources[0]).unwrap();
        std::fs::write(job.dst, &data[..data.len() / 3]).unwrap();
        Ok((data.len() / 3) as u64)
    }
    fn verify(&self, _: &Path, _: &[std::path::PathBuf], _: &Path, _: &mj_core::CancelToken) -> Result<()> {
        Ok(())
    }
}

fn tree(f: &Fixture) {
    f.write("a.txt", b"alpha alpha alpha");
    f.write("sub/b.bin", &[7u8; 5000]);
    f.write("sub/deep/c.txt", b"charlie");
    f.write("dir/x.txt", b"x");
    f.write("dir/y/z.txt", b"zz");
}

/// Apply `items`, check they succeeded, undo the run and compare the whole tree.
fn round_trip(
    f: &Fixture,
    make: impl Fn(&Fixture) -> Vec<mj_core::ActionItem>,
    services: Services<'_>,
    expect_change: bool,
) {
    let before = snapshot(&f.root());
    let items = make(f);
    let run = f.new_run(RunKind::Apply);
    let out = mj_actions::execute_with(&f.ctx(run, services), &items, &confirmed()).unwrap();
    assert_eq!(out.succeeded, items.len() as u64, "{:?}", out.errors);
    if expect_change {
        assert_ne!(snapshot(&f.root()), before, "the action changed nothing");
    }
    for e in f.journal.entries_of(run) {
        assert_eq!(e.state, JournalState::Done);
        assert!(e.reversible, "{e:?}");
    }
    let undo = f.undo_run(run);
    assert_eq!(undo.succeeded, items.len() as u64, "{:?}", undo.errors);
    assert_eq!(snapshot(&f.root()), before, "tree after undo differs from the tree before");
    for e in f.journal.entries_of(run) {
        assert_eq!(e.state, JournalState::Undone);
    }
}

#[test]
fn trash_files_and_folders_round_trip() {
    let f = Fixture::new("trash-rt");
    tree(&f);
    round_trip(
        &f,
        |f| vec![f.item(&f.root().join("a.txt"), Action::Trash), f.item(&f.root().join("dir"), Action::Trash)],
        no_services(),
        true,
    );
    // The trash was the fixture's own.
    assert!(f.t.join("xdg/Trash/info").exists());
}

#[test]
fn quarantine_round_trip_with_manifest() {
    let f = Fixture::new("quar-rt");
    tree(&f);
    let before = snapshot(&f.root());
    let (run, out) = f.apply(
        &[
            f.item(&f.root().join("sub/b.bin"), Action::Quarantine { purge_after_days: 30 }),
            f.item(&f.root().join("dir"), Action::Quarantine { purge_after_days: 30 }),
        ],
        &opts(),
    );
    assert_eq!(out.succeeded, 2);
    let e = &f.journal.entries_of(run)[0];
    let slot = e.retained_at.clone().unwrap();
    assert!(slot.starts_with(&f.qdir));
    assert_eq!(slot.file_name().unwrap(), "b.bin");
    let m = mj_actions::quarantine::read_manifest(&f.qdir, run, e.id).unwrap();
    assert_eq!(m.original, f.root().join("sub/b.bin"));
    assert_eq!(m.size, 5000);
    assert!(m.sha256.is_some() && m.xxh64.is_some());
    assert_eq!(e.original_sha256, m.sha256);
    f.undo_run(run);
    assert_eq!(snapshot(&f.root()), before);
    assert!(std::fs::read_dir(&f.qdir).unwrap().next().is_none(), "quarantine slots cleaned up");
}

#[test]
fn windows_route_sends_trash_to_quarantine() {
    let f = Fixture::new("route");
    tree(&f);
    let before = snapshot(&f.root());
    let o = mj_actions::ExecOptions { trash_route: TrashRoute::Quarantine, ..opts() };
    let (run, out) = f.apply(&[f.item(&f.root().join("a.txt"), Action::Trash)], &o);
    assert_eq!(out.succeeded, 1);
    let e = &f.journal.entries_of(run)[0];
    assert_eq!(e.action, mj_core::ActionKind::Quarantine);
    assert!(e.trash.is_none() && e.retained_at.is_some());
    assert!(!f.t.join("xdg/Trash").exists(), "the platform trash was not used");
    f.undo_run(run);
    assert_eq!(snapshot(&f.root()), before);
    assert_eq!(TrashRoute::default_for_os(), if cfg!(windows) { TrashRoute::Quarantine } else { TrashRoute::Platform });
}

#[test]
fn move_and_copy_round_trip_keeping_structure() {
    for keep in [true, false] {
        let f = Fixture::new("move-rt");
        tree(&f);
        let dest = f.root().join("archive");
        round_trip(
            &f,
            |f| {
                vec![
                    f.item(&f.root().join("sub/deep/c.txt"), Action::Move { dest: dest.clone(), keep_structure: keep }),
                    f.item(&f.root().join("dir"), Action::Move { dest: dest.clone(), keep_structure: keep }),
                ]
            },
            no_services(),
            true,
        );
        let f = Fixture::new("copy-rt");
        tree(&f);
        let dest = f.t.join("elsewhere");
        let before_dest = dest.exists();
        round_trip(
            &f,
            |f| {
                vec![
                    f.item(&f.root().join("a.txt"), Action::Copy { dest: dest.clone(), keep_structure: keep }),
                    f.item(&f.root().join("sub"), Action::Copy { dest: dest.clone(), keep_structure: keep }),
                ]
            },
            no_services(),
            false,
        );
        assert_eq!(dest.exists(), before_dest, "folders created by the copy are removed by undo");
    }
}

#[test]
fn move_keeps_structure_and_never_overwrites() {
    let f = Fixture::new("move-collide");
    tree(&f);
    let dest = f.t.join("dest");
    std::fs::create_dir_all(dest.join("sub/deep")).unwrap();
    std::fs::write(dest.join("sub/deep/c.txt"), b"already here").unwrap();
    let (run, out) = f.apply(
        &[f.item(&f.root().join("sub/deep/c.txt"), Action::Move { dest: dest.clone(), keep_structure: true })],
        &opts(),
    );
    assert_eq!(out.succeeded, 1);
    assert_eq!(std::fs::read(dest.join("sub/deep/c.txt")).unwrap(), b"already here");
    assert_eq!(std::fs::read(dest.join("sub/deep/c (1).txt")).unwrap(), b"charlie");
    assert_eq!(f.journal.entries_of(run)[0].retained_at.as_deref(), Some(dest.join("sub/deep/c (1).txt").as_path()));
}

#[test]
fn hardlink_and_reflink_round_trip() {
    for action in [Action::Hardlink, Action::Reflink] {
        let f = Fixture::new("link-rt");
        tree(&f);
        let keeper = f.write("keep/orig.bin", &[9u8; 70_000]);
        let dup = f.write("dups/copy.bin", &[9u8; 70_000]);
        // Different mtime than the keeper: undo must restore the duplicate's own.
        mj_core::testutil::set_mtime(&dup, std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000))
            .unwrap();
        let before = snapshot(&f.root());
        let mut item = f.item(&dup, action.clone());
        item.link_source = Some(keeper.clone());
        let (run, out) = f.apply(&[item], &opts());
        assert_eq!(out.succeeded, 1, "{:?}", out.errors);
        assert_eq!(out.bytes_saved, 70_000);
        #[cfg(unix)]
        if action == Action::Hardlink {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&dup).unwrap().ino(), std::fs::metadata(&keeper).unwrap().ino());
        }
        let e = &f.journal.entries_of(run)[0];
        assert!(e.created.is_empty() && e.link_source.is_some() && e.original_sha256.is_some());
        let leftovers: Vec<_> = std::fs::read_dir(f.root().join("dups")).unwrap().flatten().collect();
        assert_eq!(leftovers.len(), 1, "no temporary link left behind");
        f.undo_run(run);
        assert_eq!(snapshot(&f.root()), before);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_ne!(std::fs::metadata(&dup).unwrap().ino(), std::fs::metadata(&keeper).unwrap().ino());
        }
    }
}

#[test]
fn hardlink_refuses_differing_content() {
    let f = Fixture::new("link-differ");
    let keeper = f.write("keep.bin", b"same size AAAA");
    let dup = f.write("dup.bin", b"same size BBBB");
    let before = snapshot(&f.root());
    let mut item = f.item(&dup, Action::Hardlink);
    item.link_source = Some(keeper);
    let (run, out) = f.apply(&[item], &opts());
    assert_eq!((out.succeeded, out.skipped), (0, 1));
    assert_eq!(out.errors[0].error.as_ref().unwrap().code, "changed");
    assert_eq!(snapshot(&f.root()), before);
    assert_eq!(f.journal.entries_of(run)[0].state, JournalState::Skipped);
    // No keeper: refused as well.
    let (_, out) = f.apply(&[f.item(&f.root().join("dup.bin"), Action::Hardlink)], &opts());
    assert_eq!(out.skipped, 1);
    // A real reflink on this filesystem is either done or reported as unsupported (skipped).
    let mut f2 = Fixture::new("reflink-native");
    f2.platform.fake_reflink = false;
    let k = f2.write("k.bin", b"identical");
    let d = f2.write("d.bin", b"identical");
    let before = snapshot(&f2.root());
    let mut item = f2.item(&d, Action::Reflink);
    item.link_source = Some(k);
    let (run, out) = f2.apply(&[item], &opts());
    if out.succeeded == 0 {
        assert_eq!(out.errors[0].error.as_ref().unwrap().code, "unsupported");
        assert_eq!(snapshot(&f2.root()), before);
    } else {
        f2.undo_run(run);
        assert_eq!(snapshot(&f2.root()), before);
    }
}

#[test]
fn optimise_round_trip_for_each_originals_policy() {
    let opt: Vec<Box<dyn Optimiser>> = vec![Box::new(half(false, None))];
    let services = Services { optimisers: &opt, archiver: None, tools: &[] };
    for (policy, output) in [
        (OriginalsPolicy::RecycleBin, OutputMode::Replace),
        (OriginalsPolicy::Quarantine, OutputMode::Replace),
        (OriginalsPolicy::BackupFolder { folder: std::path::PathBuf::new() }, OutputMode::Replace),
        (OriginalsPolicy::RecycleBin, OutputMode::LightCopy { suffix: "_light".into() }),
    ] {
        let f = Fixture::new("opt-rt");
        tree(&f);
        let policy = match policy {
            OriginalsPolicy::BackupFolder { .. } => OriginalsPolicy::BackupFolder { folder: f.t.join("backup") },
            p => p,
        };
        round_trip(
            &f,
            |f| {
                vec![f.item(
                    &f.root().join("sub/b.bin"),
                    Action::Optimise {
                        optimiser: OptimiserId::PngLossless,
                        params: ParamValues::default(),
                        output: output.clone(),
                        originals: policy.clone(),
                        keep_if_not_smaller: true,
                    },
                )]
            },
            services,
            true,
        );
    }
}

#[test]
fn optimise_keeps_original_when_not_smaller_and_refuses_unconfirmed_lossy() {
    let lossy: Vec<Box<dyn Optimiser>> = vec![Box::new(half(true, None))];
    let f = Fixture::new("opt-lossy");
    tree(&f);
    f.write("tiny.txt", b"x");
    let before = snapshot(&f.root());
    let act = Action::Optimise {
        optimiser: OptimiserId::PngLossless,
        params: ParamValues::default(),
        output: OutputMode::Replace,
        originals: OriginalsPolicy::Quarantine,
        keep_if_not_smaller: true,
    };
    let run = f.new_run(RunKind::Apply);
    let items = [f.item(&f.root().join("sub/b.bin"), act.clone()), f.item(&f.root().join("tiny.txt"), act.clone())];
    let services = Services { optimisers: &lossy, archiver: None, tools: &[] };
    let out = mj_actions::execute_with(&f.ctx(run, services), &items[..1], &opts()).unwrap();
    assert_eq!((out.succeeded, out.skipped), (0, 1), "{:?}", out.errors);
    assert_eq!(out.errors[0].error.as_ref().unwrap().code, "confirmation_required");
    let same: Vec<Box<dyn Optimiser>> = vec![Box::new(same_size())];
    let s2 = Services { optimisers: &same, archiver: None, tools: &[] };
    let out = mj_actions::execute_with(&f.ctx(run, s2), &items[1..], &confirmed()).unwrap();
    assert_eq!((out.succeeded, out.skipped), (0, 1), "{:?}", out.errors);
    assert_eq!(out.errors[0].error.as_ref().unwrap().code, "conflict", "kept: not smaller");
    assert_eq!(snapshot(&f.root()), before, "no output left behind, originals untouched");
    // Confirmed: the lossy result replaces the original and the entry says so.
    let out = mj_actions::execute_with(&f.ctx(run, services), &items[..1], &confirmed()).unwrap();
    assert_eq!(out.succeeded, 1);
    assert!(f.journal.entries_of(run).iter().any(|e| e.lossy && e.state == JournalState::Done));
}

#[test]
fn archive_round_trip() {
    let f = Fixture::new("archive-rt");
    tree(&f);
    let arch = CatArchiver;
    let services = Services { optimisers: &[], archiver: Some(&arch), tools: &[] };
    round_trip(
        &f,
        |f| {
            vec![f.item(
                &f.root().join("sub/b.bin"),
                Action::Archive {
                    format: ArchiveFormat::Zip,
                    level: 9,
                    dest: None,
                    originals: OriginalsPolicy::Quarantine,
                    keep_sources: false,
                },
            )]
        },
        services,
        true,
    );
}

#[test]
fn dehydrate_and_fs_compress_round_trip() {
    let f = Fixture::new("cloud-rt");
    tree(&f);
    let p = f.root().join("a.txt");
    let (run, out) = f.apply(
        &[
            f.item(&p, Action::Dehydrate),
            f.item(&f.root().join("sub/b.bin"), Action::FsCompress { algorithm: mj_core::FsCompressAlgo::Lzx }),
        ],
        &opts(),
    );
    assert_eq!(out.succeeded, 2, "{:?}", out.errors);
    assert!(f.platform.online.lock().unwrap().contains(&p));
    assert_eq!(f.platform.compressed.lock().unwrap().len(), 1);
    f.undo_run(run);
    assert!(f.platform.online.lock().unwrap().is_empty());
    assert!(f.platform.compressed.lock().unwrap().is_empty());
    // On the real platform, dehydrate is Windows only: reported as unsupported (skipped).
    if !cfg!(windows) {
        assert_eq!(mj_platform::action::cloud::dehydrate(&p).unwrap_err().code(), "unsupported");
    }
}

#[test]
fn permanent_delete_needs_confirmation_and_is_irreversible() {
    let f = Fixture::new("perm");
    tree(&f);
    let before = snapshot(&f.root());
    let (run, out) = f.apply(&[f.item(&f.root().join("a.txt"), Action::PermanentDelete)], &opts());
    assert_eq!(out.skipped, 1);
    assert_eq!(out.errors[0].error.as_ref().unwrap().code, "confirmation_required");
    assert_eq!(snapshot(&f.root()), before);
    assert_eq!(f.journal.entries_of(run)[0].state, JournalState::Skipped);
    let (run, out) = f.apply(&[f.item(&f.root().join("a.txt"), Action::PermanentDelete)], &confirmed());
    assert_eq!(out.succeeded, 1);
    assert!(!f.root().join("a.txt").exists());
    let e = &f.journal.entries_of(run)[0];
    assert!(!e.reversible && e.undo_deadline.is_none());
    assert!(mj_actions::undo::undoable_entries(&f.journal, run, Timestamp::now()).is_empty());
    let u = mj_actions::undo::start_undo_run(&f.journal, Some(run), "Undo", Timestamp::now()).unwrap();
    let out = mj_actions::undo::undo(&f.ctx(u.run_id, no_services()), &[e.id]).unwrap();
    assert_eq!(out.skipped, 1);
}

#[test]
fn guard_refusals_touch_nothing() {
    let f = Fixture::new("guard");
    tree(&f);
    f.t.write("outside/secret.txt", b"secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(f.t.join("outside"), f.root().join("escape")).unwrap();
    let mut items = vec![
        // Outside the allowlist.
        mj_core::ActionItem { root: f.t.join("outside"), ..f.item(&f.t.join("outside/secret.txt"), Action::Trash) },
        // The root itself.
        f.item(&f.root(), Action::Trash),
        // Traversal.
        mj_core::ActionItem {
            path: f.root().join("sub/../../outside/secret.txt"),
            ..f.item(&f.root().join("a.txt"), Action::Trash)
        },
        // Relative.
        mj_core::ActionItem { path: "root/a.txt".into(), ..f.item(&f.root().join("a.txt"), Action::Trash) },
    ];
    #[cfg(unix)]
    {
        // Symlink escape: a path through a link below the root.
        let mut through = f.item(&f.t.join("outside/secret.txt"), Action::PermanentDelete);
        through.path = f.root().join("escape/secret.txt");
        through.root = f.root();
        items.push(through);
        // A link to a folder is not followed for a quarantine of its contents.
        items.push(f.item(&f.root().join("escape"), Action::Move { dest: f.t.join("m"), keep_structure: false }));
    }
    // Changed since the scan.
    let mut changed = f.item(&f.root().join("a.txt"), Action::Trash);
    changed.expected.size += 1;
    items.push(changed);
    let before = snapshot(&f.root());
    let outside_before = snapshot(&f.t.join("outside"));
    let (run, out) = f.apply(&items, &confirmed());
    assert_eq!(out.succeeded, 0, "{:?}", out.errors);
    assert_eq!(out.skipped, items.len() as u64);
    let codes: Vec<String> = out.errors.iter().map(|e| e.error.clone().unwrap().code).collect();
    let mut expected = vec!["outside_allowed_roots", "protected", "invalid_input", "invalid_input"];
    if cfg!(unix) {
        expected.extend(["link_not_followed", "link_not_followed"]);
    }
    expected.push("changed");
    assert_eq!(codes, expected);
    assert_eq!(snapshot(&f.root()), before);
    assert_eq!(snapshot(&f.t.join("outside")), outside_before);
    assert!(f.journal.entries_of(run).iter().all(|e| e.state == JournalState::Skipped));
    // Protected paths (user list) are refused too.
    let mut f2 = Fixture::new("guard-prot");
    tree(&f2);
    f2.guard = mj_actions::Guard::new(vec![f2.root()], mj_actions::ProtectedPaths::new(&[f2.root().join("sub")]));
    let (_, out) = f2.apply(&[f2.item(&f2.root().join("sub/b.bin"), Action::Trash)], &opts());
    assert_eq!(out.errors[0].error.as_ref().unwrap().code, "protected");
    assert!(f2.root().join("sub/b.bin").exists());
}

#[test]
fn mid_batch_failure_is_consistent_and_undoable() {
    let mut f = Fixture::new("midbatch");
    for i in 0..5 {
        f.write(&format!("f{i}.txt"), format!("file {i}").as_bytes());
    }
    f.platform.fail_trash_at = Some(3);
    let before = snapshot(&f.root());
    let items: Vec<_> = (0..5).map(|i| f.item(&f.root().join(format!("f{i}.txt")), Action::Trash)).collect();
    let (run, out) = f.apply(&items, &opts());
    assert_eq!((out.succeeded, out.failed), (4, 1));
    assert!(f.root().join("f2.txt").exists(), "the failed item is untouched");
    let st = states(&f.journal, run);
    assert_eq!(st.values().filter(|s| **s == JournalState::Done).count(), 4);
    assert_eq!(st.values().filter(|s| **s == JournalState::Failed).count(), 1);
    assert!(f.journal.pending_intents().is_empty(), "no dangling intent");
    // The journal on disk says the same after a restart.
    let reopened = mj_actions::Journal::open(&f.t.join("journal")).unwrap();
    assert_eq!(states(&reopened, run), st);
    let undo = f.undo_run(run);
    assert_eq!(undo.succeeded, 4);
    assert_eq!(snapshot(&f.root()), before);
}

#[test]
fn panicking_item_is_isolated() {
    let opt: Vec<Box<dyn Optimiser>> = vec![Box::new(half(false, Some("evil")))];
    let services = Services { optimisers: &opt, archiver: None, tools: &[] };
    let f = Fixture::new("panic");
    f.write("evil.bin", &[1u8; 100]);
    f.write("good.bin", &[2u8; 100]);
    let act = Action::Optimise {
        optimiser: OptimiserId::PngLossless,
        params: ParamValues::default(),
        output: OutputMode::Replace,
        originals: OriginalsPolicy::Quarantine,
        keep_if_not_smaller: true,
    };
    let run = f.new_run(RunKind::Apply);
    let items = [f.item(&f.root().join("evil.bin"), act.clone()), f.item(&f.root().join("good.bin"), act)];
    let out = mj_actions::execute_with(&f.ctx(run, services), &items, &opts()).unwrap();
    assert_eq!((out.succeeded, out.failed), (1, 1), "{:?}", out.errors);
    assert_eq!(out.errors[0].error.as_ref().unwrap().code, "internal");
    assert!(f.journal.pending_intents().is_empty(), "the panicked item's intent was settled");
    assert_eq!(std::fs::read(f.root().join("evil.bin")).unwrap(), [1u8; 100]);
}

#[test]
fn undo_refuses_occupied_original_then_succeeds() {
    let f = Fixture::new("occupied");
    tree(&f);
    let before = snapshot(&f.root());
    let (run, _) = f.apply(&[f.item(&f.root().join("a.txt"), Action::Quarantine { purge_after_days: 30 })], &opts());
    std::fs::write(f.root().join("a.txt"), b"new file in the way").unwrap();
    let out = f.undo_run(run);
    assert_eq!(out.failed, 1);
    assert_eq!(out.errors[0].error.as_ref().unwrap().code, "conflict");
    assert_eq!(std::fs::read(f.root().join("a.txt")).unwrap(), b"new file in the way", "never overwritten");
    let e = &f.journal.entries_of(run)[0];
    assert_eq!(e.state, JournalState::UndoFailed);
    assert!(e.retained_at.as_ref().unwrap().exists(), "the quarantined original is intact");
    std::fs::remove_file(f.root().join("a.txt")).unwrap();
    let out = f.undo_run(run);
    assert_eq!(out.succeeded, 1);
    assert_eq!(snapshot(&f.root()), before);
}

#[test]
fn cancel_stops_before_the_next_item_and_pause_waits() {
    struct CancelAfterFirst<'a>(&'a mj_core::CancelToken);
    impl mj_core::ProgressSink for CancelAfterFirst<'_> {
        fn report(&self, p: &mj_core::Progress) {
            if p.done == 1 {
                self.0.cancel();
            }
        }
    }
    let f = Fixture::new("cancel");
    for i in 0..4 {
        f.write(&format!("f{i}.txt"), b"x");
    }
    let items: Vec<_> = (0..4).map(|i| f.item(&f.root().join(format!("f{i}.txt")), Action::Trash)).collect();
    let run = f.new_run(RunKind::Apply);
    let sink = CancelAfterFirst(&f.cancel);
    let mut ctx = f.ctx(run, no_services());
    ctx.progress = &sink;
    let out = mj_actions::execute_with(&ctx, &items, &opts()).unwrap();
    // Item 0 done; the progress report before item 1 cancels; the checkpoint before item 2 stops.
    assert!(out.succeeded >= 1 && out.succeeded <= 2, "{out:?}");
    assert!(f.root().join("f3.txt").exists());

    let f = Fixture::new("pause");
    f.write("p.txt", b"x");
    f.cancel.pause();
    let token = f.cancel.clone();
    let h = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        token.resume();
    });
    let (_, out) = f.apply(&[f.item(&f.root().join("p.txt"), Action::Trash)], &opts());
    h.join().unwrap();
    assert_eq!(out.succeeded, 1);
}

#[test]
fn signed_and_encrypted_files_are_skipped_by_default() {
    let f = Fixture::new("confidential");
    let signed =
        f.write("contract.docx", &zip(&[("word/document.xml", b"<w/>", 0), ("_xmlsignatures/sig1.xml", b"<s/>", 0)]));
    let enc = f.write("secret.zip", &zip(&[("a.txt", b"xxxx", 1)]));
    let plain = f.write("plain.docx", &zip(&[("word/document.xml", b"<w/>", 0)]));
    let (_, out) =
        f.apply(&[f.item(&signed, Action::Trash), f.item(&enc, Action::Trash), f.item(&plain, Action::Trash)], &opts());
    assert_eq!((out.succeeded, out.skipped), (1, 2), "{:?}", out.errors);
    assert!(signed.exists() && enc.exists() && !plain.exists());
    // Copy is not destructive: allowed.
    let (_, out) = f.apply(&[f.item(&signed, Action::Copy { dest: f.t.join("c"), keep_structure: false })], &opts());
    assert_eq!(out.succeeded, 1);
    // With the setting off, the user's choice is honoured.
    let mut f2 = Fixture::new("confidential-off");
    f2.settings.safety.skip_signed = false;
    let s2 = f2.write("contract.docx", &zip(&[("_xmlsignatures/sig1.xml", b"<s/>", 0)]));
    let (_, out) = f2.apply(&[f2.item(&s2, Action::Trash)], &opts());
    assert_eq!(out.succeeded, 1);
}

#[test]
fn dry_run_measures_without_changing_anything() {
    let opt: Vec<Box<dyn Optimiser>> = vec![Box::new(half(false, None))];
    let services = Services { optimisers: &opt, archiver: None, tools: &[] };
    let f = Fixture::new("dry");
    tree(&f);
    let before = snapshot(&f.root());
    let mut changed = f.item(&f.root().join("a.txt"), Action::Trash);
    changed.expected.mtime = Timestamp(1);
    let items = [
        f.item(&f.root().join("sub/b.bin"), Action::Trash),
        changed,
        f.item(
            &f.root().join("sub/b.bin"),
            Action::Optimise {
                optimiser: OptimiserId::PngLossless,
                params: ParamValues::default(),
                output: OutputMode::Replace,
                originals: OriginalsPolicy::Quarantine,
                keep_if_not_smaller: true,
            },
        ),
    ];
    let run = f.new_run(RunKind::Apply);
    let r = mj_actions::dry_run(&f.ctx(run, services), &items).unwrap();
    assert_eq!(r.estimate.files, 2);
    assert_eq!(r.estimate.skipped, 1);
    assert_eq!(r.skipped[0].error.as_ref().unwrap().code, "changed");
    assert_eq!(r.estimate.bytes_after, 2500);
    assert_eq!(snapshot(&f.root()), before);
    assert!(f.journal.entries_of(run).is_empty(), "a dry run writes no journal entry");
    assert!(std::fs::read_dir(&f.tmp).map(|mut d| d.next().is_none()).unwrap_or(true), "dry-run outputs deleted");
    let _ = ItemStatus::Done;
}

#[test]
fn mixed_batch_with_a_failure_in_the_middle_undoes_exactly() {
    let mut f = Fixture::new("mixed");
    tree(&f);
    let keeper = f.write("k/keep.bin", &[3u8; 4096]);
    let dup = f.write("k/dup.bin", &[3u8; 4096]);
    f.platform.fail_trash_at = Some(1);
    let before = snapshot(&f.root());
    let mut link = f.item(&dup, Action::Hardlink);
    link.link_source = Some(keeper);
    let items = vec![
        f.item(&f.root().join("a.txt"), Action::Quarantine { purge_after_days: 30 }),
        f.item(&f.root().join("dir"), Action::Move { dest: f.root().join("moved"), keep_structure: true }),
        f.item(&f.root().join("sub/deep/c.txt"), Action::Trash),
        f.item(&f.root().join("sub/b.bin"), Action::Copy { dest: f.t.join("copies"), keep_structure: true }),
        link,
    ];
    let (run, out) = f.apply(&items, &opts());
    assert_eq!((out.succeeded, out.failed), (4, 1), "{:?}", out.errors);
    assert!(f.root().join("sub/deep/c.txt").exists());
    assert!(f.journal.pending_intents().is_empty());
    let undo = f.undo_run(run);
    assert_eq!((undo.succeeded, undo.failed), (4, 0), "{:?}", undo.errors);
    assert_eq!(snapshot(&f.root()), before);
    assert!(!f.t.join("copies").exists());
}
