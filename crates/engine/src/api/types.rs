//! Request and response shapes of every command. **Frozen contract**, mirrored
//! one-to-one in `docs/IPC.md` and the mock backend in `ui/js/mock/`.
//!
//! Conventions: JSON field names are the Rust snake_case names; requests reject
//! unknown fields; optional request fields may be omitted; optional response
//! fields are omitted when `None`; paths use the lossless codec
//! (`mj_core::pathutil`); timestamps are integer nanoseconds since the epoch;
//! sizes are integer bytes; ids are integers below 2^53.

use std::path::PathBuf;

use mj_core::index::ScanStats;
use mj_core::{
    Action, Cadence, Candidate, CandidateGroup, CandidateId, Confidence, Diagnostics, ErrorInfo, Estimate,
    ExportFormat, ItemOutcome, Job, JobId, JobProgress, JournalEntry, KeepRule, Opportunity, OpportunityId,
    OriginalsPolicy, ParamValues, Phase, Plan, Rule, RulePreset, RunId, RunKind, SafetyClass, SavingsSummary,
    ScanScope, Schedule, Settings, StepConfirmation, StepId, Timestamp,
};
use mj_platform::VolumeInfo;
use serde::{Deserialize, Serialize};

pub use mj_filters::{QueryError, Suggestion};

// ---------------------------------------------------------------- app

/// `app_info` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    /// `"windows"` or `"linux"`.
    pub platform: String,
    pub bundle_id: String,
    /// Portable mode (data next to the executable, AUT-04).
    pub portable: bool,
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub data_dir: PathBuf,
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub cache_dir: PathBuf,
    /// Running elevated / as root.
    pub elevated: bool,
    /// False when the undo journal could not be persisted (History survives only this session).
    pub journal_persistent: bool,
}

// ---------------------------------------------------------------- scope + folder browser

/// `scope_set` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeSetRequest {
    pub scope: ScanScope,
}

/// `scope_add_root` / `scope_remove_root` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopePathRequest {
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub path: PathBuf,
}

/// `fs_list_dir` request (in-app folder browser, D-104).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsListDirRequest {
    /// Folder to list; `None` lists the places (home, common folders, drives).
    #[serde(default, with = "mj_core::pathutil::serde_path::opt")]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub show_hidden: bool,
    /// Also list files (for "save as" pickers); folders only by default.
    #[serde(default)]
    pub include_files: bool,
}

/// One row of the folder browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsDirEntry {
    pub name: String,
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_link: bool,
    pub hidden: bool,
}

/// A shortcut in the folder browser sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsPlace {
    pub label: String,
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub path: PathBuf,
    /// `home`, `folder` or `drive`.
    pub kind: String,
}

/// `fs_list_dir` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsListDirResponse {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "mj_core::pathutil::serde_path::opt")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "mj_core::pathutil::serde_path::opt")]
    pub parent: Option<PathBuf>,
    /// Sorted: folders first, then case-insensitive name.
    pub entries: Vec<FsDirEntry>,
    pub places: Vec<FsPlace>,
}

// ---------------------------------------------------------------- scan

/// `scan_start` request.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanStartRequest {
    /// Replaces the session scope first when given.
    #[serde(default)]
    pub scope: Option<ScanScope>,
}

/// Lifecycle of the session scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanRunState {
    /// No scan yet this session.
    Idle,
    Scanning,
    Paused,
    Stopping,
    Done,
    Cancelled,
    Failed,
}

/// `scan://progress` payload (throttled to one per 50 ms).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScanProgressEvent {
    pub generation: u64,
    /// `walking`, `cache`, `hashing`, `detecting`.
    pub phase: Option<Phase>,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Items done / total in the current phase (total unknown while walking).
    pub done: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "mj_core::pathutil::serde_path::opt")]
    pub current: Option<PathBuf>,
    pub elapsed_ms: u64,
}

/// `scan://detector` payload: live per-card totals while the scan runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectorTotalsEvent {
    pub generation: u64,
    pub opportunity_id: OpportunityId,
    pub title: String,
    pub saving: u64,
    pub count: u64,
    /// This detector has finished (its card is final).
    pub finished: bool,
}

/// `scan_status` response; also the `scan://done` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanStatus {
    pub state: ScanRunState,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<Timestamp>,
    pub progress: ScanProgressEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<ScanStats>,
    /// Results came (partly) from the scan cache.
    pub cached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

/// `scan_start` response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanStarted {
    pub generation: u64,
}

// ---------------------------------------------------------------- opportunities

/// Overview tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityTab {
    #[default]
    All,
    Clean,
    Optimise,
    Archive,
    ReviewOnly,
}

/// Overview sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunitySort {
    /// "Biggest saving first" (default).
    #[default]
    Saving,
    Count,
    /// Safest first (reversible, rebuildable, light copy, lossy, permanent, review only).
    Safety,
    Name,
}

/// `opportunities_list` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpportunitiesRequest {
    #[serde(default)]
    pub tab: OpportunityTab,
    #[serde(default)]
    pub sort: OpportunitySort,
}

/// Counts per tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TabCounts {
    pub all: u64,
    pub clean: u64,
    pub optimise: u64,
    pub archive: u64,
    pub review_only: u64,
}

/// `opportunities_list` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpportunitiesList {
    pub generation: u64,
    /// Filtered by tab, sorted; disabled opportunities are included (with a reason).
    pub items: Vec<Opportunity>,
    /// Over all tabs (hero card + status strip "Reclaimable").
    pub summary: SavingsSummary,
    pub counts: TabCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanned_at: Option<Timestamp>,
    pub cached: bool,
}

// ---------------------------------------------------------------- step review

/// Request naming one opportunity (`step_detail`, `selection_reset`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpportunityRequest {
    pub opportunity_id: OpportunityId,
}

/// Selection totals of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StepSummary {
    pub candidates: u64,
    pub selected: u64,
    /// Current size of the selection.
    pub size_before: u64,
    /// Estimated size after the action.
    pub size_after: u64,
    pub saving: u64,
    pub confidence: Confidence,
}

/// `step_detail` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepDetail {
    pub opportunity: Opportunity,
    pub summary: StepSummary,
    /// What the analysis could not cover (shown as a note under the table).
    pub diagnostics: Diagnostics,
    /// Plan step for this opportunity, when it is in the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<StepId>,
    /// The step is lossy with the current parameters.
    pub lossy: bool,
    pub safety: SafetyClass,
}

/// Sortable candidate columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSortKey {
    #[default]
    Saving,
    Size,
    Name,
    Path,
    Modified,
    /// By group, keeper first.
    Group,
}

/// `candidates_page` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidatesPageRequest {
    pub opportunity_id: OpportunityId,
    /// Sub-group tab key; `None` = all.
    #[serde(default)]
    pub sub_group: Option<String>,
    #[serde(default)]
    pub offset: u64,
    /// 1-500 (default 100).
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub sort: CandidateSortKey,
    #[serde(default)]
    pub descending: bool,
    /// Case-insensitive substring of the path.
    #[serde(default)]
    pub filter: Option<String>,
}

fn default_limit() -> u32 {
    100
}

/// One table row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateRow {
    #[serde(flatten)]
    pub candidate: Candidate,
    /// Effective selection (default flag + user overrides).
    pub selected: bool,
    /// False for protected and other blocked rows (checkbox disabled).
    pub actionable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

/// `candidates_page` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidatesPage {
    pub opportunity_id: OpportunityId,
    /// Rows matching sub-group + filter.
    pub total: u64,
    pub offset: u64,
    pub rows: Vec<CandidateRow>,
    /// Groups referenced by the rows of this page.
    pub groups: Vec<CandidateGroup>,
}

/// `selection_set` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionSetRequest {
    pub opportunity_id: OpportunityId,
    /// Candidates to change; `None` = every candidate of `sub_group` (or of the step).
    #[serde(default)]
    pub ids: Option<Vec<CandidateId>>,
    #[serde(default)]
    pub sub_group: Option<String>,
    pub selected: bool,
}

/// `selection_apply_keep_rule` request (CLN-ACT-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionKeepRuleRequest {
    pub opportunity_id: OpportunityId,
    pub rule: KeepRule,
}

/// `step_params_set` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepParamsRequest {
    pub opportunity_id: OpportunityId,
    /// Values to merge over the current parameters.
    #[serde(default)]
    pub params: ParamValues,
    /// Apply this preset first (then `params`).
    #[serde(default)]
    pub preset_id: Option<String>,
}

/// `step_params_set` response (re-estimated).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepParamsResponse {
    pub params: ParamValues,
    pub summary: StepSummary,
    pub lossy: bool,
    pub safety: SafetyClass,
}

/// `candidate_preview` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidatePreviewRequest {
    pub candidate_id: CandidateId,
}

/// Audio overview for the waveform preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waveform {
    /// Peak amplitudes 0-255, evenly spaced over the duration.
    pub peaks: Vec<u8>,
    pub duration_ms: u64,
    /// Silences that the current settings would remove, `[start_ms, end_ms]`.
    pub silences: Vec<[u64; 2]>,
}

/// `candidate_preview` response (SAF-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    /// `none`, `image`, `text` or `waveform`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// Base64 image bytes (thumbnail, at most 512 px on the long edge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// First 4 KiB of text, for text files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveform: Option<Waveform>,
    /// Why no preview is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ---------------------------------------------------------------- custom search + presets

/// `search_parse` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchParseRequest {
    pub query: String,
}

/// `search_parse` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchParseResponse {
    /// The rule, when the query is valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
    pub errors: Vec<QueryError>,
}

/// `search_suggest` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchSuggestRequest {
    pub query: String,
    /// Byte offset of the caret in `query`.
    pub cursor: usize,
}

/// `search_run` request: creates (or replaces) a custom opportunity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRunRequest {
    pub rule: Rule,
    /// Card title (default "Custom search").
    #[serde(default)]
    pub title: Option<String>,
    /// CLN-ENG-04.
    #[serde(default)]
    pub topmost_only: bool,
    /// Default action of the resulting step (default `trash`).
    #[serde(default)]
    pub action: Option<Action>,
}

/// `preset_save` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetSaveRequest {
    pub preset: RulePreset,
}

/// Request naming a string id (`preset_delete`, `schedule_delete`, `schedule_run_now`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdRequest {
    pub id: String,
}

// ---------------------------------------------------------------- plan

/// `plan_add_step` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanAddStepRequest {
    pub opportunity_id: OpportunityId,
    /// Default: the opportunity's default action.
    #[serde(default)]
    pub action: Option<Action>,
    #[serde(default)]
    pub params: Option<ParamValues>,
}

/// `plan_update_step` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanUpdateStepRequest {
    pub step_id: StepId,
    #[serde(default)]
    pub action: Option<Action>,
    #[serde(default)]
    pub params: Option<ParamValues>,
}

/// Request naming one plan step (`plan_remove_step`, `plan_confirmation_info`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepRequest {
    pub step_id: StepId,
}

/// `plan_reorder` request: every step id exactly once, in the new order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanReorderRequest {
    pub step_ids: Vec<StepId>,
}

/// One step of a dry-run report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunStep {
    pub step_id: StepId,
    pub title: String,
    /// `Confidence::Exact` for removals; measured for optimisations.
    pub estimate: Estimate,
    /// First 100 skipped items with reasons.
    pub skipped: Vec<ItemOutcome>,
    pub skipped_count: u64,
    pub warnings: Vec<String>,
}

/// `plan://dry_run` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunReport {
    pub plan_id: u64,
    pub job_id: JobId,
    pub steps: Vec<DryRunStep>,
    pub total: Estimate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

/// One choice of the Originals dropdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginalsOption {
    pub policy: OriginalsPolicy,
    pub label: String,
    pub reversible: bool,
}

/// Synced-folder warning of the confirmation dialog (SAF-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedWarning {
    /// Selected items inside synced folders.
    pub count: u64,
    /// "OneDrive – Acme".
    pub provider: String,
    /// "Make them online-only instead" is possible (CLN-ACT-07).
    pub can_online_only: bool,
}

/// `plan_confirmation_info` response: everything the lossy/permanent dialog shows (SAF-11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmationInfo {
    pub step_id: StepId,
    /// "Compress 23 recordings?"
    pub title: String,
    /// "This is lossy. The new files sound fine for speech but cannot be turned back into the originals."
    pub body: String,
    pub files: u64,
    pub bytes_now: u64,
    pub bytes_after: u64,
    pub saving: u64,
    pub confidence: Confidence,
    /// Empty for actions without originals (permanent delete).
    pub originals_options: Vec<OriginalsOption>,
    pub default_originals: OriginalsPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced: Option<SyncedWarning>,
    /// "I understand the new files are lower quality than the originals"
    pub acknowledge_text: String,
    /// "Every change is logged; History → Restore brings the originals back for 30 days."
    pub undo_text: String,
    /// Acting button label ("Compress 23 files"), shown in the warning colour.
    pub action_label: String,
    /// True for permanent (irreversible) steps: the button uses the danger colour.
    pub permanent: bool,
}

/// `plan_apply` request: one confirmation per step whose safety requires it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanApplyRequest {
    #[serde(default)]
    pub confirmations: Vec<StepConfirmation>,
}

/// `plan_apply` / `schedule_run_now` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyStarted {
    pub run_id: RunId,
    /// One job per step, queued in plan order.
    pub jobs: Vec<Job>,
}

// ---------------------------------------------------------------- jobs

/// Request naming one job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    pub job_id: JobId,
}

/// `jobs_reorder` request: queued job ids in the new order (running jobs keep their place).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobsReorderRequest {
    pub job_ids: Vec<JobId>,
}

/// `job://progress` payload (throttled to one per 50 ms per job).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobProgressEvent {
    pub job_id: JobId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    pub progress: JobProgress,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "mj_core::pathutil::serde_path::opt")]
    pub current: Option<PathBuf>,
}

/// `job://done` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobDoneEvent {
    pub job: Job,
    /// A report for `job.run_id` can be fetched with `report_get`.
    pub report_available: bool,
}

// ---------------------------------------------------------------- history

/// Paging request (`history_list`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    #[serde(default)]
    pub offset: u64,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

impl Default for PageRequest {
    fn default() -> Self {
        PageRequest { offset: 0, limit: default_limit() }
    }
}

/// One History row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRun {
    pub run_id: RunId,
    pub kind: RunKind,
    pub title: String,
    pub started: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<Timestamp>,
    pub steps: Vec<String>,
    pub items: u64,
    pub done: u64,
    pub failed: u64,
    pub undone: u64,
    pub bytes_saved: u64,
    /// Some entry can still be undone.
    pub undoable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_until: Option<Timestamp>,
}

/// `history_list` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryList {
    pub total: u64,
    /// Newest first.
    pub runs: Vec<HistoryRun>,
}

/// `history_run_detail` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDetailRequest {
    pub run_id: RunId,
    #[serde(default)]
    pub offset: u64,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

/// `history_run_detail` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRunDetail {
    pub run: HistoryRun,
    pub total: u64,
    pub entries: Vec<JournalEntry>,
}

/// Request naming one run (`undo_run`, `report_get`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub run_id: RunId,
}

/// `restore_entries` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreEntriesRequest {
    pub entry_ids: Vec<mj_core::EntryId>,
}

/// `quarantine_info` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantineInfo {
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub dir: PathBuf,
    pub items: u64,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest: Option<Timestamp>,
    pub purge_after_days: u32,
}

/// `quarantine_purge` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuarantinePurgeRequest {
    /// Purge items older than this many days (default: the setting). 0 = everything.
    #[serde(default)]
    pub older_than_days: Option<u32>,
}

// ---------------------------------------------------------------- reports + exports

/// What `report_export` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportExportKind {
    /// The operation report of one run (SAF-04).
    Report,
    /// The audit log (SAF-12): one run, or the whole journal when `run_id` is omitted. CSV only.
    Audit,
}

/// `report_export` request. The destination comes from the in-app save picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportExportRequest {
    #[serde(default)]
    pub run_id: Option<RunId>,
    pub kind: ReportExportKind,
    pub format: ExportFormat,
    /// Target file; never overwritten (a ` (1)` suffix is added instead).
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub dest: PathBuf,
}

/// `results_export` request (CLN-ACT-11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultsExportRequest {
    /// One opportunity, or every opportunity when omitted.
    #[serde(default)]
    pub opportunity_id: Option<OpportunityId>,
    #[serde(default)]
    pub selected_only: bool,
    pub format: ExportFormat,
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub dest: PathBuf,
}

/// Export result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportResult {
    /// The file actually written (may carry a ` (n)` suffix).
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub path: PathBuf,
    pub bytes: u64,
    pub rows: u64,
}

// ---------------------------------------------------------------- settings

/// `settings_set` request: the whole settings object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsSetRequest {
    pub settings: Settings,
}

/// `protected_paths_get` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedPathsInfo {
    /// Built-in folders (locked).
    pub builtin: Vec<String>,
    /// Built-in patterns (locked).
    pub builtin_patterns: Vec<String>,
    /// The user's list (editable).
    #[serde(with = "mj_core::pathutil::serde_path::vec")]
    pub user: Vec<PathBuf>,
}

/// `protected_paths_set` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedPathsSetRequest {
    #[serde(with = "mj_core::pathutil::serde_path::vec")]
    pub user: Vec<PathBuf>,
}

/// `cache_info` / `cache_clear` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheInfo {
    pub entries: u64,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "mj_core::pathutil::serde_path::opt")]
    pub path: Option<PathBuf>,
}

// ---------------------------------------------------------------- schedules

/// `schedule_save` request (insert or replace by id; empty id = new).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSaveRequest {
    pub schedule: Schedule,
}

/// `schedule_from_plan` request ("Save as weekly schedule").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleFromPlanRequest {
    pub name: String,
    pub cadence: Cadence,
}

// ---------------------------------------------------------------- disk

/// `drives_list` row: a volume plus the plan's effect on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drive {
    #[serde(flatten)]
    pub volume: VolumeInfo,
    /// Bytes the current plan frees on this volume.
    pub plan_saving: u64,
    /// `free + plan_saving`.
    pub free_after_plan: u64,
}

/// `disk_usage_tree` request (treemap, SCAN-05/06).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskUsageRequest {
    /// Subtree root; `None` = one node per scan root under a virtual root.
    #[serde(default, with = "mj_core::pathutil::serde_path::opt")]
    pub path: Option<PathBuf>,
    /// Levels below `path` (1-4, default 2).
    #[serde(default = "default_depth")]
    pub depth: u8,
    /// Children smaller than this fraction of their parent are folded into `other` (default 0.01).
    #[serde(default)]
    pub min_fraction: Option<f32>,
}

fn default_depth() -> u8 {
    2
}

/// Small children folded together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskOther {
    pub size: u64,
    pub count: u64,
}

/// One treemap node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskNode {
    pub name: String,
    #[serde(with = "mj_core::pathutil::serde_path")]
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub allocated: u64,
    pub files: u64,
    /// Sorted by size, largest first.
    pub children: Vec<DiskNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<DiskOther>,
}

/// Breakdown dimension (SCAN-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakdownBy {
    /// File-type group.
    Type,
    Extension,
    /// Age buckets by modification time.
    Age,
    Owner,
}

/// `breakdown` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BreakdownRequest {
    pub by: BreakdownBy,
    #[serde(default, with = "mj_core::pathutil::serde_path::opt")]
    pub path: Option<PathBuf>,
}

/// One breakdown row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakdownRow {
    pub key: String,
    pub label: String,
    pub bytes: u64,
    pub files: u64,
}

/// `breakdown` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Breakdown {
    pub by: BreakdownBy,
    /// Largest first.
    pub rows: Vec<BreakdownRow>,
}

// ---------------------------------------------------------------- generic

/// `opportunities://changed` payload: refetch the overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpportunitiesChanged {
    pub generation: u64,
}

/// `history://changed` payload: refetch History.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HistoryChanged {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
}

/// Plan, re-exported for the IPC table.
pub type PlanResponse = Plan;
