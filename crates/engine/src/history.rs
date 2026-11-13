//! History, undo and quarantine (SAF-03, CLN-ACT-04). Owner: W3-D.

use std::sync::Arc;

use mj_core::{Error, Job, JournalState, Result, Timestamp};

use crate::api::{
    HistoryList, HistoryRun, HistoryRunDetail, PageRequest, QuarantineInfo, QuarantinePurgeRequest,
    RestoreEntriesRequest, RunDetailRequest, RunRequest,
};
use crate::Engine;

impl Engine {
    fn history_run(&self, run: &mj_core::RunRecord, now: Timestamp) -> HistoryRun {
        let entries = self.journal.entries_of(run.run_id);
        let count = |s: JournalState| entries.iter().filter(|e| e.state == s).count() as u64;
        let undoable = entries
            .iter()
            .any(|e| e.state == JournalState::Done && e.reversible && e.undo_deadline.is_none_or(|d| d > now));
        HistoryRun {
            run_id: run.run_id,
            kind: run.kind,
            title: run.title.clone(),
            started: run.started,
            finished: run.finished,
            steps: run.steps.clone(),
            items: entries.len() as u64,
            done: count(JournalState::Done),
            failed: count(JournalState::Failed),
            undone: count(JournalState::Undone),
            bytes_saved: entries
                .iter()
                .filter(|e| e.state == JournalState::Done)
                .map(|e| e.bytes_before.saturating_sub(e.bytes_after))
                .sum(),
            undoable,
            undo_until: entries.iter().filter_map(|e| e.undo_deadline).max(),
        }
    }

    /// Runs, newest first.
    pub fn history_list(&self, req: PageRequest) -> Result<HistoryList> {
        let now = Timestamp::now();
        let runs = self.journal.runs();
        let total = runs.len() as u64;
        let runs = runs
            .iter()
            .skip(req.offset as usize)
            .take(req.limit.clamp(1, 500) as usize)
            .map(|r| self.history_run(r, now))
            .collect();
        Ok(HistoryList { total, runs })
    }

    /// Entries of one run.
    pub fn history_run_detail(&self, req: RunDetailRequest) -> Result<HistoryRunDetail> {
        let run = self
            .journal
            .runs()
            .into_iter()
            .find(|r| r.run_id == req.run_id)
            .ok_or_else(|| Error::InvalidInput(format!("unknown run {}", req.run_id.0)))?;
        let entries = self.journal.entries_of(run.run_id);
        let total = entries.len() as u64;
        Ok(HistoryRunDetail {
            run: self.history_run(&run, Timestamp::now()),
            total,
            entries: entries.into_iter().skip(req.offset as usize).take(req.limit.clamp(1, 500) as usize).collect(),
        })
    }

    /// Undo the newest undoable run.
    pub fn undo_last(self: &Arc<Self>) -> Result<Job> {
        Err(Error::not_implemented("undo (W3-D)"))
    }

    /// Undo one run.
    pub fn undo_run(self: &Arc<Self>, req: RunRequest) -> Result<Job> {
        let _ = req;
        Err(Error::not_implemented("undo (W3-D)"))
    }

    /// Restore selected entries.
    pub fn restore_entries(self: &Arc<Self>, req: RestoreEntriesRequest) -> Result<Job> {
        let _ = req;
        Err(Error::not_implemented("restore (W3-D)"))
    }

    /// Quarantine size.
    pub fn quarantine_info(&self) -> Result<QuarantineInfo> {
        Err(Error::not_implemented("quarantine info (W3-D)"))
    }

    /// Purge expired quarantine items.
    pub fn quarantine_purge(self: &Arc<Self>, req: QuarantinePurgeRequest) -> Result<Job> {
        let _ = req;
        Err(Error::not_implemented("quarantine purge (W3-D)"))
    }
}
