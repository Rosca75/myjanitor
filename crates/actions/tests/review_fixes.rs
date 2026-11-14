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
