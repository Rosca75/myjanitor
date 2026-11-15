//! Small OS helpers shared by the context-menu and scheduler code: a command
//! description, a runner abstraction (so tests never start a process), file
//! writing and the kill switch for OS integration. Owner: W3-E.

use std::path::Path;

use mj_core::{Error, Result};

/// One external command: a program and its argument vector (no shell involved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
}

impl Cmd {
    /// Build a command.
    pub fn new(program: &str, args: Vec<String>) -> Cmd {
        Cmd { program: program.to_string(), args }
    }

    /// A readable one-line form for logs and "suggested command" output (not for execution).
    pub fn display(&self) -> String {
        let quote = |a: &str| {
            if a.is_empty() || a.contains([' ', '"', '\t']) {
                format!("\"{}\"", a.replace('"', "\\\""))
            } else {
                a.to_string()
            }
        };
        std::iter::once(self.program.clone()).chain(self.args.iter().map(|a| quote(a))).collect::<Vec<_>>().join(" ")
    }
}

/// What a command printed and whether it succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CmdOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs external commands. The real implementation is [`StdRunner`]; tests use a recording fake.
pub trait Runner {
    /// Run `cmd` to completion. `Err` only when it could not be started.
    fn run(&self, cmd: &Cmd) -> Result<CmdOutput>;
}

/// Runs commands with `std::process::Command` (no shell, no console window on Windows).
#[derive(Debug, Default, Clone, Copy)]
pub struct StdRunner;

impl Runner for StdRunner {
    fn run(&self, cmd: &Cmd) -> Result<CmdOutput> {
        let mut c = std::process::Command::new(&cmd.program);
        c.args(&cmd.args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: do not flash a console when the GUI calls reg.exe / schtasks.exe.
            c.creation_flags(0x0800_0000);
        }
        let out = c
            .output()
            .map_err(|e| Error::unsupported(cmd.program.clone(), format!("could not start {}: {e}", cmd.program)))?;
        Ok(CmdOutput {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Run `cmd` and turn a non-zero exit into an error carrying the tool's own message.
pub fn run_checked(runner: &dyn Runner, cmd: &Cmd) -> Result<CmdOutput> {
    let out = runner.run(cmd)?;
    if out.ok {
        Ok(out)
    } else {
        let detail = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
        Err(Error::Internal(format!("`{}` failed: {detail}", cmd.display())))
    }
}

/// A file to write for OS integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: std::path::PathBuf,
    pub content: String,
    /// Mark executable (Unix): Nautilus scripts.
    pub executable: bool,
}

/// Write a generated file, creating its folder. Never follows a stale file: it is replaced.
pub fn write_generated(f: &GeneratedFile) -> Result<()> {
    if let Some(dir) = f.path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    std::fs::write(&f.path, f.content.as_bytes()).map_err(|e| Error::io(&f.path, e))?;
    set_executable(&f.path, f.executable)
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|e| Error::io(path, e))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}

/// Remove a file only when it carries `marker` (our own generated file); returns whether it was removed.
pub fn remove_if_ours(path: &Path, marker: &str) -> Result<bool> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.contains(marker) => {
            std::fs::remove_file(path).map_err(|e| Error::io(path, e))?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Environment switch that turns every OS side effect (registry, scheduler files) off.
/// Set by test harnesses and by locked-down deployments.
pub const NO_OS_INTEGRATION_VAR: &str = "MYJANITOR_NO_OS_INTEGRATION";

/// True when OS side effects are disabled by [`NO_OS_INTEGRATION_VAR`].
pub fn os_integration_disabled() -> bool {
    std::env::var_os(NO_OS_INTEGRATION_VAR).is_some_and(|v| !v.is_empty() && v != "0")
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Mutex;

    use super::*;

    /// Records commands and answers from a script (`ok` for everything by default).
    #[derive(Default)]
    pub struct FakeRunner {
        pub calls: Mutex<Vec<Cmd>>,
        pub fail_on: Option<String>,
        pub stdout: String,
    }

    impl Runner for FakeRunner {
        fn run(&self, cmd: &Cmd) -> Result<CmdOutput> {
            self.calls.lock().unwrap().push(cmd.clone());
            let fail = self.fail_on.as_ref().is_some_and(|f| cmd.args.iter().any(|a| a.contains(f.as_str())));
            Ok(CmdOutput {
                ok: !fail,
                stdout: self.stdout.clone(),
                stderr: if fail { "boom".into() } else { String::new() },
            })
        }
    }
}
