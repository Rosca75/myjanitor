//! Events pushed from the engine to the front end. **Frozen contract**
//! (`docs/IPC.md` §4). The Tauri shell forwards each with
//! `app.emit(event.name(), event.payload())`; the CLI prints or ignores them.

use mj_core::{Job, Plan};
use serde_json::Value;

use super::types::{
    DetectorTotalsEvent, DryRunReport, HistoryChanged, JobDoneEvent, JobProgressEvent, OpportunitiesChanged,
    ScanProgressEvent, ScanStatus,
};

/// Every event.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    ScanProgress(ScanProgressEvent),
    ScanDetector(DetectorTotalsEvent),
    ScanDone(ScanStatus),
    OpportunitiesChanged(OpportunitiesChanged),
    PlanChanged(Plan),
    DryRun(DryRunReport),
    JobProgress(JobProgressEvent),
    JobState(Job),
    JobDone(JobDoneEvent),
    HistoryChanged(HistoryChanged),
}

impl Event {
    /// Event name on the wire.
    pub fn name(&self) -> &'static str {
        match self {
            Event::ScanProgress(_) => "scan://progress",
            Event::ScanDetector(_) => "scan://detector",
            Event::ScanDone(_) => "scan://done",
            Event::OpportunitiesChanged(_) => "opportunities://changed",
            Event::PlanChanged(_) => "plan://changed",
            Event::DryRun(_) => "plan://dry_run",
            Event::JobProgress(_) => "job://progress",
            Event::JobState(_) => "job://state",
            Event::JobDone(_) => "job://done",
            Event::HistoryChanged(_) => "history://changed",
        }
    }

    /// Payload as JSON.
    pub fn payload(&self) -> Value {
        let r = match self {
            Event::ScanProgress(p) => serde_json::to_value(p),
            Event::ScanDetector(p) => serde_json::to_value(p),
            Event::ScanDone(p) => serde_json::to_value(p),
            Event::OpportunitiesChanged(p) => serde_json::to_value(p),
            Event::PlanChanged(p) => serde_json::to_value(p),
            Event::DryRun(p) => serde_json::to_value(p),
            Event::JobProgress(p) => serde_json::to_value(p),
            Event::JobState(p) => serde_json::to_value(p),
            Event::JobDone(p) => serde_json::to_value(p),
            Event::HistoryChanged(p) => serde_json::to_value(p),
        };
        r.unwrap_or(Value::Null)
    }

    /// True for high-frequency progress events (front ends may drop them when busy).
    pub fn is_progress(&self) -> bool {
        matches!(self, Event::ScanProgress(_) | Event::JobProgress(_))
    }
}

/// Receiver of engine events. Must not block.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &Event);
}

/// Drops every event.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _: &Event) {}
}

/// Records events (tests, CLI).
#[derive(Debug, Default)]
pub struct CollectSink(pub std::sync::Mutex<Vec<Event>>);

impl EventSink for CollectSink {
    fn emit(&self, event: &Event) {
        mj_core::lock(&self.0).push(event.clone());
    }
}

impl CollectSink {
    /// Names of the events received so far.
    pub fn names(&self) -> Vec<&'static str> {
        mj_core::lock(&self.0).iter().map(Event::name).collect()
    }
}
