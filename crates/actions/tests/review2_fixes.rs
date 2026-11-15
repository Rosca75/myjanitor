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
