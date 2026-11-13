//! Integration tests: run the built `myjanitor-cli` against a temp data folder.
//! Only commands whose engine bodies exist are exercised for real; the
//! scan, apply, undo pipeline is `#[ignore]`d until the engine facade lands.
//! Nothing here installs schedules or registry entries (OS integration is
//! switched off through the environment) or touches a folder outside a temp dir.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use mj_core::testutil::TempDir;

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn cli(data: &Path, home: &Path, args: &[&str]) -> Run {
    let o: Output = Command::new(env!("CARGO_BIN_EXE_myjanitor-cli"))
        .args(args)
        .arg("--data-dir")
        .arg(data)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("MYJANITOR_NO_OS_INTEGRATION", "1")
        .env("MYJANITOR_UTC_OFFSET_MINUTES", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    Run { code: o.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&o.stdout).into(), err: String::from_utf8_lossy(&o.stderr).into() }
}

fn sandbox(label: &str) -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let t = TempDir::new(label).unwrap();
    let data = t.join("state/data");
    let home = t.mkdir("home").unwrap();
    (t, data, home)
}

#[test]
fn version_and_help() {
    let (_t, data, home) = sandbox("cli-help");
    let v = cli(&data, &home, &["--version"]);
    assert_eq!(v.code, 0);
    assert!(v.out.starts_with("myjanitor-cli 0."));
    let h = cli(&data, &home, &["--help"]);
    assert_eq!(h.code, 0);
    for cmd in ["scan", "list", "show", "search", "plan", "dry-run", "apply", "history", "undo", "report", "run-profile", "tools", "settings", "cache"] {
        assert!(h.out.contains(cmd), "help lists {cmd}");
    }
    let a = cli(&data, &home, &["apply", "--help"]);
    assert_eq!(a.code, 0);
    assert!(a.out.contains("--confirm-lossy") && a.out.contains("--confirm-permanent") && a.out.contains("--originals"));
    assert_eq!(cli(&data, &home, &["help", "plan"]).code, 0);
}

#[test]
fn usage_errors_exit_2_with_a_useful_message() {
    let (_t, data, home) = sandbox("cli-usage");
    for args in [&["frobnicate"][..], &["scan"], &["list", "--bogus"], &["plan"], &["settings", "set", "x"]] {
        let r = cli(&data, &home, args);
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        assert!(r.err.starts_with("error: "), "{}", r.err);
    }
}

#[test]
fn settings_round_trip_through_the_binary() {
    let (_t, data, home) = sandbox("cli-settings");
    assert_eq!(cli(&data, &home, &["settings", "get", "performance.threads"]).out.trim(), "0");
    let set = cli(&data, &home, &["settings", "set", "performance.threads", "6"]);
    assert_eq!(set.code, 0, "{}", set.err);
    assert_eq!(cli(&data, &home, &["settings", "get", "performance.threads"]).out.trim(), "6");
    assert!(data.join("settings.json").is_file());
    let low = cli(&data, &home, &["settings", "set", "performance.low_memory", "true"]);
    assert_eq!(low.code, 0);
    let all = cli(&data, &home, &["settings", "get"]);
    let v: serde_json::Value = serde_json::from_str(&all.out).unwrap();
    assert_eq!(v["performance"]["threads"], 6);
    assert_eq!(v["performance"]["low_memory"], true);
    let list = cli(&data, &home, &["settings", "set", "safety.exclusions", "[\"**/node_modules\"]"]);
    assert_eq!(list.code, 0, "{}", list.err);
}

#[test]
fn settings_errors_are_clear_and_change_nothing() {
    let (_t, data, home) = sandbox("cli-settings-bad");
    let r = cli(&data, &home, &["settings", "set", "performance.threads", "99999"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("performance.threads") || r.err.contains("does not accept"), "{}", r.err);
    assert_eq!(cli(&data, &home, &["settings", "set", "nosuch.key", "1"]).code, 1);
    assert_eq!(cli(&data, &home, &["settings", "set", "performance.nosuch", "1"]).code, 1);
    assert_eq!(cli(&data, &home, &["settings", "get", "nosuch"]).code, 1);
    let rel = cli(&data, &home, &["settings", "set", "performance.temp_dir", "relative/dir"]);
    assert_eq!(rel.code, 1);
    assert!(rel.err.contains("absolute"), "{}", rel.err);
    assert_eq!(cli(&data, &home, &["settings", "get", "performance.threads"]).out.trim(), "0");
}

#[test]
fn cache_tools_info_and_empty_lists() {
    let (_t, data, home) = sandbox("cli-misc");
    let c = cli(&data, &home, &["cache", "info"]);
    assert_eq!(c.code, 0, "{}", c.err);
    assert!(c.out.contains("0 entries"));
    assert!(cli(&data, &home, &["cache", "clear"]).out.starts_with("Cleared."));
    let j = cli(&data, &home, &["cache", "info", "--json"]);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&j.out).unwrap()["entries"], 0);
    let t = cli(&data, &home, &["tools", "--json"]);
    assert_eq!(t.code, 0, "{}", t.err);
    let tools: serde_json::Value = serde_json::from_str(&t.out).unwrap();
    assert!(tools.as_array().is_some());
    assert_eq!(cli(&data, &home, &["tools"]).code, 0);
    let info = cli(&data, &home, &["info"]);
    let v: serde_json::Value = serde_json::from_str(&info.out).unwrap();
    assert_eq!(v["data_dir"], data.to_str().unwrap());
    assert!(cli(&data, &home, &["schedule", "list"]).out.contains("No schedules"));
    assert!(cli(&data, &home, &["plan", "show"]).out.contains("plan is empty"));
    assert_eq!(cli(&data, &home, &["plan", "clear"]).code, 0);
    assert!(cli(&data, &home, &["history"]).code == 0);
}

#[test]
fn dangerous_roots_are_refused_before_anything_runs() {
    let (t, data, home) = sandbox("cli-refuse");
    let root = if cfg!(windows) { "C:\\" } else { "/" };
    let r = cli(&data, &home, &["scan", root]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("root of a file system"), "{}", r.err);
    let r = cli(&data, &home, &["scan", home.to_str().unwrap()]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("home folder"), "{}", r.err);
    // A folder that contains the data folder.
    let r = cli(&data, &home, &["scan", t.join("state").to_str().unwrap()]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("data folder"), "{}", r.err);
    let r = cli(&data, &home, &["scan", t.join("missing").to_str().unwrap()]);
    assert_eq!(r.code, 1);
    // `clean` and `search` share the check.
    assert_eq!(cli(&data, &home, &["clean", root, "--detectors", "temp_files"]).code, 1);
    assert_eq!(cli(&data, &home, &["search", "ext:tmp", root]).code, 1);
    assert!(!data.join("cli-session.json").exists(), "a refused scan stores nothing");
}

#[test]
fn apply_refuses_without_yes_when_not_a_terminal() {
    let (_t, data, home) = sandbox("cli-apply-refuse");
    let r = cli(&data, &home, &["apply"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("--yes"), "{}", r.err);
    let r = cli(&data, &home, &["clean", home.to_str().unwrap(), "--detectors", "x", "--apply"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("--yes"), "{}", r.err);
    assert_eq!(cli(&data, &home, &["apply", "--originals", "delete", "--yes"]).code, 2);
    let r = cli(&data, &home, &["apply", "--yes"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("no scan yet"), "{}", r.err);
    assert_eq!(cli(&data, &home, &["clean", home.to_str().unwrap()]).code, 2, "--detectors is required");
}

#[test]
fn context_menu_and_schedule_installs_honour_the_kill_switch() {
    let (_t, data, home) = sandbox("cli-os");
    let r = cli(&data, &home, &["context-menu", "install"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("MYJANITOR_NO_OS_INTEGRATION"), "{}", r.err);
    assert_eq!(cli(&data, &home, &["context-menu", "status"]).code, 0);
    assert_eq!(cli(&data, &home, &["schedule", "install", "nothing"]).code, 1);
}

/// Pipeline over a real tree: scan, list, plan, dry-run, apply, undo.
/// Needs the engine's scan/plan/apply/undo bodies (W3-D1).
#[test]
#[ignore = "needs W3-D1 engine bodies"]
fn scan_list_apply_undo_pipeline() {
    let (t, data, home) = sandbox("cli-pipeline");
    let tree = t.mkdir("photos").unwrap();
    let bytes = vec![7u8; 300_000];
    t.write("photos/a.bin", &bytes).unwrap();
    t.write("photos/copy of a.bin", &bytes).unwrap();
    let before = mj_core::testutil::snapshot(&tree).unwrap();
    let p = tree.to_str().unwrap();
    let scan = cli(&data, &home, &["scan", p]);
    assert_eq!(scan.code, 0, "{}", scan.err);
    let list = cli(&data, &home, &["list", "--json"]);
    assert_eq!(list.code, 0, "{}", list.err);
    assert_eq!(cli(&data, &home, &["plan", "add", "det:exact_duplicates"]).code, 0);
    assert_eq!(cli(&data, &home, &["dry-run"]).code, 0);
    assert_eq!(mj_core::testutil::snapshot(&tree).unwrap(), before, "a dry run changes nothing");
    let apply = cli(&data, &home, &["apply", "--yes", "--originals", "quarantine"]);
    assert_eq!(apply.code, 0, "{}", apply.err);
    assert_ne!(mj_core::testutil::snapshot(&tree).unwrap(), before);
    assert_eq!(cli(&data, &home, &["history"]).code, 0);
    let undo = cli(&data, &home, &["undo", "--last"]);
    assert_eq!(undo.code, 0, "{}", undo.err);
    assert_eq!(mj_core::testutil::snapshot(&tree).unwrap(), before, "undo restores the tree");
}
