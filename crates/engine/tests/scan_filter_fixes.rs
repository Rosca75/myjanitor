//! Engine-level regression tests for the scan, filter and detector review findings fixed
//! in W6-B (see `docs/decisions/W6-B.md`). Each test drives the real `Engine` on a
//! generated tree inside a `TempDir`. Names start with the finding id.

mod common;

use std::time::{Duration, SystemTime};

use common::*;
use mj_core::testutil::{set_mtime, TempDir};
use mj_core::{OpportunityId, TargetKind};
use mj_engine::api::*;

fn oid(s: &str) -> OpportunityId {
    OpportunityId(s.into())
}

fn rows(e: &mj_engine::Engine, id: &str) -> Vec<CandidateRow> {
    e.candidates_page(CandidatesPageRequest {
        opportunity_id: oid(id),
        sub_group: None,
        offset: 0,
        limit: 500,
        sort: CandidateSortKey::Path,
        descending: false,
        filter: None,
        min_size: None,
        keep_groups: false,
    })
    .unwrap()
    .rows
}

fn days_ago(d: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(d * 86_400)
}

fn run(e: &std::sync::Arc<mj_engine::Engine>, rule: mj_core::Rule, title: &str) -> Vec<CandidateRow> {
    let o =
        e.search_run(SearchRunRequest { rule, title: Some(title.into()), topmost_only: false, action: None }).unwrap();
    rows(e, &o.id.0)
}

/// RF-1: the "Downloads older than 90 days" preset on a year-old folder that holds a file
/// saved last week lists no folder, and the fresh file is in no row.
#[test]
fn rf1_custom_search_never_takes_a_folder_on_file_conditions() {
    let t = TempDir::new("rf1").unwrap();
    t.write("Downloads/Taxes/2026/return.pdf", b"fresh").unwrap();
    t.write("Downloads/old.iso", &bytes(1, 5000)).unwrap();
    for (p, d) in [
        ("Downloads/old.iso", 400),
        ("Downloads/Taxes/2026/return.pdf", 7),
        ("Downloads/Taxes/2026", 400),
        ("Downloads/Taxes", 400),
    ] {
        set_mtime(&t.join(p), days_ago(d)).unwrap();
    }
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let preset = mj_filters::presets::builtin().into_iter().find(|p| p.id == "downloads_older_90d").unwrap();
    // The preset as shipped, and the same rule without its file condition (a user rule).
    let user_rule = mj_filters::parse("path:**/Downloads/** older:90d").unwrap();
    for (rule, title) in [(preset.rule, "preset"), (user_rule, "user")] {
        let r = run(e, rule, title);
        assert!(r.iter().all(|r| r.candidate.target == TargetKind::File), "{title}: {r:#?}");
        let paths: Vec<_> = r.iter().map(|r| r.candidate.path.clone()).collect();
        assert_eq!(paths, vec![f.tree.join("Downloads/old.iso")], "{title}");
    }
    for q in ["size:<1MB", "-ext:mp4"] {
        let r = run(e, mj_filters::parse(q).unwrap(), q);
        assert!(r.iter().all(|r| r.candidate.target == TargetKind::File), "{q}: {r:#?}");
    }
    // A folder matched on its name alone is listed but not pre-selected; `kind:dir` asks
    // for folders in so many words and keeps the pre-selection.
    let r = run(e, mj_filters::parse("name:Taxes").unwrap(), "name");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].candidate.target, TargetKind::Folder);
    assert!(!r[0].candidate.default_selected && !r[0].selected, "{:?}", r[0]);
    let r = run(e, mj_filters::parse("kind:dir name:Taxes").unwrap(), "dir");
    assert_eq!(r.len(), 1);
    assert!(r[0].candidate.default_selected);
}

/// SCAN-5: Settings > Scanning reaches scans started from the app (no explicit scope).
#[test]
fn scan5_scanning_settings_apply_to_app_scans() {
    let t = TempDir::new("scan5").unwrap();
    t.write("a.txt", b"a").unwrap();
    t.write(".hidden/x.txt", b"x").unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    assert_eq!(scan(e).stats.unwrap().files, 2);
    let mut s = e.settings_get().unwrap();
    s.scan.include_hidden = false;
    s.scan.use_cache = false;
    e.settings_set(SettingsSetRequest { settings: s }).unwrap();
    assert!(!e.scope_get().unwrap().options.include_hidden);
    let st = scan(e).stats.unwrap();
    assert_eq!(st.files, 1, "hidden files are left out");
    assert_eq!((st.cache_hits, st.cache_misses), (0, 0), "the cache is not used");
    // Adding a location keeps the options; a new engine on the same data starts with them.
    e.scope_add_root(ScopePathRequest { path: f.app.path().to_path_buf() }).unwrap();
    assert!(!e.scope_get().unwrap().options.include_hidden);
    let again = mj_engine::Engine::new(config(&f.app, std::sync::Arc::new(TimedSink::default()))).unwrap();
    let o = again.scope_get().unwrap().options;
    assert!(!o.include_hidden && !o.use_cache);
}

/// RF-11: a Windows.old row is not a regenerable artefact, so a repository inside it locks
/// the row as it would on any other card.
#[test]
fn rf11_a_repository_inside_windows_old_protects_the_row() {
    let t = TempDir::new("rf11").unwrap();
    t.write("Windows.old/Users/me/proj/.git/HEAD", b"ref: refs/heads/main").unwrap();
    t.write("Windows.old/Users/me/proj/main.rs", b"fn main() {}").unwrap();
    t.write("u/.cache/pip/http/a", b"cached").unwrap();
    t.write("u/.cache/pip/src/.git/HEAD", b"ref: refs/heads/main").unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let r = rows(e, "det:app_caches");
    let row = |n: &str| r.iter().find(|r| r.candidate.path.ends_with(n)).unwrap_or_else(|| panic!("{n}: {r:#?}"));
    let old = row("Windows.old");
    assert!(old.candidate.badges.contains(mj_core::Badges::PROTECTED), "{old:?}");
    assert!(!old.candidate.badges.contains(mj_core::Badges::REBUILDABLE));
    // A real cache still owns what is inside it.
    let pip = row("pip");
    assert!(!pip.candidate.badges.contains(mj_core::Badges::PROTECTED), "{pip:?}");
}
