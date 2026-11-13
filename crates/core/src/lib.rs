//! # mj-core
//!
//! MyJanitor's domain types (the frozen Wave 1 contract), the error model and
//! shared **pure** utilities. No I/O policy lives here: no file is opened,
//! moved or deleted by this crate (except [`testutil`], behind a feature).
//!
//! Map of the contract (see `docs/ARCHITECTURE.md` for the data flow):
//!
//! | Stage | Types |
//! |---|---|
//! | scope + scan | [`ScanScope`], [`ScanOptions`], [`FileEntry`], [`FileIndex`], [`ScanStats`] |
//! | content | [`Fingerprints`], [`FingerprintStore`] |
//! | find | [`Detector`], [`Optimiser`], [`Rule`], [`Candidate`], [`CandidateGroup`], [`Diagnostics`] |
//! | rank | [`Opportunity`], [`SafetyClass`], [`Badges`], [`SavingsSummary`] |
//! | plan | [`Plan`], [`PlanStep`], [`Action`], [`OriginalsPolicy`], [`StepConfirmation`] |
//! | run | [`ActionItem`], [`Job`], [`JobState`], [`JournalEntry`], [`Report`] |
//! | config | [`Settings`], [`Schedule`], [`RulePreset`], [`ExternalTool`] |
//!
//! Utilities: [`hash`] (xxHash64, SHA-256), [`time`] (calendar maths),
//! [`glob`], [`pathutil`] (lexical paths + lossless path codec), [`size`],
//! [`pool`] (threads), [`cancel`], [`progress`].
#![forbid(unsafe_code)]

pub mod action;
pub mod badge;
pub mod cancel;
pub mod candidate;
pub mod detector;
pub mod entry;
pub mod error;
pub mod estimate;
pub mod fingerprint;
mod flags;
pub mod glob;
pub mod hash;
pub mod index;
pub mod job;
pub mod journal;
pub mod opportunity;
pub mod optimiser;
pub mod params;
pub mod pathutil;
pub mod plan;
pub mod pool;
pub mod progress;
pub mod report;
pub mod rule;
pub mod safety;
pub mod scope;
pub mod settings;
pub mod size;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;
pub mod time;
pub mod tools;

pub use action::{Action, ActionItem, ActionKind, ArchiveFormat, FsCompressAlgo, OriginalsPolicy, OutputMode};
pub use badge::Badges;
pub use cancel::CancelToken;
pub use candidate::{
    Candidate, CandidateGroup, CandidateId, DetectorOutput, Diagnostics, GroupId, GroupKind, KeepInfo, KeepRule,
    Source, TargetKind,
};
pub use detector::{DetectContext, Detector, DetectorId, DetectorInfo, PlatformSupport};
pub use entry::{Attributes, EntryKind, FileEntry, FileId, FileStamp};
pub use error::{Error, ErrorInfo, Result};
pub use estimate::{Confidence, Estimate};
pub use fingerprint::{FingerprintStore, Fingerprints, MemoryFingerprintStore, NoFingerprintStore};
pub use index::{FileIndex, ScanStats, SubtreeTotals};
pub use job::{ItemOutcome, ItemStatus, Job, JobId, JobKind, JobOutcome, JobProgress, JobState, RunId};
pub use journal::{EntryId, JournalEntry, JournalState, RunKind, RunRecord, TrashTicket};
pub use opportunity::{Availability, Category, Opportunity, OpportunityId, SavingsSummary, SubGroup};
pub use optimiser::{ArchiveJob, Archiver, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo};
pub use params::{ParamValue, ParamValues, SettingKind, SettingSpec, StepPreset};
pub use plan::{ChecklistItem, ChecklistLevel, Plan, PlanStep, SelectionOverrides, StepConfirmation, StepId};
pub use progress::{NullProgress, Phase, Progress, ProgressSink, ThrottledSink};
pub use report::{ExportFormat, Report, ReportSection, ReportTotals};
pub use rule::{Cmp, Condition, DateField, DateValue, Rule, RulePreset, TextMode};
pub use safety::{SafetyClass, SavingsBucket};
pub use scope::{ScanOptions, ScanScope};
pub use settings::{Cadence, Schedule, ScheduledStep, Settings};
pub use time::Timestamp;
pub use tools::ExternalTool;

/// Application name used for config/cache/data directories.
pub const APP_NAME: &str = "MyJanitor";
/// Lower-case directory name (`~/.cache/myjanitor`).
pub const APP_DIR: &str = "myjanitor";
/// Bundle identifier.
pub const BUNDLE_ID: &str = "com.myjanitor.app";

/// Lock a mutex, recovering from poisoning (a panicked worker must not wedge the app).
pub fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
