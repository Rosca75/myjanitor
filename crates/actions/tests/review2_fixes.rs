//! Regression tests for the findings of `docs/review/adversarial-review-2.md` that the
//! action engine owns. Each test reproduces the reviewed scenario on a fixture tree inside
//! its own `TempDir` and asserts the safe behaviour. Names start with the finding id.

mod common;

#[allow(unused_imports)]
use std::path::{Path, PathBuf};

use common::*;
use mj_actions::{Guard, ProtectedPaths};
use mj_core::Action;

/// G-01: the scan root is a symlink to the real folder and the user protected a folder
/// under its real (canonical) spelling; deleting a folder above it through the link must
/// be refused. Also the reverse: protected under the link spelling, item spelled canonically.
#[cfg(unix)]
#[test]
fn g01_protected_entry_below_a_folder_is_seen_through_a_symlinked_root() {
    let mut f = Fixture::new("g01-symroot");
    let real = f.t.join("real");
    std::fs::create_dir_all(real.join("proj/contracts")).unwrap();
    std::fs::write(real.join("proj/contracts/c.pdf"), b"signed contract").unwrap();
    std::fs::write(real.join("proj/notes.txt"), b"notes").unwrap();
    std::fs::remove_dir_all(f.root()).unwrap();
    std::os::unix::fs::symlink(&real, f.root()).unwrap();
    for protect in [real.join("proj/contracts"), f.root().join("proj/contracts")] {
        f.guard = Guard::new(
            vec![f.root()],
            ProtectedPaths::empty().with_app_dirs(&[f.t.join("q")]).with_user_folders(std::slice::from_ref(&protect)),
        );
        for action in [Action::PermanentDelete, Action::Trash, Action::Quarantine { purge_after_days: 30 }] {
            let item = f.item(&f.root().join("proj"), action.clone());
            let (_, out) = f.apply(&[item], &confirmed());
            assert_eq!(out.succeeded, 0, "{protect:?} {action:?}: {out:?}");
            assert!(real.join("proj/contracts/c.pdf").exists(), "{protect:?} {action:?}: contract gone");
        }
    }
    // A protected pattern matched only in the canonical spelling is honoured too.
    f.guard = Guard::new(
        vec![f.root()],
        ProtectedPaths::empty()
            .with_app_dirs(&[f.t.join("q")])
            .with_patterns(&[format!("{}/**", real.join("proj/contracts").display())]),
    );
    let item = f.item(&f.root().join("proj"), Action::PermanentDelete);
    let (_, out) = f.apply(&[item], &confirmed());
    assert_eq!(out.succeeded, 0, "{out:?}");
    assert!(real.join("proj/contracts/c.pdf").exists());
}

/// G-02: a cross-volume move (forced copy) where an application writes a new file and
/// rewrites another while the source is being deleted: neither may be lost, the copy is
/// kept and the move reports what it left behind.
#[test]
fn g02_cross_volume_move_never_deletes_what_changed_after_the_copy() {
    let t = mj_core::testutil::TempDir::new("g02-xvol").unwrap();
    t.write("src/a.txt", b"aaaa").unwrap();
    t.write("src/sub/b.txt", b"old content").unwrap();
    t.write("src/sub/c.txt", b"cccc").unwrap();
    let (src, dst) = (t.join("src"), t.join("dst"));
    let first = std::sync::atomic::AtomicBool::new(true);
    let (late, b) = (src.join("sub/late.txt"), src.join("sub/b.txt"));
    let how = mj_actions::fsops::move_item_with(&src, &dst, true, &|p: &Path, dir: bool| {
        if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
            std::fs::write(&late, b"written during the move").unwrap();
            std::fs::write(&b, b"NEW content!!").unwrap();
        }
        if dir {
            std::fs::remove_dir(p)
        } else {
            std::fs::remove_file(p)
        }
    })
    .unwrap();
    assert!(matches!(how, mj_actions::fsops::MoveHow::CopiedSourceLeft { .. }), "{how:?}");
    assert_eq!(std::fs::read(&late).unwrap(), b"written during the move", "new file kept at the source");
    assert_eq!(std::fs::read(&b).unwrap(), b"NEW content!!", "rewritten file kept at the source");
    assert_eq!(std::fs::read(dst.join("sub/b.txt")).unwrap(), b"old content", "the copy is complete");
    assert!(dst.join("a.txt").exists() && dst.join("sub/c.txt").exists());
}

/// G-02: when the source changed between the end of the copy and the first deletion,
/// nothing is deleted: our own copy is removed again and the move is refused (`changed`).
#[test]
fn g02_source_changed_before_deletion_is_refused_whole_and_untouched() {
    for case in ["new file", "rewrite", "single file"] {
        let t = mj_core::testutil::TempDir::new("g02-pre").unwrap();
        t.write("src/a.txt", b"aaaa").unwrap();
        t.write("src/sub/b.txt", b"old content").unwrap();
        let single = case == "single file";
        let src = if single { t.join("src/a.txt") } else { t.join("src") };
        let dst = t.join("dst");
        let s2 = src.clone();
        let r = mj_actions::fsops::move_item_hooked(
            &src,
            &dst,
            true,
            &|| match case {
                "new file" => std::fs::write(s2.join("sub/late.txt"), b"late").unwrap(),
                "rewrite" => std::fs::write(s2.join("sub/b.txt"), b"NEW content").unwrap(),
                _ => std::fs::write(&s2, b"aaaa+appended").unwrap(),
            },
            &|_, _| panic!("{case}: nothing may be deleted"),
        );
        assert_eq!(r.unwrap_err().code(), "changed", "{case}");
        assert!(!dst.exists(), "{case}: our copy was removed again");
        assert!(src.exists(), "{case}");
    }
}

/// G-07: a link or a folder put where a moved file was is never moved back by Restore.
#[cfg(unix)]
#[test]
fn g07_restore_refuses_a_link_or_folder_found_where_a_moved_file_was() {
    for replace_with in ["link", "folder"] {
        let f = Fixture::new("g07");
        let file = f.write("report.txt", b"the original report");
        let dest = f.t.join("archive-folder");
        let (run, out) = f.apply(&[f.item(&file, Action::Move { dest: dest.clone(), keep_structure: false })], &opts());
        assert_eq!(out.succeeded, 1);
        let moved = dest.join("report.txt");
        std::fs::remove_file(&moved).unwrap();
        if replace_with == "link" {
            f.t.write("elsewhere/secret.txt", b"secret").unwrap();
            std::os::unix::fs::symlink(f.t.join("elsewhere/secret.txt"), &moved).unwrap();
        } else {
            std::fs::create_dir(&moved).unwrap();
        }
        let undo = f.undo_run(run);
        assert_eq!(undo.failed, 1, "{replace_with}: {undo:?}");
        assert_eq!(undo.errors[0].error.as_ref().unwrap().code, "conflict");
        assert!(std::fs::symlink_metadata(&file).is_err(), "{replace_with}: moved back");
    }
}

/// G-07: a link removed as itself (a broken link) is still restored as a link.
#[cfg(unix)]
#[test]
fn g07_a_trashed_link_is_restored_as_a_link() {
    let f = Fixture::new("g07-link");
    let link = f.root().join("broken");
    std::os::unix::fs::symlink(f.t.join("nowhere"), &link).unwrap();
    let mut item = f.item(&f.root(), Action::Trash);
    item.path = link.clone();
    item.target = mj_core::TargetKind::File;
    item.expected_tree = None;
    let md = std::fs::symlink_metadata(&link).unwrap();
    item.expected = mj_core::FileStamp { size: 0, mtime: mj_core::Timestamp::from_system_time(md.modified().unwrap()) };
    let (run, out) = f.apply(&[item], &opts());
    assert_eq!(out.succeeded, 1, "{out:?}");
    assert!(f.journal.entries_of(run).iter().any(|e| e.original_link));
    let undo = f.undo_run(run);
    assert_eq!(undo.succeeded, 1, "{undo:?}");
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
}

/// G-09: an archiver whose verification window sees the source file being rewritten.
struct WriteDuringVerify;

impl mj_core::Archiver for WriteDuringVerify {
    fn formats(&self, _: &[mj_core::ExternalTool]) -> Vec<mj_core::ArchiveFormat> {
        vec![mj_core::ArchiveFormat::Zip]
    }
    fn create(&self, job: &mj_core::ArchiveJob<'_>) -> mj_core::Result<u64> {
        std::fs::write(job.dst, b"archive bytes").unwrap();
        Ok(13)
    }
    fn verify(&self, _: &Path, sources: &[PathBuf], _: &Path, _: &mj_core::CancelToken) -> mj_core::Result<()> {
        std::fs::write(&sources[0], b"edited after it was archived").unwrap();
        Ok(())
    }
}

#[test]
fn g09_an_archived_file_is_rechecked_before_its_source_goes() {
    let f = Fixture::new("g09");
    let file = f.write("report.txt", b"the report as archived");
    let archiver = WriteDuringVerify;
    let services = mj_actions::Services { optimisers: &[], archiver: Some(&archiver), tools: &[] };
    for originals in [mj_core::OriginalsPolicy::Delete, mj_core::OriginalsPolicy::Quarantine] {
        std::fs::write(&file, b"the report as archived").unwrap();
        let item = f.item(
            &file,
            Action::Archive {
                format: mj_core::ArchiveFormat::Zip,
                level: 6,
                dest: None,
                originals: originals.clone(),
                keep_sources: false,
            },
        );
        let run = f.new_run(mj_core::RunKind::Apply);
        let out = mj_actions::execute_with(&f.ctx(run, services), &[item], &confirmed()).unwrap();
        assert_eq!(out.succeeded, 0, "{originals:?}: {out:?}");
        assert_eq!(std::fs::read(&file).unwrap(), b"edited after it was archived", "{originals:?}");
    }
}
