//! Shared plumbing of the commands: exit-code errors, the engine with its
//! directories, the persisted CLI session, and waiting for scans and jobs.
//! Owner: W3-E.
//!
//! **Session persistence.** The engine keeps scan results and the plan in
//! memory, so they do not survive a process. The CLI therefore stores, in
//! `<data>/cli-session.json`, the *scope* of the last `scan` and the *plan
//! templates* (opportunity id, optional action, optional parameters). Commands
//! that need results (`list`, `show`, `plan show`, `dry-run`, `apply`) re-run
//! the scan of the stored scope (fast with the scan cache) and replay the plan
//! before acting, so `scan` then `list` then `apply` works across processes.
//! `clean` does everything in one invocation and stores nothing.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mj_core::{Action, Job, OpportunityId, OriginalsPolicy, ParamValues, ScanScope};
use mj_engine::api::{
    ApplyStarted, CollectSink, PlanAddStepRequest, PlanUpdateStepRequest, ScanRunState, ScanStartRequest, ScanStatus,
    ScopeSetRequest,
};
use mj_engine::paths::AppDirs;
use mj_engine::{Engine, EngineConfig};
use serde::{Deserialize, Serialize};

use crate::args::Parsed;

/// A failed command and its exit code.
#[derive(Debug)]
pub enum Fail {
    /// Exit 2: the command line was wrong.
    Usage(String),
    /// Exit 1: something went wrong.
    Error(String),
    /// Exit 3: the run finished but some items failed.
    Partial(String),
}

impl Fail {
    /// Process exit code.
    pub fn code(&self) -> u8 {
        match self {
            Fail::Error(_) => 1,
            Fail::Usage(_) => 2,
            Fail::Partial(_) => 3,
        }
    }
    /// Message for stderr.
    pub fn message(&self) -> &str {
        match self {
            Fail::Usage(m) | Fail::Error(m) | Fail::Partial(m) => m,
        }
    }
}

impl From<mj_core::Error> for Fail {
    fn from(e: mj_core::Error) -> Fail {
        let hint = e.hint();
        Fail::Error(if hint.is_empty() { e.to_string() } else { format!("{e} ({hint})") })
    }
}

impl From<crate::args::UsageError> for Fail {
    fn from(e: crate::args::UsageError) -> Fail {
        Fail::Usage(e.0)
    }
}

/// Command result.
pub type Res<T = ()> = Result<T, Fail>;

/// One planned step as the CLI remembers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub opportunity: OpportunityId,
    #[serde(default)]
    pub action: Option<Action>,
    #[serde(default)]
    pub params: Option<ParamValues>,
}

/// What persists between invocations.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub scope: Option<ScanScope>,
    #[serde(default)]
    pub plan: Vec<Template>,
}

/// Everything a command needs.
pub struct Ctx {
    pub engine: Arc<Engine>,
    pub sink: Arc<CollectSink>,
    pub dirs: AppDirs,
    pub json: bool,
}

/// The directories the command line selects: `--data-dir` / `--cache-dir`, else the OS ones.
pub fn dirs_for(a: &Parsed) -> AppDirs {
    let mut d = match a.value("data-dir") {
        Some(root) => AppDirs::under(&PathBuf::from(root)),
        None => mj_engine::paths::detect(),
    };
    if let Some(c) = a.value("cache-dir") {
        d.cache = PathBuf::from(c);
        d.temp = d.cache.join("tmp");
    }
    d
}

impl Ctx {
    /// Build the engine for this invocation.
    pub fn open(a: &Parsed) -> Res<Ctx> {
        let dirs = dirs_for(a);
        let sink = Arc::new(CollectSink::default());
        let mut cfg = EngineConfig::new(dirs.data.clone(), dirs.cache.clone());
        cfg.portable = dirs.portable;
        cfg.events = sink.clone();
        let engine = Engine::new(cfg)?;
        // AUT-07: a configured temp folder becomes this process's temp folder.
        if engine.settings_get()?.performance.temp_dir.is_some() {
            let t = engine.temp_dir();
            if std::fs::create_dir_all(&t).is_ok() {
                std::env::set_var("TMPDIR", &t);
                std::env::set_var("TEMP", &t);
                std::env::set_var("TMP", &t);
            }
        }
        Ok(Ctx { engine, sink, dirs, json: a.flag("json") })
    }

    fn session_path(&self) -> PathBuf {
        self.dirs.data.join("cli-session.json")
    }

    /// The stored session (empty when missing or unreadable).
    pub fn session(&self) -> Session {
        std::fs::read(self.session_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Store the session atomically.
    pub fn save_session(&self, s: &Session) -> Res {
        let p = self.session_path();
        let tmp = p.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(s).map_err(|e| Fail::Error(e.to_string()))?;
        std::fs::create_dir_all(&self.dirs.data)
            .and_then(|_| std::fs::write(&tmp, data))
            .and_then(|_| std::fs::rename(&tmp, &p))
            .map_err(|e| Fail::Error(format!("could not save {}: {e}", p.display())))
    }

    /// Run a scan of `scope` and wait for it.
    pub fn run_scan(&self, scope: ScanScope) -> Res<ScanStatus> {
        let e = &self.engine;
        e.scope_set(ScopeSetRequest { scope })?;
        e.scan_start(ScanStartRequest::default())?;
        let status = loop {
            let s = e.scan_status()?;
            if !matches!(s.state, ScanRunState::Scanning | ScanRunState::Paused | ScanRunState::Stopping) {
                break s;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        match (&status.state, &status.error) {
            (_, Some(err)) => Err(Fail::Error(format!("{} ({})", err.message, err.hint))),
            (ScanRunState::Done, None) => Ok(status),
            (other, None) => Err(Fail::Error(format!("the scan ended as {other:?}"))),
        }
    }

    /// Re-run the stored scan (quietly) so results exist in this process.
    pub fn ensure_scanned(&self) -> Res<Session> {
        let session = self.session();
        let scope = session.scope.clone().ok_or_else(|| {
            Fail::Error("no scan yet: run `myjanitor-cli scan <FOLDER>...` first (or use `clean`)".into())
        })?;
        if !self.json {
            eprintln!("Re-reading the last scan (the scan cache makes this fast)...");
        }
        self.run_scan(scope)?;
        Ok(session)
    }

    /// Put the stored plan templates into the engine's plan (clearing it first).
    /// `originals` rewrites the originals policy of optimise and archive steps.
    pub fn replay_plan(&self, templates: &[Template], originals: Option<&OriginalsPolicy>) -> Res {
        let e = &self.engine;
        e.plan_clear()?;
        for t in templates {
            e.plan_add_step(PlanAddStepRequest {
                opportunity_id: t.opportunity.clone(),
                action: t.action.clone(),
                params: t.params.clone(),
            })
            .map_err(|err| {
                Fail::Error(format!(
                    "{}: {err}; remove it with `plan remove {}` or scan again",
                    t.opportunity.0, t.opportunity.0
                ))
            })?;
        }
        if let Some(p) = originals {
            for step in e.plan_get()?.steps {
                let action = match step.action {
                    Action::Optimise { optimiser, params, output, keep_if_not_smaller, .. } => {
                        Action::Optimise { optimiser, params, output, originals: p.clone(), keep_if_not_smaller }
                    }
                    Action::Archive { format, level, dest, keep_sources, .. } => {
                        Action::Archive { format, level, dest, originals: p.clone(), keep_sources }
                    }
                    _ => continue,
                };
                e.plan_update_step(PlanUpdateStepRequest { step_id: step.id, action: Some(action), params: None })?;
            }
        }
        Ok(())
    }

    /// Wait for the given jobs to finish; progress goes to stderr (not with `--json`).
    pub fn wait_jobs(&self, ids: &[mj_core::JobId]) -> Res<Vec<Job>> {
        let mut last = Instant::now();
        loop {
            let jobs: Vec<Job> = self.engine.jobs_list()?.into_iter().filter(|j| ids.contains(&j.id)).collect();
            if jobs.len() == ids.len() && jobs.iter().all(|j| j.state.is_finished()) {
                return Ok(jobs);
            }
            if !self.json && last.elapsed() > Duration::from_secs(2) {
                let (d, t) = jobs.iter().fold((0, 0), |(d, t), j| (d + j.progress.done, t + j.progress.total));
                eprintln!("  working: {d} of {t} items");
                last = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Wait for the jobs of an apply.
    pub fn wait_apply(&self, s: &ApplyStarted) -> Res<Vec<Job>> {
        let ids: Vec<_> = s.jobs.iter().map(|j| j.id).collect();
        self.wait_jobs(&ids)
    }
}

/// True when stdin and stdout are terminals (a typed confirmation is possible).
pub fn interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Ask for a typed word; true only when the user typed exactly `word`.
pub fn confirm_typed(prompt: &str, word: &str) -> bool {
    print!("{prompt} Type {word} to continue: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).is_ok() && line.trim() == word
}
