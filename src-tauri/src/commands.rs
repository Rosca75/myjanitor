//! Thin `#[tauri::command]` wrappers: one per entry of `mj_engine::api::COMMANDS`,
//! same name, same request (`req` argument) and response. **Owner: W3-D.**
//!
//! Every wrapper is `async` and runs the engine method on Tauri's blocking pool,
//! so no command ever runs on the main (UI) thread. Errors are `ErrorInfo`
//! (`{code, message, hint, path?}`); the JS promise rejects with that object.

use std::sync::Arc;

use mj_core::{ErrorInfo, ExternalTool, Job, Opportunity, Plan, Report, RulePreset, ScanScope, Schedule, Settings};
use mj_engine::api::*;
use mj_engine::Engine;
use tauri::State;

/// Managed state: the one engine.
pub struct AppState {
    pub engine: Arc<Engine>,
}

async fn run<T, F>(state: State<'_, AppState>, f: F) -> Result<T, ErrorInfo>
where
    T: Send + 'static,
    F: FnOnce(Arc<Engine>) -> mj_core::Result<T> + Send + 'static,
{
    let engine = Arc::clone(&state.engine);
    match tauri::async_runtime::spawn_blocking(move || f(engine)).await {
        Ok(r) => r.map_err(|e| e.info()),
        Err(e) => Err(mj_core::Error::Internal(format!("command worker failed: {e}")).info()),
    }
}

/// Version, platform, data/cache folders, portable and elevation state.
#[tauri::command]
pub async fn app_info(state: State<'_, AppState>) -> Result<AppInfo, ErrorInfo> {
    run(state, move |e| e.app_info()).await
}

/// The session scan scope (scope chips).
#[tauri::command]
pub async fn scope_get(state: State<'_, AppState>) -> Result<ScanScope, ErrorInfo> {
    run(state, move |e| e.scope_get()).await
}

/// Replace the scope (roots, includes, excludes, options).
#[tauri::command]
pub async fn scope_set(state: State<'_, AppState>, req: ScopeSetRequest) -> Result<ScanScope, ErrorInfo> {
    run(state, move |e| e.scope_set(req)).await
}

/// "+ Add location": add one root (normalised; nested roots collapse).
#[tauri::command]
pub async fn scope_add_root(state: State<'_, AppState>, req: ScopePathRequest) -> Result<ScanScope, ErrorInfo> {
    run(state, move |e| e.scope_add_root(req)).await
}

/// Remove a scope chip.
#[tauri::command]
pub async fn scope_remove_root(state: State<'_, AppState>, req: ScopePathRequest) -> Result<ScanScope, ErrorInfo> {
    run(state, move |e| e.scope_remove_root(req)).await
}

/// In-app folder browser (D-104): list a folder, or the places when `path` is omitted.
#[tauri::command]
pub async fn fs_list_dir(state: State<'_, AppState>, req: FsListDirRequest) -> Result<FsListDirResponse, ErrorInfo> {
    run(state, move |e| e.fs_list_dir(req)).await
}

/// Start a scan of the scope; progress via `scan://progress`, `scan://detector`, end via `scan://done`.
#[tauri::command]
pub async fn scan_start(state: State<'_, AppState>, req: ScanStartRequest) -> Result<ScanStarted, ErrorInfo> {
    run(state, move |e| e.scan_start(req)).await
}

/// Pause the running scan.
#[tauri::command]
pub async fn scan_pause(state: State<'_, AppState>) -> Result<ScanStatus, ErrorInfo> {
    run(state, move |e| e.scan_pause()).await
}

/// Resume a paused scan.
#[tauri::command]
pub async fn scan_resume(state: State<'_, AppState>) -> Result<ScanStatus, ErrorInfo> {
    run(state, move |e| e.scan_resume()).await
}

/// Stop the scan; results so far are discarded, the previous results stay.
#[tauri::command]
pub async fn scan_stop(state: State<'_, AppState>) -> Result<ScanStatus, ErrorInfo> {
    run(state, move |e| e.scan_stop()).await
}

/// Current scan state (used at start-up and after reloads).
#[tauri::command]
pub async fn scan_status(state: State<'_, AppState>) -> Result<ScanStatus, ErrorInfo> {
    run(state, move |e| e.scan_status()).await
}

/// Ranked opportunity cards for a tab, the hero summary and tab counts.
#[tauri::command]
pub async fn opportunities_list(
    state: State<'_, AppState>,
    req: OpportunitiesRequest,
) -> Result<OpportunitiesList, ErrorInfo> {
    run(state, move |e| e.opportunities_list(req)).await
}

/// Step review header and right panel: opportunity, settings, presets, summary, diagnostics.
#[tauri::command]
pub async fn step_detail(state: State<'_, AppState>, req: OpportunityRequest) -> Result<StepDetail, ErrorInfo> {
    run(state, move |e| e.step_detail(req)).await
}

/// One page of the step's candidates table (sub-group, sort, filter).
#[tauri::command]
pub async fn candidates_page(
    state: State<'_, AppState>,
    req: CandidatesPageRequest,
) -> Result<CandidatesPage, ErrorInfo> {
    run(state, move |e| e.candidates_page(req)).await
}

/// Check / uncheck rows (or a whole sub-group).
#[tauri::command]
pub async fn selection_set(state: State<'_, AppState>, req: SelectionSetRequest) -> Result<StepSummary, ErrorInfo> {
    run(state, move |e| e.selection_set(req)).await
}

/// Smart auto-select in groups (CLN-ACT-01).
#[tauri::command]
pub async fn selection_apply_keep_rule(
    state: State<'_, AppState>,
    req: SelectionKeepRuleRequest,
) -> Result<StepSummary, ErrorInfo> {
    run(state, move |e| e.selection_apply_keep_rule(req)).await
}

/// Back to the default selection.
#[tauri::command]
pub async fn selection_reset(state: State<'_, AppState>, req: OpportunityRequest) -> Result<StepSummary, ErrorInfo> {
    run(state, move |e| e.selection_reset(req)).await
}

/// Change presets / toggles of a step and re-estimate.
#[tauri::command]
pub async fn step_params_set(
    state: State<'_, AppState>,
    req: StepParamsRequest,
) -> Result<StepParamsResponse, ErrorInfo> {
    run(state, move |e| e.step_params_set(req)).await
}

/// Preview for one row (thumbnail, text head, waveform).
#[tauri::command]
pub async fn candidate_preview(state: State<'_, AppState>, req: CandidatePreviewRequest) -> Result<Preview, ErrorInfo> {
    run(state, move |e| e.candidate_preview(req)).await
}

/// Parse a quick query into a rule tree, with error positions.
#[tauri::command]
pub async fn search_parse(
    state: State<'_, AppState>,
    req: SearchParseRequest,
) -> Result<SearchParseResponse, ErrorInfo> {
    run(state, move |e| e.search_parse(req)).await
}

/// Autocomplete for the search box.
#[tauri::command]
pub async fn search_suggest(
    state: State<'_, AppState>,
    req: SearchSuggestRequest,
) -> Result<Vec<Suggestion>, ErrorInfo> {
    run(state, move |e| e.search_suggest(req)).await
}

/// Run a rule over the index; creates or replaces a `custom:<n>` opportunity.
#[tauri::command]
pub async fn search_run(state: State<'_, AppState>, req: SearchRunRequest) -> Result<Opportunity, ErrorInfo> {
    run(state, move |e| e.search_run(req)).await
}

/// Built-in and user rule presets.
#[tauri::command]
pub async fn presets_list(state: State<'_, AppState>) -> Result<Vec<RulePreset>, ErrorInfo> {
    run(state, move |e| e.presets_list()).await
}

/// Save a user preset (insert or replace by id).
#[tauri::command]
pub async fn preset_save(state: State<'_, AppState>, req: PresetSaveRequest) -> Result<Vec<RulePreset>, ErrorInfo> {
    run(state, move |e| e.preset_save(req)).await
}

/// Delete a user preset (built-ins cannot be deleted).
#[tauri::command]
pub async fn preset_delete(state: State<'_, AppState>, req: IdRequest) -> Result<Vec<RulePreset>, ErrorInfo> {
    run(state, move |e| e.preset_delete(req)).await
}

/// The plan with steps, estimate and checklist.
#[tauri::command]
pub async fn plan_get(state: State<'_, AppState>) -> Result<Plan, ErrorInfo> {
    run(state, move |e| e.plan_get()).await
}

/// "+ Add" on a card.
#[tauri::command]
pub async fn plan_add_step(state: State<'_, AppState>, req: PlanAddStepRequest) -> Result<Plan, ErrorInfo> {
    run(state, move |e| e.plan_add_step(req)).await
}

/// Change a step's action or parameters.
#[tauri::command]
pub async fn plan_update_step(state: State<'_, AppState>, req: PlanUpdateStepRequest) -> Result<Plan, ErrorInfo> {
    run(state, move |e| e.plan_update_step(req)).await
}

/// "×" on a plan step.
#[tauri::command]
pub async fn plan_remove_step(state: State<'_, AppState>, req: StepRequest) -> Result<Plan, ErrorInfo> {
    run(state, move |e| e.plan_remove_step(req)).await
}

/// Drag-and-drop order of steps.
#[tauri::command]
pub async fn plan_reorder(state: State<'_, AppState>, req: PlanReorderRequest) -> Result<Plan, ErrorInfo> {
    run(state, move |e| e.plan_reorder(req)).await
}

/// Remove every step.
#[tauri::command]
pub async fn plan_clear(state: State<'_, AppState>) -> Result<Plan, ErrorInfo> {
    run(state, move |e| e.plan_clear()).await
}

/// "Estimate exactly (dry run)": queues a dry-run job; result via `plan://dry_run`.
#[tauri::command]
pub async fn plan_dry_run(state: State<'_, AppState>) -> Result<Job, ErrorInfo> {
    run(state, move |e| e.plan_dry_run()).await
}

/// Content of the lossy / permanent confirmation dialog for one step.
#[tauri::command]
pub async fn plan_confirmation_info(
    state: State<'_, AppState>,
    req: StepRequest,
) -> Result<ConfirmationInfo, ErrorInfo> {
    run(state, move |e| e.plan_confirmation_info(req)).await
}

/// "Apply plan": queues one job per step; fails with `confirmation_required` if a step lacks its confirmation.
#[tauri::command]
pub async fn plan_apply(state: State<'_, AppState>, req: PlanApplyRequest) -> Result<ApplyStarted, ErrorInfo> {
    run(state, move |e| e.plan_apply(req)).await
}

/// Queue contents (running, queued, recently finished).
#[tauri::command]
pub async fn jobs_list(state: State<'_, AppState>) -> Result<Vec<Job>, ErrorInfo> {
    run(state, move |e| e.jobs_list()).await
}

/// Pause a job (between items).
#[tauri::command]
pub async fn job_pause(state: State<'_, AppState>, req: JobRequest) -> Result<Job, ErrorInfo> {
    run(state, move |e| e.job_pause(req)).await
}

/// Resume a paused job.
#[tauri::command]
pub async fn job_resume(state: State<'_, AppState>, req: JobRequest) -> Result<Job, ErrorInfo> {
    run(state, move |e| e.job_resume(req)).await
}

/// Cancel a job (finished items stay applied and undoable).
#[tauri::command]
pub async fn job_cancel(state: State<'_, AppState>, req: JobRequest) -> Result<Job, ErrorInfo> {
    run(state, move |e| e.job_cancel(req)).await
}

/// Reorder queued jobs.
#[tauri::command]
pub async fn jobs_reorder(state: State<'_, AppState>, req: JobsReorderRequest) -> Result<Vec<Job>, ErrorInfo> {
    run(state, move |e| e.jobs_reorder(req)).await
}

/// History: runs newest first.
#[tauri::command]
pub async fn history_list(state: State<'_, AppState>, req: PageRequest) -> Result<HistoryList, ErrorInfo> {
    run(state, move |e| e.history_list(req)).await
}

/// Journal entries of one run.
#[tauri::command]
pub async fn history_run_detail(
    state: State<'_, AppState>,
    req: RunDetailRequest,
) -> Result<HistoryRunDetail, ErrorInfo> {
    run(state, move |e| e.history_run_detail(req)).await
}

/// "Undo last action": undo the newest undoable run.
#[tauri::command]
pub async fn undo_last(state: State<'_, AppState>) -> Result<Job, ErrorInfo> {
    run(state, move |e| e.undo_last()).await
}

/// Restore every undoable entry of a run.
#[tauri::command]
pub async fn undo_run(state: State<'_, AppState>, req: RunRequest) -> Result<Job, ErrorInfo> {
    run(state, move |e| e.undo_run(req)).await
}

/// Restore selected entries.
#[tauri::command]
pub async fn restore_entries(state: State<'_, AppState>, req: RestoreEntriesRequest) -> Result<Job, ErrorInfo> {
    run(state, move |e| e.restore_entries(req)).await
}

/// Quarantine folder size and age.
#[tauri::command]
pub async fn quarantine_info(state: State<'_, AppState>) -> Result<QuarantineInfo, ErrorInfo> {
    run(state, move |e| e.quarantine_info()).await
}

/// Permanently delete expired quarantine items.
#[tauri::command]
pub async fn quarantine_purge(state: State<'_, AppState>, req: QuarantinePurgeRequest) -> Result<Job, ErrorInfo> {
    run(state, move |e| e.quarantine_purge(req)).await
}

/// Operation report of a run (SAF-04).
#[tauri::command]
pub async fn report_get(state: State<'_, AppState>, req: RunRequest) -> Result<Report, ErrorInfo> {
    run(state, move |e| e.report_get(req)).await
}

/// "Last report" in the status strip (`null` when none).
#[tauri::command]
pub async fn report_last(state: State<'_, AppState>) -> Result<Option<Report>, ErrorInfo> {
    run(state, move |e| e.report_last()).await
}

/// Write a report (CSV/HTML) or the audit log (CSV, SAF-12).
#[tauri::command]
pub async fn report_export(state: State<'_, AppState>, req: ReportExportRequest) -> Result<ExportResult, ErrorInfo> {
    run(state, move |e| e.report_export(req)).await
}

/// Export candidates (CSV/HTML, CLN-ACT-11).
#[tauri::command]
pub async fn results_export(state: State<'_, AppState>, req: ResultsExportRequest) -> Result<ExportResult, ErrorInfo> {
    run(state, move |e| e.results_export(req)).await
}

/// All settings.
#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> Result<Settings, ErrorInfo> {
    run(state, move |e| e.settings_get()).await
}

/// Replace settings (validated, persisted, side effects applied).
#[tauri::command]
pub async fn settings_set(state: State<'_, AppState>, req: SettingsSetRequest) -> Result<Settings, ErrorInfo> {
    run(state, move |e| e.settings_set(req)).await
}

/// Built-in (locked) and user protected paths (SAF-01).
#[tauri::command]
pub async fn protected_paths_get(state: State<'_, AppState>) -> Result<ProtectedPathsInfo, ErrorInfo> {
    run(state, move |e| e.protected_paths_get()).await
}

/// Replace the user's protected paths.
#[tauri::command]
pub async fn protected_paths_set(
    state: State<'_, AppState>,
    req: ProtectedPathsSetRequest,
) -> Result<ProtectedPathsInfo, ErrorInfo> {
    run(state, move |e| e.protected_paths_set(req)).await
}

/// Scan cache size.
#[tauri::command]
pub async fn cache_info(state: State<'_, AppState>) -> Result<CacheInfo, ErrorInfo> {
    run(state, move |e| e.cache_info()).await
}

/// "Clear cache" (SCAN-04).
#[tauri::command]
pub async fn cache_clear(state: State<'_, AppState>) -> Result<CacheInfo, ErrorInfo> {
    run(state, move |e| e.cache_clear()).await
}

/// Scheduled profiles (AUT-02).
#[tauri::command]
pub async fn schedules_list(state: State<'_, AppState>) -> Result<Vec<Schedule>, ErrorInfo> {
    run(state, move |e| e.schedules_list()).await
}

/// Insert or replace a schedule.
#[tauri::command]
pub async fn schedule_save(state: State<'_, AppState>, req: ScheduleSaveRequest) -> Result<Vec<Schedule>, ErrorInfo> {
    run(state, move |e| e.schedule_save(req)).await
}

/// Delete a schedule.
#[tauri::command]
pub async fn schedule_delete(state: State<'_, AppState>, req: IdRequest) -> Result<Vec<Schedule>, ErrorInfo> {
    run(state, move |e| e.schedule_delete(req)).await
}

/// "Save as weekly schedule" (rejects lossy / permanent steps).
#[tauri::command]
pub async fn schedule_from_plan(
    state: State<'_, AppState>,
    req: ScheduleFromPlanRequest,
) -> Result<Schedule, ErrorInfo> {
    run(state, move |e| e.schedule_from_plan(req)).await
}

/// Run a schedule now (scan its scope, then apply its steps).
#[tauri::command]
pub async fn schedule_run_now(state: State<'_, AppState>, req: IdRequest) -> Result<ApplyStarted, ErrorInfo> {
    run(state, move |e| e.schedule_run_now(req)).await
}

/// Detected external tools.
#[tauri::command]
pub async fn tools_list(state: State<'_, AppState>) -> Result<Vec<ExternalTool>, ErrorInfo> {
    run(state, move |e| e.tools_list()).await
}

/// Detect external tools again.
#[tauri::command]
pub async fn tools_refresh(state: State<'_, AppState>) -> Result<Vec<ExternalTool>, ErrorInfo> {
    run(state, move |e| e.tools_refresh()).await
}

/// Volumes with free space before / after the plan (hero drive bars).
#[tauri::command]
pub async fn drives_list(state: State<'_, AppState>) -> Result<Vec<Drive>, ErrorInfo> {
    run(state, move |e| e.drives_list()).await
}

/// Folder tree by size (treemap, SCAN-05/06).
#[tauri::command]
pub async fn disk_usage_tree(state: State<'_, AppState>, req: DiskUsageRequest) -> Result<DiskNode, ErrorInfo> {
    run(state, move |e| e.disk_usage_tree(req)).await
}

/// Size by type, extension, age or owner (SCAN-07).
#[tauri::command]
pub async fn breakdown(state: State<'_, AppState>, req: BreakdownRequest) -> Result<Breakdown, ErrorInfo> {
    run(state, move |e| e.breakdown(req)).await
}

/// Names registered in `lib.rs` (kept equal to `mj_engine::api::COMMANDS` by a test).
pub const REGISTERED: &[&str] = &[
    "app_info",
    "scope_get",
    "scope_set",
    "scope_add_root",
    "scope_remove_root",
    "fs_list_dir",
    "scan_start",
    "scan_pause",
    "scan_resume",
    "scan_stop",
    "scan_status",
    "opportunities_list",
    "step_detail",
    "candidates_page",
    "selection_set",
    "selection_apply_keep_rule",
    "selection_reset",
    "step_params_set",
    "candidate_preview",
    "search_parse",
    "search_suggest",
    "search_run",
    "presets_list",
    "preset_save",
    "preset_delete",
    "plan_get",
    "plan_add_step",
    "plan_update_step",
    "plan_remove_step",
    "plan_reorder",
    "plan_clear",
    "plan_dry_run",
    "plan_confirmation_info",
    "plan_apply",
    "jobs_list",
    "job_pause",
    "job_resume",
    "job_cancel",
    "jobs_reorder",
    "history_list",
    "history_run_detail",
    "undo_last",
    "undo_run",
    "restore_entries",
    "quarantine_info",
    "quarantine_purge",
    "report_get",
    "report_last",
    "report_export",
    "results_export",
    "settings_get",
    "settings_set",
    "protected_paths_get",
    "protected_paths_set",
    "cache_info",
    "cache_clear",
    "schedules_list",
    "schedule_save",
    "schedule_delete",
    "schedule_from_plan",
    "schedule_run_now",
    "tools_list",
    "tools_refresh",
    "drives_list",
    "disk_usage_tree",
    "breakdown",
];

/// The invoke handler with every command.
#[macro_export]
macro_rules! mj_handlers {
    () => {
        tauri::generate_handler![
            $crate::commands::app_info,
            $crate::commands::scope_get,
            $crate::commands::scope_set,
            $crate::commands::scope_add_root,
            $crate::commands::scope_remove_root,
            $crate::commands::fs_list_dir,
            $crate::commands::scan_start,
            $crate::commands::scan_pause,
            $crate::commands::scan_resume,
            $crate::commands::scan_stop,
            $crate::commands::scan_status,
            $crate::commands::opportunities_list,
            $crate::commands::step_detail,
            $crate::commands::candidates_page,
            $crate::commands::selection_set,
            $crate::commands::selection_apply_keep_rule,
            $crate::commands::selection_reset,
            $crate::commands::step_params_set,
            $crate::commands::candidate_preview,
            $crate::commands::search_parse,
            $crate::commands::search_suggest,
            $crate::commands::search_run,
            $crate::commands::presets_list,
            $crate::commands::preset_save,
            $crate::commands::preset_delete,
            $crate::commands::plan_get,
            $crate::commands::plan_add_step,
            $crate::commands::plan_update_step,
            $crate::commands::plan_remove_step,
            $crate::commands::plan_reorder,
            $crate::commands::plan_clear,
            $crate::commands::plan_dry_run,
            $crate::commands::plan_confirmation_info,
            $crate::commands::plan_apply,
            $crate::commands::jobs_list,
            $crate::commands::job_pause,
            $crate::commands::job_resume,
            $crate::commands::job_cancel,
            $crate::commands::jobs_reorder,
            $crate::commands::history_list,
            $crate::commands::history_run_detail,
            $crate::commands::undo_last,
            $crate::commands::undo_run,
            $crate::commands::restore_entries,
            $crate::commands::quarantine_info,
            $crate::commands::quarantine_purge,
            $crate::commands::report_get,
            $crate::commands::report_last,
            $crate::commands::report_export,
            $crate::commands::results_export,
            $crate::commands::settings_get,
            $crate::commands::settings_set,
            $crate::commands::protected_paths_get,
            $crate::commands::protected_paths_set,
            $crate::commands::cache_info,
            $crate::commands::cache_clear,
            $crate::commands::schedules_list,
            $crate::commands::schedule_save,
            $crate::commands::schedule_delete,
            $crate::commands::schedule_from_plan,
            $crate::commands::schedule_run_now,
            $crate::commands::tools_list,
            $crate::commands::tools_refresh,
            $crate::commands::drives_list,
            $crate::commands::disk_usage_tree,
            $crate::commands::breakdown,
        ]
    };
}
