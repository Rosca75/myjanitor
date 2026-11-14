//! History, undo and quarantine (SAF-03, CLN-ACT-04). Owner: W3-D.
//!
//! Runs and entries come from the persistent journal. Undo runs as a queued job (it
//! changes files) in its own undo run (`RunKind::Undo`, `undo_of`). The guard for an
//! undo must cover the originals: the scope roots recorded for the run at apply time
//! (`run-meta.json`), or, for runs without metadata, each original's parent folder.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use mj_core::{lock, EntryId, Error, Job, JobKind, JournalState, Result, RunId, RunKind, Timestamp};
use serde::{Deserialize, Serialize};

use crate::api::{
    Event, HistoryChanged, HistoryList, HistoryRun, HistoryRunDetail, PageRequest, QuarantineInfo,
    QuarantinePurgeRequest, RestoreEntriesRequest, RunDetailRequest, RunRequest,
};
use crate::jobs::{JobResult, Work};
use crate::Engine;

const META_FILE: &str = "run-meta.json";

/// What the engine remembers about a run beyond the journal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMeta {
    /// Allowed roots of the run (the scanned roots).
    #[serde(with = "mj_core::pathutil::serde_path::vec")]
    pub roots: Vec<PathBuf>,
    /// `(step id, title)` in plan order (report sections).
    pub steps: Vec<(u64, String)>,
}

pub(crate) fn load_run_meta(engine: &Engine) {
    if engine.cfg.persist {
        if let Some(m) = crate::store::read_json::<BTreeMap<u64, RunMeta>>(&engine.cfg.data_dir.join(META_FILE)) {
            *lock(&engine.run_meta) = m;
        }
    }
}

pub(crate) fn save_run_meta(engine: &Engine) {
    if engine.cfg.persist {
        let m = lock(&engine.run_meta).clone();
        let _ = crate::store::write_json(&engine.cfg.data_dir.join(META_FILE), &m);
    }
}

impl Engine {
    pub(crate) fn remember_run(&self, run: RunId, roots: Vec<PathBuf>, steps: Vec<(u64, String)>) {
        lock(&self.run_meta).insert(run.0, RunMeta { roots, steps });
        save_run_meta(self);
    }

    /// Settle journal intents left by a crash (inspects and records only, D-2E05).
    pub(crate) fn recover_journal(&self) {
        if self.journal.pending_intents().is_empty() {
            return;
        }
        let cancel = mj_core::CancelToken::new();
        let _ = self.with_exec(Vec::new(), RunId(0), mj_core::JobId(0), None, &cancel, &mj_core::NullProgress, |ctx| {
            mj_actions::undo::recover(ctx)
        });
    }

    /// Roots covering the originals of `entries`.
    fn undo_roots(&self, entries: &[EntryId]) -> Vec<PathBuf> {
        let meta = lock(&self.run_meta).clone();
        let mut roots = Vec::new();
        for id in entries {
            let Some(e) = self.journal.entry(*id) else { continue };
            match meta.get(&e.run_id.0) {
                Some(m) if !m.roots.is_empty() => roots.extend(m.roots.iter().cloned()),
                _ => {
                    if let Some(p) = e.original.parent() {
                        roots.push(p.to_path_buf());
                    }
                }
            }
        }
        let scope = mj_core::ScanScope { roots, ..Default::default() };
        mj_scan::effective_roots(&scope)
    }

    fn history_run(&self, run: &mj_core::RunRecord, now: Timestamp) -> HistoryRun {
        let entries = self.journal.entries_of(run.run_id);
        let count = |s: JournalState| entries.iter().filter(|e| e.state == s).count() as u64;
        let undoable = entries.iter().any(|e| mj_actions::undo::is_undoable(e, now));
        HistoryRun {
            run_id: run.run_id,
            kind: run.kind,
            title: run.title.clone(),
            started: run.started,
            finished: run.finished,
            steps: run.steps.clone(),
            items: entries.len() as u64,
            done: count(JournalState::Done),
            failed: count(JournalState::Failed) + count(JournalState::UndoFailed),
            undone: count(JournalState::Undone),
            bytes_saved: entries
                .iter()
                .filter(|e| matches!(e.state, JournalState::Done | JournalState::Purged))
                .filter(|e| e.action != mj_core::ActionKind::Copy)
                .map(|e| e.bytes_before.saturating_sub(e.bytes_after))
                .sum(),
            undoable,
            undo_until: entries
                .iter()
                .filter(|e| mj_actions::undo::is_undoable(e, now))
                .filter_map(|e| e.undo_deadline)
                .max(),
        }
    }

    /// Runs, newest first.
    pub fn history_list(&self, req: PageRequest) -> Result<HistoryList> {
        let now = Timestamp::now();
        let mut runs = self.journal.runs();
        runs.sort_by_key(|r| std::cmp::Reverse(r.run_id));
        let total = runs.len() as u64;
        let runs = runs
            .iter()
            .skip(usize::try_from(req.offset).unwrap_or(usize::MAX))
            .take(req.limit.clamp(1, 500) as usize)
            .map(|r| self.history_run(r, now))
            .collect();
        Ok(HistoryList { total, runs })
    }

    /// Entries of one run.
    pub fn history_run_detail(&self, req: RunDetailRequest) -> Result<HistoryRunDetail> {
        let run =
            self.journal.run(req.run_id).ok_or_else(|| Error::InvalidInput(format!("unknown run {}", req.run_id.0)))?;
        let entries = self.journal.entries_of(run.run_id);
        let total = entries.len() as u64;
        Ok(HistoryRunDetail {
            run: self.history_run(&run, Timestamp::now()),
            total,
            entries: entries
                .into_iter()
                .skip(usize::try_from(req.offset).unwrap_or(usize::MAX))
                .take(req.limit.clamp(1, 500) as usize)
                .collect(),
        })
    }

    /// Queue an undo of `entries` as its own undo run.
    fn queue_undo(self: &Arc<Self>, of: Option<RunId>, title: String, entries: Vec<EntryId>) -> Result<Job> {
        if entries.is_empty() {
            return Err(Error::InvalidInput("nothing can be undone".into()));
        }
        if self.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::Conflict("MyJanitor is shutting down".into()));
        }
        let roots = self.undo_roots(&entries);
        let job = self.new_job(JobKind::Undo, None, None, &title, entries.len() as u64, 0);
        let body: Work = Box::new(move |engine, ctx| {
            let run = match mj_actions::undo::start_undo_run(&engine.journal, of, &title, Timestamp::now()) {
                Ok(r) => r,
                Err(e) => return JobResult { error: Some(e), ..Default::default() },
            };
            {
                let mut st = lock(&engine.jobs);
                if let Some(j) = st.jobs.iter_mut().find(|j| j.id == ctx.job_id) {
                    j.run_id = Some(run.run_id);
                }
            }
            engine.remember_run(run.run_id, roots.clone(), Vec::new());
            let res = engine.with_exec(roots, run.run_id, ctx.job_id, None, ctx.cancel, ctx.progress, |ec| {
                mj_actions::undo::undo(ec, &entries)
            });
            let mut finished = run.clone();
            finished.finished = Some(Timestamp::now());
            let _ = engine.journal.put_run(&finished);
            engine.emit(Event::HistoryChanged(HistoryChanged { run_id: Some(run.run_id) }));
            match res {
                Ok(o) => JobResult { outcome: Some(o), error: None, report: true },
                Err(e) => JobResult { outcome: None, error: Some(e), report: true },
            }
        });
        self.enqueue(vec![(job.clone(), body)])?;
        Ok(job)
    }

    /// Undo every undoable entry of the newest run that has one.
    pub fn undo_last(self: &Arc<Self>) -> Result<Job> {
        let now = Timestamp::now();
        let mut runs = self.journal.runs();
        runs.sort_by_key(|r| std::cmp::Reverse(r.run_id));
        for r in runs.iter().filter(|r| r.kind != RunKind::Undo) {
            let ids = mj_actions::undo::undoable_entries(&self.journal, r.run_id, now);
            if !ids.is_empty() {
                return self.queue_undo(Some(r.run_id), format!("Undo · {}", r.title), ids);
            }
        }
        Err(Error::InvalidInput("nothing can be undone".into()))
    }

    /// Undo one run.
    pub fn undo_run(self: &Arc<Self>, req: RunRequest) -> Result<Job> {
        let run =
            self.journal.run(req.run_id).ok_or_else(|| Error::InvalidInput(format!("unknown run {}", req.run_id.0)))?;
        if run.kind == RunKind::Undo {
            return Err(Error::InvalidInput(
                "an undo run cannot be undone; restore its entries from the original run".into(),
            ));
        }
        let ids = mj_actions::undo::undoable_entries(&self.journal, run.run_id, Timestamp::now());
        self.queue_undo(Some(run.run_id), format!("Undo · {}", run.title), ids)
    }

    /// Restore selected entries.
    pub fn restore_entries(self: &Arc<Self>, req: RestoreEntriesRequest) -> Result<Job> {
        let now = Timestamp::now();
        let mut ids = Vec::new();
        for id in &req.entry_ids {
            let e = self.journal.entry(*id).ok_or_else(|| Error::InvalidInput(format!("unknown entry {}", id.0)))?;
            if !mj_actions::undo::is_undoable(&e, now) {
                return Err(Error::InvalidInput(format!("{} can no longer be restored", e.original.display())));
            }
            ids.push(*id);
        }
        let n = ids.len();
        self.queue_undo(None, format!("Restore {n} item{}", if n == 1 { "" } else { "s" }), ids)
    }

    /// Quarantine size.
    pub fn quarantine_info(&self) -> Result<QuarantineInfo> {
        let s = mj_actions::quarantine::summary(&self.journal, &self.cfg.quarantine_dir);
        Ok(QuarantineInfo {
            dir: self.cfg.quarantine_dir.clone(),
            items: s.items,
            bytes: s.bytes,
            oldest: s.oldest,
            purge_after_days: lock(&self.settings).general.quarantine_days,
        })
    }

    /// Permanently delete quarantined items older than the given age (queued job).
    pub fn quarantine_purge(self: &Arc<Self>, req: QuarantinePurgeRequest) -> Result<Job> {
        let days = req.older_than_days.unwrap_or(lock(&self.settings).general.quarantine_days);
        let info = self.quarantine_info()?;
        let job = self.new_job(JobKind::PurgeQuarantine, None, None, "Empty quarantine", info.items, info.bytes);
        let body: Work = Box::new(move |engine, _ctx| {
            match mj_actions::quarantine::purge(&engine.journal, &engine.cfg.quarantine_dir, Timestamp::now(), days) {
                Ok(freed) => {
                    engine.emit(Event::HistoryChanged(HistoryChanged { run_id: None }));
                    JobResult {
                        outcome: Some(mj_core::JobOutcome { bytes_saved: freed, ..Default::default() }),
                        error: None,
                        report: false,
                    }
                }
                Err(e) => JobResult { outcome: None, error: Some(e), report: false },
            }
        });
        self.enqueue(vec![(job.clone(), body)])?;
        Ok(job)
    }
}
