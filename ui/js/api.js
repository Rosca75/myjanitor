// @ts-check
/**
 * The ONLY module that touches `window.__TAURI__`. Owner: Wave 1 (frozen
 * shape; W2-F/W3-D may add helpers below the generated section).
 *
 * Every command of docs/IPC.md is exposed as `api.<camelCaseName>(req)`, which
 * calls `<snake_case_name>` with `{ req }`. Commands reject with an ErrorInfo
 * object `{ code, message, hint, path? }`. Events: `api.on(name, cb)` returns a
 * Promise of an unlisten function; `cb` receives the payload.
 *
 * Backend switch: the mock backend (ui/js/mock/backend.js) is used when
 * `window.__TAURI__` is missing (plain browser, node --test), when the page URL
 * has `?mock=1`, or after `useMock(true)`.
 */

/** Command names, in docs/IPC.md order. */
export const COMMANDS = Object.freeze([
  'app_info',
  'scope_get',
  'scope_set',
  'scope_add_root',
  'scope_remove_root',
  'fs_list_dir',
  'scan_start',
  'scan_pause',
  'scan_resume',
  'scan_stop',
  'scan_status',
  'opportunities_list',
  'step_detail',
  'candidates_page',
  'selection_set',
  'selection_apply_keep_rule',
  'selection_reset',
  'step_params_set',
  'candidate_preview',
  'search_parse',
  'search_suggest',
  'search_run',
  'presets_list',
  'preset_save',
  'preset_delete',
  'plan_get',
  'plan_add_step',
  'plan_update_step',
  'plan_remove_step',
  'plan_reorder',
  'plan_clear',
  'plan_dry_run',
  'plan_confirmation_info',
  'plan_apply',
  'jobs_list',
  'job_pause',
  'job_resume',
  'job_cancel',
  'jobs_reorder',
  'history_list',
  'history_run_detail',
  'undo_last',
  'undo_run',
  'restore_entries',
  'quarantine_info',
  'quarantine_purge',
  'report_get',
  'report_last',
  'report_export',
  'results_export',
  'settings_get',
  'settings_set',
  'protected_paths_get',
  'protected_paths_set',
  'cache_info',
  'cache_clear',
  'schedules_list',
  'schedule_save',
  'schedule_delete',
  'schedule_from_plan',
  'schedule_run_now',
  'tools_list',
  'tools_refresh',
  'drives_list',
  'disk_usage_tree',
  'breakdown',
]);

/** Event names, in docs/IPC.md order. */
export const EVENTS = Object.freeze([
  'scan://progress',
  'scan://detector',
  'scan://done',
  'opportunities://changed',
  'plan://changed',
  'plan://dry_run',
  'job://progress',
  'job://state',
  'job://done',
  'history://changed',
]);

/**
 * @typedef {{ call(cmd: string, req?: object): Promise<any>, listen(event: string, cb: (payload: any) => void): Promise<() => void>, kind: string }} Backend
 */

/** @type {Backend | null} */
let backend = null;
/** @type {boolean | null} */
let forceMock = null;

function wantsMock() {
  if (forceMock !== null) return forceMock;
  const g = /** @type {any} */ (globalThis);
  if (!g.__TAURI__ || !g.__TAURI__.core) return true;
  try {
    return new URLSearchParams(g.location?.search ?? '').get('mock') === '1';
  } catch {
    return false;
  }
}

/** @returns {Backend} */
function tauriBackend() {
  const t = /** @type {any} */ (globalThis).__TAURI__;
  return {
    kind: 'tauri',
    call: (cmd, req) => t.core.invoke(cmd, req === undefined ? {} : { req }),
    listen: (event, cb) => t.event.listen(event, (/** @type {any} */ e) => cb(e.payload)),
  };
}

/** @returns {Promise<Backend>} */
async function getBackend() {
  if (backend) return backend;
  if (wantsMock()) {
    const { createMockBackend } = await import('./mock/backend.js');
    backend = createMockBackend();
  } else {
    backend = tauriBackend();
  }
  return backend;
}

/**
 * Force the mock (true), the real backend (false) or auto-detection (null).
 * @param {boolean | null} on
 */
export function useMock(on) {
  forceMock = on;
  backend = null;
}

/** @returns {Promise<string>} `"tauri"` or `"mock"`. */
export async function backendKind() {
  return (await getBackend()).kind;
}

/**
 * Call a command by name.
 * @param {string} cmd
 * @param {object} [req]
 */
export async function call(cmd, req) {
  if (!COMMANDS.includes(cmd)) {
    throw { code: 'invalid_input', message: `Unknown command: ${cmd}`, hint: 'This is a bug in the UI.' };
  }
  return (await getBackend()).call(cmd, req);
}

/**
 * Subscribe to an event.
 * @param {string} event
 * @param {(payload: any) => void} cb
 * @returns {Promise<() => void>}
 */
export async function on(event, cb) {
  if (!EVENTS.includes(event)) {
    throw { code: 'invalid_input', message: `Unknown event: ${event}`, hint: 'This is a bug in the UI.' };
  }
  return (await getBackend()).listen(event, cb);
}

/** One function per command (generated from docs/IPC.md). */
export const api = Object.freeze({
  /** Version, platform, data/cache folders, portable and elevation state. @returns {Promise<any>} AppInfo */
  appInfo: () => call('app_info'),
  /** The session scan scope (scope chips). @returns {Promise<any>} ScanScope */
  scopeGet: () => call('scope_get'),
  /** Replace the scope (roots, includes, excludes, options). @param {object} req ScopeSetRequest @returns {Promise<any>} ScanScope */
  scopeSet: (req) => call('scope_set', req),
  /** "+ Add location": add one root (normalised; nested roots collapse). @param {object} req ScopePathRequest @returns {Promise<any>} ScanScope */
  scopeAddRoot: (req) => call('scope_add_root', req),
  /** Remove a scope chip. @param {object} req ScopePathRequest @returns {Promise<any>} ScanScope */
  scopeRemoveRoot: (req) => call('scope_remove_root', req),
  /** In-app folder browser (D-104): list a folder, or the places when `path` is omitted. @param {object} req FsListDirRequest @returns {Promise<any>} FsListDirResponse */
  fsListDir: (req) => call('fs_list_dir', req),
  /** Start a scan of the scope; progress via `scan://progress`, `scan://detector`, end via `scan://done`. @param {object} req ScanStartRequest @returns {Promise<any>} ScanStarted */
  scanStart: (req) => call('scan_start', req),
  /** Pause the running scan. @returns {Promise<any>} ScanStatus */
  scanPause: () => call('scan_pause'),
  /** Resume a paused scan. @returns {Promise<any>} ScanStatus */
  scanResume: () => call('scan_resume'),
  /** Stop the scan; results so far are discarded, the previous results stay. @returns {Promise<any>} ScanStatus */
  scanStop: () => call('scan_stop'),
  /** Current scan state (used at start-up and after reloads). @returns {Promise<any>} ScanStatus */
  scanStatus: () => call('scan_status'),
  /** Ranked opportunity cards for a tab, the hero summary and tab counts. @param {object} req OpportunitiesRequest @returns {Promise<any>} OpportunitiesList */
  opportunitiesList: (req) => call('opportunities_list', req),
  /** Step review header and right panel: opportunity, settings, presets, summary, diagnostics. @param {object} req OpportunityRequest @returns {Promise<any>} StepDetail */
  stepDetail: (req) => call('step_detail', req),
  /** One page of the step's candidates table (sub-group, sort, filter). @param {object} req CandidatesPageRequest @returns {Promise<any>} CandidatesPage */
  candidatesPage: (req) => call('candidates_page', req),
  /** Check / uncheck rows (or a whole sub-group). @param {object} req SelectionSetRequest @returns {Promise<any>} StepSummary */
  selectionSet: (req) => call('selection_set', req),
  /** Smart auto-select in groups (CLN-ACT-01). @param {object} req SelectionKeepRuleRequest @returns {Promise<any>} StepSummary */
  selectionApplyKeepRule: (req) => call('selection_apply_keep_rule', req),
  /** Back to the default selection. @param {object} req OpportunityRequest @returns {Promise<any>} StepSummary */
  selectionReset: (req) => call('selection_reset', req),
  /** Change presets / toggles of a step and re-estimate. @param {object} req StepParamsRequest @returns {Promise<any>} StepParamsResponse */
  stepParamsSet: (req) => call('step_params_set', req),
  /** Preview for one row (thumbnail, text head, waveform). @param {object} req CandidatePreviewRequest @returns {Promise<any>} Preview */
  candidatePreview: (req) => call('candidate_preview', req),
  /** Parse a quick query into a rule tree, with error positions. @param {object} req SearchParseRequest @returns {Promise<any>} SearchParseResponse */
  searchParse: (req) => call('search_parse', req),
  /** Autocomplete for the search box. @param {object} req SearchSuggestRequest @returns {Promise<any>} Vec<Suggestion> */
  searchSuggest: (req) => call('search_suggest', req),
  /** Run a rule over the index; creates or replaces a `custom:<n>` opportunity. @param {object} req SearchRunRequest @returns {Promise<any>} Opportunity */
  searchRun: (req) => call('search_run', req),
  /** Built-in and user rule presets. @returns {Promise<any>} Vec<RulePreset> */
  presetsList: () => call('presets_list'),
  /** Save a user preset (insert or replace by id). @param {object} req PresetSaveRequest @returns {Promise<any>} Vec<RulePreset> */
  presetSave: (req) => call('preset_save', req),
  /** Delete a user preset (built-ins cannot be deleted). @param {object} req IdRequest @returns {Promise<any>} Vec<RulePreset> */
  presetDelete: (req) => call('preset_delete', req),
  /** The plan with steps, estimate and checklist. @returns {Promise<any>} Plan */
  planGet: () => call('plan_get'),
  /** "+ Add" on a card. @param {object} req PlanAddStepRequest @returns {Promise<any>} Plan */
  planAddStep: (req) => call('plan_add_step', req),
  /** Change a step's action or parameters. @param {object} req PlanUpdateStepRequest @returns {Promise<any>} Plan */
  planUpdateStep: (req) => call('plan_update_step', req),
  /** "×" on a plan step. @param {object} req StepRequest @returns {Promise<any>} Plan */
  planRemoveStep: (req) => call('plan_remove_step', req),
  /** Drag-and-drop order of steps. @param {object} req PlanReorderRequest @returns {Promise<any>} Plan */
  planReorder: (req) => call('plan_reorder', req),
  /** Remove every step. @returns {Promise<any>} Plan */
  planClear: () => call('plan_clear'),
  /** "Estimate exactly (dry run)": queues a dry-run job; result via `plan://dry_run`. @returns {Promise<any>} Job */
  planDryRun: () => call('plan_dry_run'),
  /** Content of the lossy / permanent confirmation dialog for one step. @param {object} req StepRequest @returns {Promise<any>} ConfirmationInfo */
  planConfirmationInfo: (req) => call('plan_confirmation_info', req),
  /** "Apply plan": queues one job per step; fails with `confirmation_required` if a step lacks its confirmation. @param {object} req PlanApplyRequest @returns {Promise<any>} ApplyStarted */
  planApply: (req) => call('plan_apply', req),
  /** Queue contents (running, queued, recently finished). @returns {Promise<any>} Vec<Job> */
  jobsList: () => call('jobs_list'),
  /** Pause a job (between items). @param {object} req JobRequest @returns {Promise<any>} Job */
  jobPause: (req) => call('job_pause', req),
  /** Resume a paused job. @param {object} req JobRequest @returns {Promise<any>} Job */
  jobResume: (req) => call('job_resume', req),
  /** Cancel a job (finished items stay applied and undoable). @param {object} req JobRequest @returns {Promise<any>} Job */
  jobCancel: (req) => call('job_cancel', req),
  /** Reorder queued jobs. @param {object} req JobsReorderRequest @returns {Promise<any>} Vec<Job> */
  jobsReorder: (req) => call('jobs_reorder', req),
  /** History: runs newest first. @param {object} req PageRequest @returns {Promise<any>} HistoryList */
  historyList: (req) => call('history_list', req),
  /** Journal entries of one run. @param {object} req RunDetailRequest @returns {Promise<any>} HistoryRunDetail */
  historyRunDetail: (req) => call('history_run_detail', req),
  /** "Undo last action": undo the newest undoable run. @returns {Promise<any>} Job */
  undoLast: () => call('undo_last'),
  /** Restore every undoable entry of a run. @param {object} req RunRequest @returns {Promise<any>} Job */
  undoRun: (req) => call('undo_run', req),
  /** Restore selected entries. @param {object} req RestoreEntriesRequest @returns {Promise<any>} Job */
  restoreEntries: (req) => call('restore_entries', req),
  /** Quarantine folder size and age. @returns {Promise<any>} QuarantineInfo */
  quarantineInfo: () => call('quarantine_info'),
  /** Permanently delete expired quarantine items. @param {object} req QuarantinePurgeRequest @returns {Promise<any>} Job */
  quarantinePurge: (req) => call('quarantine_purge', req),
  /** Operation report of a run (SAF-04). @param {object} req RunRequest @returns {Promise<any>} Report */
  reportGet: (req) => call('report_get', req),
  /** "Last report" in the status strip (`null` when none). @returns {Promise<any>} Option<Report> */
  reportLast: () => call('report_last'),
  /** Write a report (CSV/HTML) or the audit log (CSV, SAF-12). @param {object} req ReportExportRequest @returns {Promise<any>} ExportResult */
  reportExport: (req) => call('report_export', req),
  /** Export candidates (CSV/HTML, CLN-ACT-11). @param {object} req ResultsExportRequest @returns {Promise<any>} ExportResult */
  resultsExport: (req) => call('results_export', req),
  /** All settings. @returns {Promise<any>} Settings */
  settingsGet: () => call('settings_get'),
  /** Replace settings (validated, persisted, side effects applied). @param {object} req SettingsSetRequest @returns {Promise<any>} Settings */
  settingsSet: (req) => call('settings_set', req),
  /** Built-in (locked) and user protected paths (SAF-01). @returns {Promise<any>} ProtectedPathsInfo */
  protectedPathsGet: () => call('protected_paths_get'),
  /** Replace the user's protected paths. @param {object} req ProtectedPathsSetRequest @returns {Promise<any>} ProtectedPathsInfo */
  protectedPathsSet: (req) => call('protected_paths_set', req),
  /** Scan cache size. @returns {Promise<any>} CacheInfo */
  cacheInfo: () => call('cache_info'),
  /** "Clear cache" (SCAN-04). @returns {Promise<any>} CacheInfo */
  cacheClear: () => call('cache_clear'),
  /** Scheduled profiles (AUT-02). @returns {Promise<any>} Vec<Schedule> */
  schedulesList: () => call('schedules_list'),
  /** Insert or replace a schedule. @param {object} req ScheduleSaveRequest @returns {Promise<any>} Vec<Schedule> */
  scheduleSave: (req) => call('schedule_save', req),
  /** Delete a schedule. @param {object} req IdRequest @returns {Promise<any>} Vec<Schedule> */
  scheduleDelete: (req) => call('schedule_delete', req),
  /** "Save as weekly schedule" (rejects lossy / permanent steps). @param {object} req ScheduleFromPlanRequest @returns {Promise<any>} Schedule */
  scheduleFromPlan: (req) => call('schedule_from_plan', req),
  /** Run a schedule now (scan its scope, then apply its steps). @param {object} req IdRequest @returns {Promise<any>} ApplyStarted */
  scheduleRunNow: (req) => call('schedule_run_now', req),
  /** Detected external tools. @returns {Promise<any>} Vec<ExternalTool> */
  toolsList: () => call('tools_list'),
  /** Detect external tools again. @returns {Promise<any>} Vec<ExternalTool> */
  toolsRefresh: () => call('tools_refresh'),
  /** Volumes with free space before / after the plan (hero drive bars). @returns {Promise<any>} Vec<Drive> */
  drivesList: () => call('drives_list'),
  /** Folder tree by size (treemap, SCAN-05/06). @param {object} req DiskUsageRequest @returns {Promise<any>} DiskNode */
  diskUsageTree: (req) => call('disk_usage_tree', req),
  /** Size by type, extension, age or owner (SCAN-07). @param {object} req BreakdownRequest @returns {Promise<any>} Breakdown */
  breakdown: (req) => call('breakdown', req),
  on,
});

export default api;
