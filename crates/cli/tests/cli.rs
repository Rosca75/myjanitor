//! Integration tests: run the built `myjanitor-cli` against a temp data folder.
//! The scan → plan → apply → undo pipeline runs across separate processes against
//! a generated tree; every removal goes to the test's own quarantine.
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
        // The freedesktop Trash would follow XDG_DATA_HOME: keep it inside the sandbox too.
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("MYJANITOR_NO_OS_INTEGRATION", "1")
        .env("MYJANITOR_UTC_OFFSET_MINUTES", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    Run {
        code: o.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&o.stdout).into(),
        err: String::from_utf8_lossy(&o.stderr).into(),
    }
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
    for cmd in [
        "scan",
        "list",
        "show",
        "search",
        "plan",
        "dry-run",
        "apply",
        "history",
        "undo",
        "report",
        "run-profile",
        "tools",
        "settings",
        "cache",
    ] {
        assert!(h.out.contains(cmd), "help lists {cmd}");
    }
    let a = cli(&data, &home, &["apply", "--help"]);
    assert_eq!(a.code, 0);
    assert!(
        a.out.contains("--confirm-lossy") && a.out.contains("--confirm-permanent") && a.out.contains("--originals")
    );
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

/// Pipeline over a real tree across separate processes: scan, list, show, plan add,
/// dry-run, apply --yes, history, report, undo; then the `clean` one-shot form.
#[test]
fn scan_list_apply_undo_pipeline() {
    let (t, data, home) = sandbox("cli-pipeline");
    let tree = t.mkdir("photos").unwrap();
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
    t.write("photos/a.bin", &bytes).unwrap();
    t.write("photos/copy of a.bin", &bytes).unwrap();
    let before = mj_core::testutil::snapshot(&tree).unwrap();
    let p = tree.to_str().unwrap();
    let scan = cli(&data, &home, &["scan", p]);
    assert_eq!(scan.code, 0, "{}", scan.err);
    let list = cli(&data, &home, &["list", "--json"]);
    assert_eq!(list.code, 0, "{}", list.err);
    let v: serde_json::Value = serde_json::from_str(&list.out).unwrap();
    assert!(list.out.contains("det:exact_duplicates"), "{v}");
    let show = cli(&data, &home, &["show", "det:exact_duplicates", "--json"]);
    assert_eq!(show.code, 0, "{}", show.err);
    assert!(show.out.contains("a.bin"), "{}", show.out);
    let add = cli(&data, &home, &["plan", "add", "det:exact_duplicates", "--action", "quarantine"]);
    assert_eq!(add.code, 0, "{}", add.err);
    let dry = cli(&data, &home, &["dry-run"]);
    assert_eq!(dry.code, 0, "{}", dry.err);
    assert_eq!(mj_core::testutil::snapshot(&tree).unwrap(), before, "a dry run changes nothing");
    let apply = cli(&data, &home, &["apply", "--yes"]);
    assert_eq!(apply.code, 0, "{}{}", apply.out, apply.err);
    let after = mj_core::testutil::snapshot(&tree).unwrap();
    assert_eq!(after.len(), 1, "one copy left: {after:?}");
    assert!(std::fs::read_dir(data.join("quarantine")).unwrap().next().is_some(), "held in the quarantine");
    let hist = cli(&data, &home, &["history", "--json"]);
    assert_eq!(hist.code, 0, "{}", hist.err);
    let h: serde_json::Value = serde_json::from_str(&hist.out).unwrap();
    assert_eq!(h["runs"][0]["undoable"], true, "{}", hist.out);
    let report = cli(&data, &home, &["report"]);
    assert_eq!(report.code, 0, "{}", report.err);
    let undo = cli(&data, &home, &["undo", "--last"]);
    assert_eq!(undo.code, 0, "{}{}", undo.out, undo.err);
    assert_eq!(mj_core::testutil::snapshot(&tree).unwrap(), before, "undo restores the tree");
    assert_eq!(cli(&data, &home, &["undo", "--last"]).code, 1, "nothing left to undo");

    // One-shot form: estimate only by default, then apply.
    let est = cli(&data, &home, &["clean", p, "--detectors", "exact_duplicates"]);
    assert_eq!(est.code, 0, "{}{}", est.out, est.err);
    assert_eq!(mj_core::testutil::snapshot(&tree).unwrap(), before, "clean without --apply changes nothing");
    let unknown = cli(&data, &home, &["clean", p, "--detectors", "no_such_detector", "--dry-run"]);
    assert_ne!(unknown.code, 0, "an unknown detector is an error: {}", unknown.out);
    assert_eq!(mj_core::testutil::snapshot(&tree).unwrap(), before);
}

#[test]
fn quarantine_list_and_purge() {
    let (t, data, home) = sandbox("cli-quarantine");
    let tree = t.mkdir("photos").unwrap();
    let bytes: Vec<u8> = (0..50_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
    t.write("photos/a.bin", &bytes).unwrap();
    t.write("photos/b.bin", &bytes).unwrap();
    let p = tree.to_str().unwrap();
    assert_eq!(cli(&data, &home, &["quarantine", "list"]).code, 0);
    assert_eq!(cli(&data, &home, &["scan", p]).code, 0);
    assert_eq!(cli(&data, &home, &["plan", "add", "det:exact_duplicates", "--action", "quarantine"]).code, 0);
    let apply = cli(&data, &home, &["apply", "--yes"]);
    assert_eq!(apply.code, 0, "{}{}", apply.out, apply.err);
    let list = cli(&data, &home, &["quarantine", "list", "--json"]);
    assert_eq!(list.code, 0, "{}", list.err);
    let v: serde_json::Value = serde_json::from_str(&list.out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1, "{v}");
    // Not expired yet: the plain purge keeps it.
    let purge = cli(&data, &home, &["quarantine", "purge"]);
    assert_eq!(purge.code, 0, "{}{}", purge.out, purge.err);
    assert!(purge.out.contains("1 item(s) remain"), "{}", purge.out);
    // An explicit age needs --yes off a terminal.
    assert_ne!(cli(&data, &home, &["quarantine", "purge", "--older-than", "0"]).code, 0);
    let all = cli(&data, &home, &["quarantine", "purge", "--older-than", "0", "--yes"]);
    assert_eq!(all.code, 0, "{}{}", all.out, all.err);
    assert!(all.out.contains("0 item(s) remain"), "{}", all.out);
    assert_eq!(mj_core::testutil::snapshot(&tree).unwrap().len(), 1, "the kept copy is untouched");
}

/// IPC-6: a command that changes files refuses at once while another MyJanitor holds the
/// data folder, before it rescans, measures or asks for a confirmation.
#[test]
fn acting_commands_refuse_at_once_while_another_myjanitor_holds_the_data_folder() {
    let (t, data, home) = sandbox("cli-held");
    let tree = t.mkdir("tree").unwrap();
    let tmp = t.write("tree/old.tmp", b"scratch").unwrap();
    mj_core::testutil::set_mtime(&tmp, std::time::SystemTime::now() - std::time::Duration::from_secs(86_400 * 5))
        .unwrap();
    let root = tree.to_str().unwrap();
    assert_eq!(cli(&data, &home, &["scan", root]).code, 0);
    let _held = mj_engine::instance::acquire(&data).unwrap();
    for args in [vec!["apply", "--yes"], vec!["clean", root, "--detectors", "temp_files", "--apply", "--yes"]] {
        let r = cli(&data, &home, &args);
        assert_eq!(r.code, 1, "{args:?}: {}", r.err);
        assert!(r.err.contains("another MyJanitor"), "{}", r.err);
        assert!(!r.err.to_lowercase().contains("scan"), "refused before any scan: {}", r.err);
    }
    assert!(tmp.exists());
    // Reading still works while the folder is held.
    assert_eq!(cli(&data, &home, &["list"]).code, 0);
}
