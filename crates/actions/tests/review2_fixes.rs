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
            ProtectedPaths::empty().with_app_dirs(&[f.t.join("q")]).with_user_folders(&[protect.clone()]),
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
