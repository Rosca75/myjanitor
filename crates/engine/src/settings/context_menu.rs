//! File-manager context menu (AUT-03). Owner: W3-E.
//!
//! * **Windows**: per-user entries under `HKCU\Software\Classes\...\shell\MyJanitor.*`,
//!   written with `reg.exe` argument vectors (no admin, no registry FFI):
//!   folders get "Optimise with MyJanitor" and "Find duplicates here", files get
//!   "Optimise with MyJanitor".
//! * **Linux**: a `.desktop` file with actions under `$XDG_DATA_HOME/applications/`
//!   and, where `$XDG_DATA_HOME/nautilus/scripts/` exists, two Nautilus scripts.
//!   Other file managers are reported as "not available" rather than guessed.
//!
//! Generation is pure (see [`plan_install`], [`plan_remove`]); [`install`] and
//! [`remove`] run a plan and are only ever called on an explicit user action.
//! Files we write carry [`MARKER`] so removal never touches a file we did not write.
//!
//! The entries launch the GUI executable with `--optimise <path>` or
//! `--duplicates <path>`; the shell must accept these launch arguments.

use std::path::{Path, PathBuf};

use mj_core::{Error, Result};

use super::os::{remove_if_ours, run_checked, write_generated, Cmd, GeneratedFile, Runner};
use crate::paths::{EnvMap, Os};

/// First-line marker of every file we generate (used before deleting one).
pub const MARKER: &str = "MyJanitor context menu entry";

/// What an entry applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Folder,
    File,
}

/// One context-menu entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Registry key name / desktop action id.
    pub id: &'static str,
    pub label: &'static str,
    /// Launch argument handed to the GUI before the selected path.
    pub flag: &'static str,
    pub targets: &'static [Target],
}

/// The entries (AUT-03: "Optimise with...", "Find duplicates here").
pub const ENTRIES: &[Entry] = &[
    Entry {
        id: "MyJanitor.Optimise",
        label: "Optimise with MyJanitor",
        flag: "--optimise",
        targets: &[Target::Folder, Target::File],
    },
    Entry {
        id: "MyJanitor.Duplicates",
        label: "Find duplicates here",
        flag: "--duplicates",
        targets: &[Target::Folder],
    },
];

/// Everything an install (or removal) will do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Commands to run (Windows: `reg.exe`).
    pub commands: Vec<Cmd>,
    /// Files to write (Linux).
    pub files: Vec<GeneratedFile>,
    /// Files to delete if they are ours (Linux, removal).
    pub remove_files: Vec<PathBuf>,
    /// Things that were skipped and why (shown to the user).
    pub skipped: Vec<String>,
}

/// Whether and how the context menu can be used here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub available: bool,
    pub installed: bool,
    /// Why it is unavailable, or a note about a partial install.
    pub reason: Option<String>,
}

fn reg_root(t: Target) -> &'static str {
    match t {
        Target::Folder => "HKCU\\Software\\Classes\\Directory\\shell",
        Target::File => "HKCU\\Software\\Classes\\*\\shell",
    }
}

/// `reg.exe` commands that create every entry for the executable at `exe`.
pub fn windows_add_commands(exe: &Path) -> Vec<Cmd> {
    let exe_s = exe.display().to_string();
    let mut out = Vec::new();
    for e in ENTRIES {
        for t in e.targets {
            let key = format!("{}\\{}", reg_root(*t), e.id);
            let reg = |args: &[&str]| Cmd::new("reg.exe", args.iter().map(|s| s.to_string()).collect());
            out.push(reg(&["add", &key, "/ve", "/d", e.label, "/f"]));
            out.push(reg(&["add", &key, "/v", "Icon", "/d", &format!("{exe_s},0"), "/f"]));
            let command = format!("\"{exe_s}\" {} \"%1\"", e.flag);
            out.push(reg(&["add", &format!("{key}\\command"), "/ve", "/d", &command, "/f"]));
        }
    }
    out
}

/// `reg.exe` commands that delete every entry (deleting a key removes its `command` child).
pub fn windows_remove_commands() -> Vec<Cmd> {
    let mut out = Vec::new();
    for e in ENTRIES {
        for t in e.targets {
            let key = format!("{}\\{}", reg_root(*t), e.id);
            out.push(Cmd::new("reg.exe", vec!["delete".into(), key, "/f".into()]));
        }
    }
    out
}

/// The query that tells whether the first entry exists.
pub fn windows_query_command() -> Cmd {
    let e = ENTRIES[0];
    Cmd::new("reg.exe", vec!["query".into(), format!("{}\\{}", reg_root(e.targets[0]), e.id)])
}

/// Quote one argument of a freedesktop `Exec=` line (Desktop Entry spec: double quotes, backslash-escape `"` `` ` `` `$` `\`).
pub fn desktop_exec_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Quote for a POSIX shell: single quotes, `'` becomes `'\''`.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Where Linux files go: `$XDG_DATA_HOME` or `~/.local/share`.
pub fn linux_data_home(env: &EnvMap) -> Option<PathBuf> {
    let var = |k: &str| env.get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("XDG_DATA_HOME").filter(|p| p.is_absolute()).or_else(|| var("HOME").map(|h| h.join(".local/share")))
}

/// Path of the generated `.desktop` file.
pub fn desktop_file_path(data_home: &Path) -> PathBuf {
    data_home.join("applications").join("myjanitor-actions.desktop")
}

/// Nautilus script paths (one per entry that applies to folders).
pub fn nautilus_script_path(data_home: &Path, e: &Entry) -> PathBuf {
    data_home.join("nautilus").join("scripts").join(e.label)
}

/// The `.desktop` file content: a hidden application entry whose actions are the menu entries.
pub fn desktop_file(exe: &Path) -> String {
    let exe_q = desktop_exec_quote(&exe.display().to_string());
    let actions = ENTRIES.iter().map(|e| e.id.replace("MyJanitor.", "")).collect::<Vec<_>>().join(";");
    let mut s = format!(
        "[Desktop Entry]\n# {MARKER}\nType=Application\nName=MyJanitor\nExec={exe_q} %f\nIcon=myjanitor\nTerminal=false\nNoDisplay=true\nCategories=Utility;\nMimeType=inode/directory;\nActions={actions};\n"
    );
    for e in ENTRIES {
        s.push_str(&format!(
            "\n[Desktop Action {}]\nName={}\nExec={exe_q} {} %f\n",
            e.id.replace("MyJanitor.", ""),
            e.label,
            e.flag
        ));
    }
    s
}

/// A Nautilus script: Nautilus passes the selection as arguments.
pub fn nautilus_script(exe: &Path, e: &Entry) -> String {
    format!(
        "#!/bin/sh\n# {MARKER}\nfor p in \"$@\"; do\n  exec {} {} \"$p\"\ndone\n",
        sh_quote(&exe.display().to_string()),
        e.flag
    )
}

/// What installing does on `os`. `nautilus_scripts_exist` says whether
/// `<data home>/nautilus/scripts` is a folder (Linux only).
pub fn plan_install(os: Os, env: &EnvMap, exe: &Path, nautilus_scripts_exist: bool) -> Result<Plan> {
    match os {
        Os::Windows => Ok(Plan { commands: windows_add_commands(exe), ..Plan::default() }),
        Os::Linux => {
            let home = linux_data_home(env)
                .ok_or_else(|| Error::unsupported("context menu", "no home folder: set HOME or XDG_DATA_HOME"))?;
            let mut plan = Plan::default();
            plan.files.push(GeneratedFile {
                path: desktop_file_path(&home),
                content: desktop_file(exe),
                executable: false,
            });
            if nautilus_scripts_exist {
                for e in ENTRIES.iter().filter(|e| e.targets.contains(&Target::Folder)) {
                    plan.files.push(GeneratedFile {
                        path: nautilus_script_path(&home, e),
                        content: nautilus_script(exe, e),
                        executable: true,
                    });
                }
            } else {
                plan.skipped
                    .push("Nautilus scripts folder not found; the file manager entries were not installed".into());
            }
            Ok(plan)
        }
    }
}

/// What removing does on `os`.
pub fn plan_remove(os: Os, env: &EnvMap) -> Result<Plan> {
    match os {
        Os::Windows => Ok(Plan { commands: windows_remove_commands(), ..Plan::default() }),
        Os::Linux => {
            let home = linux_data_home(env)
                .ok_or_else(|| Error::unsupported("context menu", "no home folder: set HOME or XDG_DATA_HOME"))?;
            let mut plan = Plan { remove_files: vec![desktop_file_path(&home)], ..Plan::default() };
            plan.remove_files.extend(
                ENTRIES.iter().filter(|e| e.targets.contains(&Target::Folder)).map(|e| nautilus_script_path(&home, e)),
            );
            Ok(plan)
        }
    }
}

/// Run an install plan. Windows: commands in order, stopping at the first failure
/// (after which a removal is attempted so no half-registered menu stays behind).
pub fn execute_install(plan: &Plan, runner: &dyn Runner) -> Result<()> {
    for (i, c) in plan.commands.iter().enumerate() {
        if let Err(e) = run_checked(runner, c) {
            if i > 0 {
                for undo in windows_remove_commands() {
                    let _ = runner.run(&undo);
                }
            }
            return Err(e);
        }
    }
    plan.files.iter().try_for_each(write_generated)
}

/// Run a removal plan. Missing registry keys are not an error (removal is idempotent).
pub fn execute_remove(plan: &Plan, runner: &dyn Runner) -> Result<()> {
    for c in &plan.commands {
        runner.run(c)?; // a non-zero exit means "key not found": already removed
    }
    for f in &plan.remove_files {
        remove_if_ours(f, MARKER)?;
    }
    Ok(())
}

/// Whether the menu is installed, using `runner` for the registry query on Windows.
pub fn query_status(os: Os, env: &EnvMap, runner: &dyn Runner) -> Status {
    match os {
        Os::Windows => {
            let installed = runner.run(&windows_query_command()).map(|o| o.ok).unwrap_or(false);
            Status { available: true, installed, reason: None }
        }
        Os::Linux => match linux_data_home(env) {
            None => Status {
                available: false,
                installed: false,
                reason: Some("no home folder (HOME and XDG_DATA_HOME are unset)".into()),
            },
            Some(home) => {
                let installed = std::fs::read_to_string(desktop_file_path(&home)).is_ok_and(|t| t.contains(MARKER));
                let nautilus = home.join("nautilus").join("scripts").is_dir();
                let reason = (!nautilus).then(|| {
                    "No Nautilus scripts folder: only the application entry is used by other file managers".to_string()
                });
                Status { available: true, installed, reason }
            }
        },
    }
}

/// Resolve the GUI executable the menu entries launch. When the current process
/// is the CLI, the GUI is its sibling `myjanitor(.exe)`.
pub fn resolve_gui_exe(current: &Path) -> Result<PathBuf> {
    let stem = current.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if stem != "myjanitor-cli" {
        return Ok(current.to_path_buf());
    }
    let sibling = current.with_file_name(if cfg!(windows) { "myjanitor.exe" } else { "myjanitor" });
    if sibling.is_file() {
        Ok(sibling)
    } else {
        Err(Error::unsupported(
            "context menu",
            format!("the application executable was not found next to the CLI ({}); enable the menu from the app's Settings", sibling.display()),
        ))
    }
}

/// Install for this machine from the current process (explicit user action only).
pub fn install(runner: &dyn Runner) -> Result<Vec<String>> {
    if super::os::os_integration_disabled() {
        return Err(Error::unsupported("context menu", format!("disabled by {}", super::os::NO_OS_INTEGRATION_VAR)));
    }
    let exe = resolve_gui_exe(&std::env::current_exe().map_err(|e| Error::Internal(e.to_string()))?)?;
    let env = crate::paths::current_env();
    let os = Os::current();
    let nautilus = linux_data_home(&env).is_some_and(|h| h.join("nautilus").join("scripts").is_dir());
    let plan = plan_install(os, &env, &exe, nautilus)?;
    execute_install(&plan, runner)?;
    Ok(plan.skipped)
}

/// Remove for this machine (explicit user action only).
pub fn remove(runner: &dyn Runner) -> Result<()> {
    if super::os::os_integration_disabled() {
        return Ok(());
    }
    execute_remove(&plan_remove(Os::current(), &crate::paths::current_env())?, runner)
}

#[cfg(test)]
mod tests {
    use mj_core::testutil::TempDir;

    use super::*;
    use crate::settings::os::fake::FakeRunner;

    fn env(pairs: &[(&str, &str)]) -> EnvMap {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn windows_commands_are_per_user_reg_exe_vectors() {
        let cmds = windows_add_commands(Path::new("C:\\Program Files\\MyJanitor\\myjanitor.exe"));
        assert!(cmds.iter().all(|c| c.program == "reg.exe"));
        let keys: Vec<&str> = cmds
            .iter()
            .filter(|c| c.args.iter().any(|a| a == "/ve") && !c.args[1].ends_with("command"))
            .map(|c| c.args[1].as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "HKCU\\Software\\Classes\\Directory\\shell\\MyJanitor.Optimise",
                "HKCU\\Software\\Classes\\*\\shell\\MyJanitor.Optimise",
                "HKCU\\Software\\Classes\\Directory\\shell\\MyJanitor.Duplicates",
            ]
        );
        // Nothing outside HKCU, so no admin rights are needed.
        assert!(cmds.iter().all(|c| c.args[1].starts_with("HKCU\\")));
        let command = cmds.iter().find(|c| c.args[1].ends_with("Duplicates\\command")).unwrap();
        assert_eq!(command.args[4], "\"C:\\Program Files\\MyJanitor\\myjanitor.exe\" --duplicates \"%1\"");
        assert!(cmds.iter().any(|c| c.args.contains(&"Optimise with MyJanitor".to_string())));
        assert!(cmds.iter().any(|c| c.args.contains(&"Find duplicates here".to_string())));
    }

    #[test]
    fn windows_removal_deletes_each_key_and_query_checks_the_first() {
        let del = windows_remove_commands();
        assert_eq!(del.len(), 3);
        assert!(del
            .iter()
            .all(|c| c.args[0] == "delete" && c.args.last().unwrap() == "/f" && c.args[1].starts_with("HKCU\\")));
        assert_eq!(windows_query_command().args[0], "query");
    }

    #[test]
    fn desktop_file_has_actions_and_quotes_the_executable() {
        let d = desktop_file(Path::new("/opt/my janitor/myjanitor"));
        assert!(d.starts_with("[Desktop Entry]\n"));
        assert!(d.contains(MARKER));
        assert!(d.contains("Actions=Optimise;Duplicates;"));
        assert!(d.contains(
            "[Desktop Action Optimise]\nName=Optimise with MyJanitor\nExec=\"/opt/my janitor/myjanitor\" --optimise %f"
        ));
        assert!(d.contains("Name=Find duplicates here"));
        assert_eq!(desktop_exec_quote("a\"b$c"), "\"a\\\"b\\$c\"");
    }

    #[test]
    fn nautilus_script_is_a_quoted_shell_script() {
        let s = nautilus_script(Path::new("/opt/it's/myjanitor"), &ENTRIES[1]);
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("exec '/opt/it'\\''s/myjanitor' --duplicates \"$p\""));
        assert!(s.contains(MARKER));
    }

    #[test]
    fn linux_plan_skips_nautilus_when_its_folder_is_missing() {
        let e = env(&[("HOME", "/home/m")]);
        let p = plan_install(Os::Linux, &e, Path::new("/opt/mj"), false).unwrap();
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].path, PathBuf::from("/home/m/.local/share/applications/myjanitor-actions.desktop"));
        assert_eq!(p.skipped.len(), 1);
        let p = plan_install(Os::Linux, &e, Path::new("/opt/mj"), true).unwrap();
        assert_eq!(p.files.len(), 3);
        assert!(p.files[1].executable);
        assert_eq!(p.files[1].path, PathBuf::from("/home/m/.local/share/nautilus/scripts/Optimise with MyJanitor"));
        assert!(plan_install(Os::Linux, &env(&[]), Path::new("/opt/mj"), true).is_err());
    }

    #[test]
    fn install_and_remove_write_only_inside_the_given_data_home() {
        let t = TempDir::new("ctxmenu").unwrap();
        t.mkdir("share/nautilus/scripts").unwrap();
        let e = env(&[("XDG_DATA_HOME", t.join("share").to_str().unwrap())]);
        let plan = plan_install(Os::Linux, &e, Path::new("/opt/mj/myjanitor"), true).unwrap();
        execute_install(&plan, &FakeRunner::default()).unwrap();
        let desktop = t.join("share/applications/myjanitor-actions.desktop");
        assert!(desktop.is_file());
        assert!(t.join("share/nautilus/scripts/Find duplicates here").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode =
                std::fs::metadata(t.join("share/nautilus/scripts/Find duplicates here")).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        assert!(query_status(Os::Linux, &e, &FakeRunner::default()).installed);
        // A foreign file with the same name as one of ours is left alone.
        std::fs::write(t.join("share/nautilus/scripts/Optimise with MyJanitor"), "#!/bin/sh\necho mine\n").unwrap();
        execute_remove(&plan_remove(Os::Linux, &e).unwrap(), &FakeRunner::default()).unwrap();
        assert!(!desktop.exists());
        assert!(!t.join("share/nautilus/scripts/Find duplicates here").exists());
        assert!(t.join("share/nautilus/scripts/Optimise with MyJanitor").exists());
        assert!(!query_status(Os::Linux, &e, &FakeRunner::default()).installed);
        // Removal is idempotent.
        execute_remove(&plan_remove(Os::Linux, &e).unwrap(), &FakeRunner::default()).unwrap();
    }

    #[test]
    fn windows_install_runs_every_command_and_rolls_back_on_failure() {
        let plan = plan_install(Os::Windows, &env(&[]), Path::new("C:\\mj\\myjanitor.exe"), false).unwrap();
        let ok = FakeRunner::default();
        execute_install(&plan, &ok).unwrap();
        assert_eq!(ok.calls.lock().unwrap().len(), plan.commands.len());

        let failing = FakeRunner { fail_on: Some("Duplicates\\command".into()), ..FakeRunner::default() };
        assert!(execute_install(&plan, &failing).is_err());
        let calls = failing.calls.lock().unwrap();
        assert!(calls.iter().any(|c| c.args[0] == "delete"), "a failed install removes what it added");
    }

    #[test]
    fn windows_status_follows_the_registry_query() {
        let found = FakeRunner::default();
        assert!(query_status(Os::Windows, &env(&[]), &found).installed);
        let missing = FakeRunner { fail_on: Some("HKCU".into()), ..FakeRunner::default() };
        assert!(!query_status(Os::Windows, &env(&[]), &missing).installed);
    }

    #[test]
    fn the_cli_resolves_the_gui_next_to_itself() {
        let t = TempDir::new("ctxexe").unwrap();
        let cli = t.write(if cfg!(windows) { "myjanitor-cli.exe" } else { "myjanitor-cli" }, b"").unwrap();
        assert!(resolve_gui_exe(&cli).is_err());
        let gui = t.write(if cfg!(windows) { "myjanitor.exe" } else { "myjanitor" }, b"").unwrap();
        assert_eq!(resolve_gui_exe(&cli).unwrap(), gui);
        assert_eq!(resolve_gui_exe(&gui).unwrap(), gui);
    }
}
