# MyJanitor IPC contract

**Frozen in Wave 1.** Changes go through the orchestrator. This file is
machine-checked:

- every `### \`name\`` heading is a command or an event, in the order of
  `mj_engine::api::COMMANDS` then `EVENTS` (test `every_command_is_unique_and_documented`
  in `crates/engine/src/tests.rs`, and `ui/js/api.test.js`);
- every fenced block opened with three backticks followed by `json <TypeName>`
  must deserialize into that Rust type (test `ipc_examples_parse`).

The same API is used by the Tauri shell (`src-tauri/src/commands.rs`, one thin
wrapper per command), the CLI (`crates/cli`, calling `Engine` methods directly)
and the mock backend (`ui/js/mock/backend.js`). The engine method for each
command has the command's name: `Engine::plan_add_step(req)`.

## 1. Conventions

| Topic | Rule |
|---|---|
| Call | `window.__TAURI__.core.invoke(name, { req })` for commands with a request, `invoke(name, {})` otherwise. From JS always go through `ui/js/api.js` (`api.planAddStep(req)`). |
| Field names | Exactly the Rust snake_case names (`opportunity_id`, `bytes_before`). No camelCase anywhere in payloads. |
| Requests | Unknown fields are rejected (`invalid_input`). Fields marked optional may be omitted. |
| Responses | Optional fields are **omitted** when absent (not `null`), except `Option` results such as `report_last`, which return `null`. |
| Enums | snake_case strings (`"lossy_opt_in"`). Tagged unions carry a tag field: `Action` → `kind`, `OriginalsPolicy` → `policy`, `OutputMode` → `mode`, `Condition` → `field`, `SettingKind` → `type`, `Availability` → `state`, `KeepRule` → `rule`, `Cadence` → `every`. `Rule` and `Source` are externally tagged (`{"all":[...]}`, `{"detector":"temp_files"}`). |
| Flags | `Badges`, `Attributes` and `FingerprintFlags` are arrays of names (`["duplicate","synced"]`). Unknown names are rejected. |
| Paths | Strings. A path that is not valid Unicode is sent as `"\u0000"` + an escaped form (`%XX` bytes on Linux, `%uXXXX` surrogates on Windows); it round-trips exactly. Display it with the replacement; never build paths in JS by string concatenation except for display. |
| Timestamps | Integer **nanoseconds** since 1970-01-01 UTC. JS: `new Date(ts / 1e6)`. Precision loss in JS (~256 ns) is irrelevant. |
| Sizes | Integer bytes. Display with binary units and Explorer labels (`1.5 KB` = 1536 B, D-107). |
| Ids | Integers below 2^53 (`CandidateId`, `GroupId`, `StepId`, `JobId`, `RunId`, `EntryId`), strings for `OpportunityId` (`det:<detector>`, `opt:<optimiser>`, `custom:<n>`), preset and schedule ids. Candidate ids are valid for one scan generation only. |
| Errors | Rejected promise with an `ErrorInfo` (`{code, message, hint, path?}`). Per-item failures of a batch are **not** command errors: they are in `JobOutcome.errors`, reports and journal entries. |
| Long work | Commands marked *spawns* return immediately (a `Job` or a generation) and report through events. All wrappers run off the UI thread. |
| Generations | Every scan has a `generation`. Results and events from an older generation must be ignored by the UI. |
| Throttling | `scan://progress` and `job://progress` are sent at most once per 50 ms per source; final values are always sent. |

### 1.1 Error codes

| `code` | Meaning |
|---|---|
| `io` | Generic I/O failure (`message` has the OS text). |
| `not_found` | The path no longer exists. |
| `permission_denied` | The OS refused access. |
| `unsupported` | Not possible here (platform, filesystem). Show the feature disabled with the message. |
| `not_implemented` | Planned but not built yet. Show the feature disabled. |
| `cancelled` | The user cancelled. |
| `invalid_input` | Bad request (unknown id, bad value). |
| `protected` | A protected path (SAF-01). |
| `outside_allowed_roots` | A destructive action outside the scanned locations. |
| `changed` | The file changed since the scan (TOCTOU guard). |
| `link_not_followed` | A symlink/junction was not followed. |
| `parse` | A file could not be parsed; it was left unchanged. |
| `tool_missing` | An external tool is missing. |
| `conflict` | State conflict (a scan already runs, a step already in the plan). |
| `confirmation_required` | `plan_apply` without the confirmation of a lossy/permanent step. |
| `internal` | A bug. |

```json ErrorInfo
{"code":"confirmation_required","message":"Confirmation required: step 7 (Meeting recordings) needs confirmation","hint":"Review the step and confirm it in the dialog."}
```

## 2. Screens to commands

| Screen / state | Commands | Events |
|---|---|---|
| Start-up | `app_info`, `scope_get`, `scan_status`, `opportunities_list`, `plan_get`, `jobs_list`, `report_last`, `drives_list`, `tools_list` | all |
| Empty state | `fs_list_dir`, `scope_add_root`, `scan_start` | |
| Scanning | `scan_pause`, `scan_resume`, `scan_stop`, `scan_status` | `scan://progress`, `scan://detector`, `scan://done` |
| B1 Overview | `opportunities_list`, `drives_list`, `plan_add_step`, `plan_remove_step`, `plan_reorder`, `plan_dry_run`, `schedule_from_plan` | `opportunities://changed`, `plan://changed`, `plan://dry_run` |
| B2 Step review | `step_detail`, `candidates_page`, `selection_set`, `selection_apply_keep_rule`, `selection_reset`, `step_params_set`, `candidate_preview`, `results_export` | |
| Confirmation dialog | `plan_confirmation_info`, `plan_apply` | |
| Action running | `jobs_list`, `job_pause`, `job_resume`, `job_cancel`, `jobs_reorder` | `job://progress`, `job://state`, `job://done` |
| Completed | `report_last`, `report_get`, `report_export`, `undo_last` | `history://changed` |
| History | `history_list`, `history_run_detail`, `undo_run`, `restore_entries`, `quarantine_info`, `quarantine_purge` | `history://changed` |
| Custom search | `search_parse`, `search_suggest`, `search_run`, `presets_list`, `preset_save`, `preset_delete` | |
| Settings drawer | `settings_get`, `settings_set`, `protected_paths_get`, `protected_paths_set`, `cache_info`, `cache_clear`, `schedules_list`, `schedule_save`, `schedule_delete`, `schedule_run_now`, `tools_refresh` | |
| Disk usage | `disk_usage_tree`, `breakdown` | |

## 3. Commands

Format: **request type** → **response type**. Examples of every type are in §5.

### `app_info`
No request → `AppInfo`. Version, platform (`windows`/`linux`), folders, portable
mode, elevation, and whether the undo journal is persistent (if `false`, show a
warning that History will not survive a restart).

### `scope_get`
No request → `ScanScope`. The session scope (scope chips).

### `scope_set`
`ScopeSetRequest` → `ScanScope`. Roots must be absolute; nested roots collapse
into their parent (`/a` and `/a/b` → `/a`).

```json ScopeSetRequest
{"scope":{"roots":["/home/me/Downloads","/home/me/Projects"],"include":[],"exclude":["**/.cache/thumbnails"],"options":{"include_hidden":true,"use_fast_index":true,"use_cache":true,"one_file_system":true,"max_depth":null,"threads":0,"collect_owner":false}}}
```

### `scope_add_root`
`ScopePathRequest` → `ScanScope`. "+ Add location". Fails with `invalid_input`
when the path is not an existing folder.

```json ScopePathRequest
{"path":"C:\\Users\\Michel"}
```

### `scope_remove_root`
`ScopePathRequest` → `ScanScope`. Removes a chip (compared case-insensitively on Windows).

### `fs_list_dir`
`FsListDirRequest` → `FsListDirResponse`. The in-app folder browser used for
"+ Add location", move/copy/backup destinations and export targets (D-104: no
dialog plugin). Without `path` it returns only `places` (Home, Desktop,
Documents, Downloads, Pictures, Videos, Music, drives / mount points). Read-only.

```json FsListDirRequest
{"path":"/home/me","show_hidden":false,"include_files":false}
```

### `scan_start`
`ScanStartRequest` → `ScanStarted`. *Spawns.* Optionally replaces the scope
first. Fails with `conflict` when a scan is running and `invalid_input` when the
scope has no root. Progress via `scan://progress`, live cards via
`scan://detector`, end via `scan://done`; then `opportunities://changed`.

```json ScanStartRequest
{}
```

```json ScanStarted
{"generation":3}
```

### `scan_pause`
No request → `ScanStatus`. No-op unless scanning.

### `scan_resume`
No request → `ScanStatus`. No-op unless paused.

### `scan_stop`
No request → `ScanStatus` (state `stopping`, then `scan://done` with `cancelled`).
The previous results stay visible.

### `scan_status`
No request → `ScanStatus`. Used at start-up and after a webview reload.

### `opportunities_list`
`OpportunitiesRequest` → `OpportunitiesList`. Cards with at least one candidate,
filtered by tab and sorted. `summary` and `counts` always cover every tab.
Tabs: `clean`/`optimise`/`archive` exclude review-only cards, `review_only`
holds them. Disabled cards (`availability.state = "disabled"`) are included
so the UI can show them greyed with the reason.

```json OpportunitiesRequest
{"tab":"all","sort":"saving"}
```

### `step_detail`
`OpportunityRequest` → `StepDetail`. B2 header, summary line, sub-group tabs
(`opportunity.sub_groups`), right panel (`opportunity.presets`, `opportunity.settings`,
`opportunity.params`) and the diagnostics note.

```json OpportunityRequest
{"opportunity_id":"det:exact_duplicates"}
```

### `candidates_page`
`CandidatesPageRequest` → `CandidatesPage`. One page (`limit` 1-500, default 100)
of the step's table. `sort` defaults to `saving`; `descending` reverses the
ascending order. `groups` contains the groups referenced by the page's rows.

```json CandidatesPageRequest
{"opportunity_id":"det:exact_duplicates","sub_group":null,"offset":0,"limit":100,"sort":"group","descending":false,"filter":"photos"}
```

### `selection_set`
`SelectionSetRequest` → `StepSummary`. Checks or unchecks rows. With `ids`
omitted, applies to the whole `sub_group` (or the whole step). Protected rows
never become selected. Emits `opportunities://changed`.

```json SelectionSetRequest
{"opportunity_id":"opt:audio","ids":[1041,1042],"selected":false}
```

### `selection_apply_keep_rule`
`SelectionKeepRuleRequest` → `StepSummary`. CLN-ACT-01: in every group the
keeper chosen by the rule is unchecked and the other members checked.

```json SelectionKeepRuleRequest
{"opportunity_id":"det:exact_duplicates","rule":{"rule":"preferred_folder","folder":"/home/me/Photos"}}
```

### `selection_reset`
`OpportunityRequest` → `StepSummary`. Back to each candidate's `default_selected`.

### `step_params_set`
`StepParamsRequest` → `StepParamsResponse`. Applies a preset tile (`preset_id`)
and/or merges toggle/number values into the step parameters, then re-estimates.
The result says whether the step is now lossy (and its safety class), which
switches the plan step and the badge to *lossy · opt-in*. If the opportunity is
in the plan the plan step is updated too (`plan://changed`).

```json StepParamsRequest
{"opportunity_id":"opt:audio","preset_id":"speech","params":{"downmix_mono":true,"remove_silences":true,"max_pause_s":2,"strip_cover_art":true}}
```

### `candidate_preview`
`CandidatePreviewRequest` → `Preview`. Thumbnail (base64, ≤ 512 px), first 4 KiB
of text, or a waveform with the silences the current settings would remove.
`kind: "none"` with a `reason` when not available.

```json CandidatePreviewRequest
{"candidate_id":1041}
```

### `search_parse`
`SearchParseRequest` → `SearchParseResponse`. Quick query → rule tree (grammar
in §6). Invalid queries return `errors` with byte offsets for underlining; the
command itself does not fail.

```json SearchParseRequest
{"query":"ext:mp4 size:>1GB modified:<2024"}
```

### `search_suggest`
`SearchSuggestRequest` → `Suggestion[]`. Autocomplete for the token at `cursor`.

```json SearchSuggestRequest
{"query":"ext:mp4 si","cursor":10}
```

### `search_run`
`SearchRunRequest` → `Opportunity`. Evaluates the rule over the current index
and creates (or replaces, same title) a `custom:<n>` opportunity that then
behaves like any card (step review, plan). Fails with `invalid_input` before
the first scan.

```json SearchRunRequest
{"rule":{"all":[{"cond":{"field":"extension","values":["mp4"]}},{"cond":{"field":"size","cmp":"gt","bytes":1073741824}}]},"title":"Big videos","topmost_only":false,"action":{"kind":"trash"}}
```

### `presets_list`
No request → `RulePreset[]`. Built-ins first (`builtin: true`), then user presets.

### `preset_save`
`PresetSaveRequest` → `RulePreset[]`. Insert or replace by `id`; an empty id
creates a new preset. Built-ins cannot be overwritten.

```json PresetSaveRequest
{"preset":{"id":"","name":"Old screen recordings","description":"Screen recordings older than 6 months","rule":{"all":[{"cond":{"field":"path","mode":"wildcard","pattern":"**/Screen Recordings/**","case_sensitive":false}},{"cond":{"field":"date","which":"modified","cmp":"lt","value":{"ago":{"amount":6,"unit":"months"}}}}]},"topmost_only":false,"builtin":false}}
```

### `preset_delete`
`IdRequest` → `RulePreset[]`. Built-ins cannot be deleted.

```json IdRequest
{"id":"user_42"}
```

### `plan_get`
No request → `Plan`.

### `plan_add_step`
`PlanAddStepRequest` → `Plan`. "+ Add" on a card. `action` and `params` default
to the opportunity's. Fails with `conflict` if already in the plan and
`unsupported` for disabled opportunities. Emits `plan://changed`.

```json PlanAddStepRequest
{"opportunity_id":"det:dev_artefacts"}
```

### `plan_update_step`
`PlanUpdateStepRequest` → `Plan`. Change the action (e.g. Trash → Quarantine)
or merge parameters.

```json PlanUpdateStepRequest
{"step_id":12,"action":{"kind":"quarantine","purge_after_days":7}}
```

### `plan_remove_step`
`StepRequest` → `Plan`. "×" on a plan step.

```json StepRequest
{"step_id":12}
```

### `plan_reorder`
`PlanReorderRequest` → `Plan`. Must list every step id exactly once.

```json PlanReorderRequest
{"step_ids":[14,12,13,15]}
```

### `plan_clear`
No request → `Plan`.

### `plan_dry_run`
No request → `Job` (`kind: "dry_run"`). *Spawns.* "Estimate exactly (dry run)":
every check of a real run without changing anything; optimisations are run
into the temp folder to measure. The result arrives as `plan://dry_run`; the
plan's step estimates become `confidence: "exact"` where measured.

### `plan_confirmation_info`
`StepRequest` → `ConfirmationInfo`. Everything the lossy / permanent dialog
shows (SAF-11): tiles, originals options with the default (Recycle Bin), the
synced-folder warning with the online-only alternative (SAF-10, CLN-ACT-07),
the acknowledgement text and the acting-button label. The UI renders the
acting button in the warning colour (`permanent: false`) or the danger colour
(`permanent: true`), disabled until the checkbox is ticked.

### `plan_apply`
`PlanApplyRequest` → `ApplyStarted`. *Spawns.* Needs one `StepConfirmation`
with `acknowledged: true` for every step whose safety class requires
confirmation (`lossy_opt_in`, `permanent`); otherwise fails with
`confirmation_required` and nothing runs. Creates a run (`RunId`), queues one
job per step in plan order; progress via `job://*`, then `history://changed`
and `plan://changed` (applied steps leave the plan).

```json PlanApplyRequest
{"confirmations":[{"step_id":15,"acknowledged":true,"originals":{"policy":"recycle_bin"},"synced_online_only":true}]}
```

### `jobs_list`
No request → `Job[]`. Running and queued jobs plus the jobs finished this session.

### `job_pause`
`JobRequest` → `Job`. Takes effect between items.

```json JobRequest
{"job_id":31}
```

### `job_resume`
`JobRequest` → `Job`.

### `job_cancel`
`JobRequest` → `Job`. Queued jobs become `cancelled` at once; running ones go
through `cancelling`. Items already applied stay applied (and undoable).

### `jobs_reorder`
`JobsReorderRequest` → `Job[]`. Reorders queued jobs only.

```json JobsReorderRequest
{"job_ids":[33,32]}
```

### `history_list`
`PageRequest` → `HistoryList`. Runs newest first.

```json PageRequest
{"offset":0,"limit":50}
```

### `history_run_detail`
`RunDetailRequest` → `HistoryRunDetail`. Journal entries of one run (paged).

```json RunDetailRequest
{"run_id":1794582245123,"offset":0,"limit":100}
```

### `undo_last`
No request → `Job` (`kind: "undo"`). *Spawns.* "Undo last action": undoes every
undoable entry of the newest run that has one. `invalid_input` when nothing
can be undone.

### `undo_run`
`RunRequest` → `Job`. *Spawns.* History → Restore for a whole run.

```json RunRequest
{"run_id":1794582245123}
```

### `restore_entries`
`RestoreEntriesRequest` → `Job`. *Spawns.* Restore selected entries.

```json RestoreEntriesRequest
{"entry_ids":[1794582245123000001,1794582245123000002]}
```

### `quarantine_info`
No request → `QuarantineInfo`.

### `quarantine_purge`
`QuarantinePurgeRequest` → `Job`. *Spawns.* Permanently deletes quarantined
items older than `older_than_days` (default: the setting); needs no extra
confirmation from the engine, so the UI asks first (it is irreversible).

```json QuarantinePurgeRequest
{"older_than_days":30}
```

### `report_get`
`RunRequest` → `Report`.

### `report_last`
No request → `Report` or `null`. "Last report" in the status strip.

### `report_export`
`ReportExportRequest` → `ExportResult`. `kind: "report"` writes one run's
report as CSV or HTML; `kind: "audit"` writes the audit CSV (SAF-12) of one run
or, without `run_id`, of the whole journal. Never overwrites: `name (1).csv`.
Columns (CSV, UTF-8 with BOM): report — `step, path, status, bytes_before,
bytes_after, saved, error`; audit — `time, run_id, entry_id, action, state,
path, retained_at, bytes_before, bytes_after, sha256, lossy, error`.

```json ReportExportRequest
{"run_id":1794582245123,"kind":"audit","format":"csv","dest":"/home/me/Documents/myjanitor-audit.csv"}
```

### `results_export`
`ResultsExportRequest` → `ExportResult`. CLN-ACT-11: candidates of one or all
opportunities. Columns: `opportunity, path, size, size_after, saving, reason,
badges, selected, group, modified`.

```json ResultsExportRequest
{"opportunity_id":"det:old_installers","selected_only":true,"format":"html","dest":"/home/me/Documents/installers.html"}
```

### `settings_get`
No request → `Settings`.

### `settings_set`
`SettingsSetRequest` → `Settings`. Replaces the whole object; validated
(`invalid_input` with the field in the message), persisted atomically, side
effects applied (context menu, watcher). `optimise.keep_if_not_smaller` cannot
be turned off.

### `protected_paths_get`
No request → `ProtectedPathsInfo`. Built-ins are shown locked.

### `protected_paths_set`
`ProtectedPathsSetRequest` → `ProtectedPathsInfo`. Absolute paths only.

```json ProtectedPathsSetRequest
{"user":["/home/me/Clients/Archive"]}
```

### `cache_info`
No request → `CacheInfo`.

### `cache_clear`
No request → `CacheInfo`. "Clear cache" (SCAN-04).

### `schedules_list`
No request → `Schedule[]`.

### `schedule_save`
`ScheduleSaveRequest` → `Schedule[]`. Insert or replace by id (empty id = new).
Steps whose action needs confirmation are rejected (`invalid_input`):
lossy and permanent steps never run unattended (SAF-11).

### `schedule_delete`
`IdRequest` → `Schedule[]`.

### `schedule_from_plan`
`ScheduleFromPlanRequest` → `Schedule`. "Save as weekly schedule": the scope
plus the plan's steps (without per-candidate selections, which are not stable
across scans).

```json ScheduleFromPlanRequest
{"name":"Weekly cleanup","cadence":{"every":"weekly","weekday":1,"hour":8,"minute":47}}
```

### `schedule_run_now`
`IdRequest` → `ApplyStarted`. *Spawns.* Scans the schedule's scope, then applies
its steps with default selections.

### `tools_list`
No request → `ExternalTool[]`.

### `tools_refresh`
No request → `ExternalTool[]`. Detect again (after the user installs a tool or
edits `settings.tool_paths`).

### `drives_list`
No request → `Drive[]`. Hero drive bars: free now and after the plan.

### `disk_usage_tree`
`DiskUsageRequest` → `DiskNode`. Folder tree by size for the treemap and the
size-sorted tree. Without `path`: a virtual root with one child per scan root.
Children below `min_fraction` of their parent fold into `other`. Fails with
`invalid_input` before the first scan.

```json DiskUsageRequest
{"path":"/home/me/Projects","depth":2,"min_fraction":0.01}
```

### `breakdown`
`BreakdownRequest` → `Breakdown`. Size by file-type group, extension, age
(buckets: `<1m`, `1-6m`, `6-12m`, `1-2y`, `2-5y`, `>5y` of the modification
time) or owner.

```json BreakdownRequest
{"by":"type"}
```

## 4. Events

Subscribe with `api.on(name, cb)`; `cb` receives the payload.

### `scan://progress`
`ScanProgressEvent`. At most every 50 ms.

### `scan://detector`
`DetectorTotalsEvent`. One per opportunity while the scan's analysis phase runs,
so cards fill in live (`finished: true` when final).

### `scan://done`
`ScanStatus` with `state` `done`, `cancelled` or `failed` (`error`).

### `opportunities://changed`
`OpportunitiesChanged`. Refetch `opportunities_list` (and `drives_list`).

### `plan://changed`
`Plan`. The new plan.

### `plan://dry_run`
`DryRunReport`.

### `job://progress`
`JobProgressEvent`. At most every 50 ms per job.

### `job://state`
`Job`. State changes (queued → running → paused → ... ).

### `job://done`
`JobDoneEvent`. Terminal state; `report_available` tells whether `report_get`
works for `job.run_id`.

### `history://changed`
`HistoryChanged`. Refetch History, "Undo last action" availability and "Last report".

## 5. Types

Every example below is checked against the Rust types. Field order is free.

#### AppInfo

```json AppInfo
{"name":"MyJanitor","version":"0.1.0","platform":"windows","bundle_id":"com.myjanitor.app","portable":false,"data_dir":"C:\\Users\\Michel\\AppData\\Roaming\\MyJanitor","cache_dir":"C:\\Users\\Michel\\AppData\\Local\\MyJanitor\\cache","elevated":false,"journal_persistent":true}
```

#### ScanScope

`options` and its fields are optional (defaults shown). Includes and excludes
are glob patterns (path mode: `*` stops at `/`, `**` crosses it) or absolute
paths; global exclusions from settings are added by the engine.

```json ScanScope
{"roots":["/home/me"],"include":[],"exclude":["/home/me/VMs"],"options":{"include_hidden":true,"use_fast_index":true,"use_cache":true,"one_file_system":true,"max_depth":null,"threads":0,"collect_owner":false}}
```

#### FsListDirResponse

```json FsListDirResponse
{"path":"/home/me","parent":"/home","entries":[{"name":"Downloads","path":"/home/me/Downloads","is_dir":true,"is_link":false,"hidden":false}],"places":[{"label":"Home","path":"/home/me","kind":"home"},{"label":"Computer","path":"/","kind":"drive"}]}
```

#### ScanStatus

```json ScanStatus
{"state":"done","generation":3,"started":1794582100000000000,"finished":1794582160000000000,"progress":{"generation":3,"phase":"detecting","files":184233,"dirs":20117,"bytes":412000000000,"done":42,"total":42,"elapsed_ms":60012},"stats":{"files":184233,"dirs":20117,"links":12,"bytes":412000000000,"allocated":409000000000,"online_only_files":310,"online_only_bytes":5200000000,"unreadable":3,"unreadable_sample":["/home/me/.local/share/containers"],"permission_denied":3,"excluded":2,"cache_hits":180000,"cache_misses":4233,"elapsed_ms":60012,"used_fast_index":false},"cached":true}
```

#### ScanProgressEvent

```json ScanProgressEvent
{"generation":3,"phase":"walking","files":51200,"dirs":4410,"bytes":98000000000,"done":55610,"current":"/home/me/Projects/app/node_modules","elapsed_ms":4200}
```

#### DetectorTotalsEvent

```json DetectorTotalsEvent
{"generation":3,"opportunity_id":"det:dev_artefacts","title":"Developer artefacts","saving":10307921510,"count":19,"finished":true}
```

#### Opportunity

`size_before`, `saving`, `count` and `selected` follow the current selection.
`rank` orders "Biggest saving first". `availability` disabled = show greyed
with `reason`. `params` holds the current values of `settings` (keys missing
from `params` take the spec default).

```json Opportunity
{"id":"opt:audio","source":{"optimiser":"audio"},"category":"optimise","title":"Meeting recordings","description":"Speech preset, silences trimmed","safety":"lossy_opt_in","size_before":5153960755,"saving":4509715660,"confidence":"estimated","count":23,"unit":"files","candidates":25,"selected":23,"badges":["duplicate","confidential","synced"],"default_action":{"kind":"optimise","optimiser":"audio","params":{},"output":{"mode":"replace"},"originals":{"policy":"recycle_bin"},"keep_if_not_smaller":true},"available_actions":["optimise"],"presets":[{"id":"speech","label":"Speech","description":"Opus 24 kbps mono · smallest","values":{"codec":"opus","bitrate_kbps":24,"downmix_mono":true},"lossy":true},{"id":"lossless","label":"Lossless","description":"FLAC · no quality loss, ~45 % smaller","values":{"codec":"flac"},"lossy":false}],"settings":[{"key":"downmix_mono","label":"Downmix to mono","kind":{"type":"toggle","default":true}},{"key":"remove_silences","label":"Remove long silences","kind":{"type":"toggle","default":true},"lossy":true},{"key":"max_pause_s","label":"Max pause","kind":{"type":"number","default":2,"min":1,"max":30,"step":1,"unit":"s max"}},{"key":"transcribe","label":"Transcribe on this PC, then archive audio","help":"Keeps a searchable text file next to each meeting","kind":{"type":"toggle","default":false},"advanced":true,"disabled_reason":"Requires a local transcription tool"}],"params":{"downmix_mono":true,"remove_silences":true,"max_pause_s":2},"sub_groups":[{"key":"same_meeting","label":"Same meeting twice","count":6},{"key":"needs_transcript","label":"Need transcript","count":4},{"key":"skipped","label":"Skipped","count":2}],"availability":{"state":"available"},"in_plan":true,"rank":4509715660.0,"feature_ids":["OPT-AUD-01","OPT-AUD-04","OPT-AUD-05","OPT-AUD-06"]}
```

#### OpportunitiesList

```json OpportunitiesList
{"generation":3,"items":[],"summary":{"total":52183474995,"reversible":44238230323,"opt_in":5476083302,"review_only":2469606195,"opportunities":18},"counts":{"all":18,"clean":11,"optimise":7,"archive":1,"review_only":3},"scanned_at":1794582160000000000,"cached":true}
```

#### StepSummary

```json StepSummary
{"candidates":25,"selected":23,"size_before":5153960755,"size_after":644245094,"saving":4509715661,"confidence":"estimated"}
```

#### StepDetail

```json StepDetail
{"opportunity":{"id":"det:temp_files","source":{"detector":"temp_files"},"category":"clean","title":"Temporary files","description":"Leftover temporary and partial downloads","safety":"reversible","size_before":1200,"saving":1200,"confidence":"exact","count":2,"unit":"files","candidates":2,"selected":2,"badges":[],"default_action":{"kind":"trash"},"available_actions":["trash","quarantine","permanent_delete"],"presets":[],"settings":[],"params":{},"availability":{"state":"available"},"in_plan":false,"rank":1200.0,"feature_ids":["CLN-DET-10"]},"summary":{"candidates":2,"selected":2,"size_before":1200,"size_after":0,"saving":1200,"confidence":"exact"},"diagnostics":{"unreadable":1,"unreadable_sample":["/home/me/.cache/locked.tmp"],"skipped":{},"notes":[]},"lossy":false,"safety":"reversible"}
```

#### Candidate

`size` is what the file occupies now (allocated size when known; 0 for
online-only files), `size_after` the estimate after the default action (0 for
removals). `stamp` is the scan-time `{size, mtime}` the action engine
re-checks. `columns` carries per-source table cells (B2: Source, Length).

```json Candidate
{"id":1041,"path":"C:\\Users\\Michel\\Recordings\\Steering committee 14 Oct.wav","target":"file","source":{"optimiser":"audio"},"size":1181116006,"size_after":54525952,"confidence":"estimated","reason":"Speech recording, 14 min of silence","badges":[],"default_selected":true,"sub_group":"needs_transcript","modified":1792000000000000000,"stamp":{"size":1181116006,"mtime":1792000000000000000},"columns":{"source":"Local","length":"1:47:30"}}
```

#### CandidateRow

A `Candidate` plus the effective selection.

```json CandidateRow
{"id":1047,"path":"C:\\Users\\Michel\\Recordings\\Board session – privileged.m4a","target":"file","source":{"optimiser":"audio"},"size":220200960,"size_after":220200960,"confidence":"estimated","reason":"Marked confidential","badges":["confidential"],"default_selected":false,"modified":1792000000000000000,"stamp":{"size":220200960,"mtime":1792000000000000000},"selected":false,"actionable":true}
```

#### CandidatesPage

```json CandidatesPage
{"opportunity_id":"det:exact_duplicates","total":214,"offset":0,"rows":[{"id":7,"path":"/home/me/Photos/IMG_0001.JPG","target":"file","source":{"detector":"exact_duplicates"},"size":4194304,"size_after":0,"confidence":"exact","reason":"Kept: newest copy","badges":["duplicate","keeper"],"default_selected":false,"group":3,"modified":1700000000000000000,"stamp":{"size":4194304,"mtime":1700000000000000000},"selected":false,"actionable":true},{"id":8,"path":"/home/me/Downloads/IMG_0001.JPG","target":"file","source":{"detector":"exact_duplicates"},"size":4194304,"size_after":0,"confidence":"exact","reason":"Same content as 1 other file","badges":["duplicate"],"default_selected":true,"group":3,"modified":1690000000000000000,"stamp":{"size":4194304,"mtime":1690000000000000000},"selected":true,"actionable":true}],"groups":[{"id":3,"kind":"exact","label":"2 copies of IMG_0001.JPG","confidence":100.0,"keeper":"/home/me/Photos/IMG_0001.JPG","members":["/home/me/Photos/IMG_0001.JPG","/home/me/Downloads/IMG_0001.JPG"]}]}
```

#### StepParamsResponse

```json StepParamsResponse
{"params":{"codec":"opus","bitrate_kbps":24,"downmix_mono":true,"remove_silences":true,"max_pause_s":2},"summary":{"candidates":25,"selected":23,"size_before":5153960755,"size_after":644245094,"saving":4509715661,"confidence":"estimated"},"lossy":true,"safety":"lossy_opt_in"}
```

#### Preview

```json Preview
{"kind":"waveform","waveform":{"peaks":[12,40,200,180,3,2,2,90],"duration_ms":6450000,"silences":[[1200000,2040000]]}}
```

#### SearchParseResponse

```json SearchParseResponse
{"rule":{"all":[{"cond":{"field":"extension","values":["mp4"]}},{"cond":{"field":"size","cmp":"gt","bytes":1073741824}},{"cond":{"field":"date","which":"modified","cmp":"lt","value":{"at":1704067200000000000}}}]},"errors":[]}
```

#### Suggestion

```json Suggestion
{"insert":"size:>","replace_len":2,"label":"size:>1GB  bigger than 1 GB"}
```

#### Rule

Nodes: `all`, `any`, `not`, `cond`. Conditions (`field`): `name`, `path`
(`mode`: `wildcard`/`contains`/`exact`/`regex`), `extension`, `type_group`,
`size`, `zero_byte`, `largest_n`, `date` (`which`: `created`/`modified`/`accessed`;
`value`: `{"at": ts}` or `{"ago": {"amount", "unit"}}`), `attribute`, `owner`,
`depth`, `path_length`, `invalid_name`, `kind`, `extension_mismatch`,
`content_contains`. `cmp`: `lt`, `le`, `eq`, `ge`, `gt`.

```json Rule
{"any":[{"cond":{"field":"name","mode":"wildcard","pattern":"~$*","case_sensitive":false}},{"not":{"cond":{"field":"attribute","attr":["hidden"],"set":true}}},{"cond":{"field":"largest_n","n":50}},{"cond":{"field":"zero_byte"}}]}
```

#### RulePreset

```json RulePreset
{"id":"downloads_older_90d","name":"Downloads older than 90 days","description":"Anything in a Downloads folder not modified for 90 days","rule":{"all":[{"cond":{"field":"path","mode":"wildcard","pattern":"**/Downloads/**","case_sensitive":false}},{"cond":{"field":"date","which":"modified","cmp":"lt","value":{"ago":{"amount":90,"unit":"days"}}}}]},"topmost_only":false,"builtin":true}
```

#### Action

`kind`: `trash`, `quarantine{purge_after_days}`, `permanent_delete`,
`move{dest, keep_structure}`, `copy{dest, keep_structure}`, `hardlink`,
`reflink`, `dehydrate`, `optimise{optimiser, params, output, originals,
keep_if_not_smaller}`, `archive{format, level, dest?, originals, keep_sources}`,
`fs_compress{algorithm}`, `fs_decompress`, `none`. `OriginalsPolicy`:
`recycle_bin` (default), `quarantine`, `backup_folder{folder}`, `delete`.
`OutputMode`: `replace` (default), `light_copy{suffix}`.

```json Action
{"kind":"archive","format":"zip","level":9,"originals":{"policy":"backup_folder","folder":"D:\\Archive\\originals"},"keep_sources":false}
```

#### Plan

```json Plan
{"id":5,"name":"Your plan","steps":[{"id":12,"opportunity":"det:exact_duplicates","title":"Exact duplicates","summary":"Keep newest · others to Recycle Bin","action":{"kind":"trash"},"params":{"keep_rule":"newest"},"selection":{"include":[],"exclude":[8]},"estimate":{"files":153,"bytes_before":9234179686,"bytes_after":0,"confidence":"exact","skipped":0},"safety":"reversible","lossy":false},{"id":15,"opportunity":"opt:audio","title":"Meeting recordings","summary":"Opus 24 kbps mono · originals to Recycle Bin","action":{"kind":"optimise","optimiser":"audio","params":{"codec":"opus","bitrate_kbps":24},"output":{"mode":"replace"},"originals":{"policy":"recycle_bin"},"keep_if_not_smaller":true},"params":{"codec":"opus","bitrate_kbps":24},"estimate":{"files":23,"bytes_before":5153960755,"bytes_after":644245094,"confidence":"estimated","skipped":0},"safety":"lossy_opt_in","lossy":true}],"estimate":{"files":176,"bytes_before":14388140441,"bytes_after":644245094,"confidence":"estimated","skipped":0},"checklist":[{"level":"ok","code":"undo_window","text":"Every reversible step can be undone from History for 30 days"},{"level":"warning","code":"confirmation_steps","text":"1 step will ask for confirmation"},{"level":"ok","code":"skipped_confidential","text":"14 signed or sensitivity-labelled files are skipped"},{"level":"info","code":"synced_files","text":"Synced OneDrive files are made online-only, never deleted in the cloud"}],"updated":1794582245000000000}
```

#### PlanStep

```json PlanStep
{"id":13,"opportunity":"det:dev_artefacts","title":"Developer artefacts","summary":"To Recycle Bin · rebuild with npm / pip","action":{"kind":"trash"},"params":{},"estimate":{"files":19,"bytes_before":10307921510,"bytes_after":0,"confidence":"exact","skipped":0},"safety":"rebuildable","lossy":false}
```

#### Job

```json Job
{"id":31,"kind":"apply","run_id":1794582245123,"step_id":12,"title":"Exact duplicates","state":"completed_with_errors","progress":{"done":153,"total":153,"bytes_done":9234179686,"bytes_total":9234179686,"saved":9230000000},"created":1794582245000000000,"started":1794582245100000000,"finished":1794582290000000000,"outcome":{"succeeded":151,"skipped":1,"failed":1,"bytes_saved":9230000000,"errors":[{"path":"/home/me/Downloads/locked.iso","status":"failed","bytes_before":4179686,"bytes_after":4179686,"error":{"code":"permission_denied","message":"Permission denied: /home/me/Downloads/locked.iso","hint":"MyJanitor has no access to this path. Skip it, or run with access to it.","path":"/home/me/Downloads/locked.iso"}}]}}
```

#### JobProgressEvent

```json JobProgressEvent
{"job_id":31,"run_id":1794582245123,"progress":{"done":40,"total":153,"bytes_done":2400000000,"bytes_total":9234179686,"saved":2400000000},"current":"/home/me/Downloads/IMG_0001.JPG"}
```

#### JobDoneEvent

```json JobDoneEvent
{"job":{"id":32,"kind":"dry_run","title":"Dry run","state":"completed","progress":{"done":4,"total":4,"bytes_done":0,"bytes_total":0,"saved":0},"created":1794582245000000000,"started":1794582245000000000,"finished":1794582250000000000},"report_available":false}
```

#### DryRunReport

```json DryRunReport
{"plan_id":5,"job_id":32,"steps":[{"step_id":12,"title":"Exact duplicates","estimate":{"files":152,"bytes_before":9230000000,"bytes_after":0,"confidence":"exact","skipped":1},"skipped":[{"path":"/home/me/Downloads/changed.jpg","status":"skipped","bytes_before":4194304,"bytes_after":4194304,"error":{"code":"changed","message":"Changed since the scan: /home/me/Downloads/changed.jpg","hint":"The file changed after the scan, so it was skipped. Scan again."}}],"skipped_count":1,"warnings":[]}],"total":{"files":152,"bytes_before":9230000000,"bytes_after":0,"confidence":"exact","skipped":1}}
```

#### ConfirmationInfo

```json ConfirmationInfo
{"step_id":15,"title":"Compress 23 recordings?","body":"This is lossy. The new files sound fine for speech but cannot be turned back into the originals.","files":23,"bytes_now":5153960755,"bytes_after":644245094,"saving":4509715661,"confidence":"estimated","originals_options":[{"policy":{"policy":"recycle_bin"},"label":"Move to Recycle Bin (default)","reversible":true},{"policy":{"policy":"quarantine"},"label":"Keep in quarantine for 30 days","reversible":true},{"policy":{"policy":"delete"},"label":"Delete permanently","reversible":false}],"default_originals":{"policy":"recycle_bin"},"synced":{"count":6,"provider":"OneDrive – Acme","can_online_only":true},"acknowledge_text":"I understand the new files are lower quality than the originals","undo_text":"Every change is logged; History → Restore brings the originals back for 30 days.","action_label":"Compress 23 files","permanent":false}
```

#### StepConfirmation

```json StepConfirmation
{"step_id":15,"acknowledged":true,"originals":{"policy":"quarantine"}}
```

#### ApplyStarted

```json ApplyStarted
{"run_id":1794582245123,"jobs":[{"id":31,"kind":"apply","run_id":1794582245123,"step_id":12,"title":"Exact duplicates","state":"queued","progress":{"done":0,"total":153,"bytes_done":0,"bytes_total":9234179686,"saved":0},"created":1794582245000000000}]}
```

#### HistoryList

```json HistoryList
{"total":1,"runs":[{"run_id":1794582245123,"kind":"apply","title":"Plan · 4 steps","started":1794582245000000000,"finished":1794582400000000000,"steps":["Exact duplicates","Developer artefacts","App caches & junk","Meeting recordings"],"items":239,"done":236,"failed":1,"undone":0,"bytes_saved":31460000000,"undoable":true,"undo_until":1797174245000000000}]}
```

#### JournalEntry

`state`: `intent`, `done`, `failed`, `skipped`, `undone`, `undo_failed`, `purged`.
`retained_at` is where the original now lives (quarantine, move destination,
backup); `trash` locates a Recycle Bin / Trash item; `created` lists files the
action wrote (undo removes them).

```json JournalEntry
{"id":1794582245123000001,"run_id":1794582245123,"job_id":31,"step_id":12,"at":1794582246000000000,"state":"done","action":"trash","target":"file","original":"/home/me/Downloads/IMG_0001.JPG","original_stamp":{"size":4194304,"mtime":1690000000000000000},"original_sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","bytes_before":4194304,"bytes_after":0,"trash":{"location":"/home/me/.local/share/Trash/files/IMG_0001.JPG","info":"/home/me/.local/share/Trash/info/IMG_0001.JPG.trashinfo","trashed_at":1794582246000000000},"reversible":true,"undo_deadline":1797174246000000000}
```

#### HistoryRunDetail

```json HistoryRunDetail
{"run":{"run_id":1794582245123,"kind":"apply","title":"Plan · 1 step","started":1794582245000000000,"steps":["Temporary files"],"items":1,"done":1,"failed":0,"undone":0,"bytes_saved":1200,"undoable":true},"total":1,"entries":[{"id":1794582245123000001,"run_id":1794582245123,"job_id":31,"at":1794582246000000000,"state":"done","action":"quarantine","target":"file","original":"/home/me/a.tmp","original_stamp":{"size":1200,"mtime":1690000000000000000},"bytes_before":1200,"bytes_after":0,"retained_at":"/home/me/.local/share/myjanitor/quarantine/1794582245123/1794582245123000001/a.tmp","reversible":true}]}
```

#### QuarantineInfo

```json QuarantineInfo
{"dir":"/home/me/.local/share/myjanitor/quarantine","items":38,"bytes":7408715776,"oldest":1792000000000000000,"purge_after_days":30}
```

#### Report

```json Report
{"run_id":1794582245123,"title":"Plan · 1 step","started":1794582245000000000,"finished":1794582250000000000,"totals":{"files":2,"succeeded":1,"skipped":1,"failed":0,"bytes_before":2400,"bytes_after":1200,"bytes_saved":1200},"sections":[{"title":"Temporary files","action":"To Recycle Bin","totals":{"files":2,"succeeded":1,"skipped":1,"failed":0,"bytes_before":2400,"bytes_after":1200,"bytes_saved":1200},"items":[{"path":"/home/me/a.tmp","status":"done","bytes_before":1200,"bytes_after":0},{"path":"/home/me/b.tmp","status":"skipped","bytes_before":1200,"bytes_after":1200,"error":{"code":"protected","message":"Protected path: /home/me/b.tmp","hint":"Protected paths are never touched. Edit them in Settings."}}]}],"undo_until":1797174250000000000}
```

#### ExportResult

```json ExportResult
{"path":"/home/me/Documents/myjanitor-audit (1).csv","bytes":48213,"rows":239}
```

#### Settings

Every field is optional on input (defaults shown here); unknown fields are
ignored. `type_groups` are the editable file-type groups (CLN-FLT-02).

```json Settings
{"version":1,"general":{"undo_days":30,"quarantine_days":30,"default_originals":{"policy":"recycle_bin"},"light_copy_suffix":"_light"},"scan":{"include_hidden":true,"use_fast_index":true,"use_cache":true,"one_file_system":true,"max_depth":null,"threads":0,"collect_owner":false},"safety":{"protected_paths":[],"exclusions":[],"skip_signed":true,"skip_labelled":true,"skip_encrypted":true,"synced_folder_warning":true},"optimise":{"lossless_only":true,"keep_if_not_smaller":true,"preserve_timestamps":true,"preserve_permissions":true,"preserve_ads":true},"performance":{"threads":0,"parallel_jobs":0,"low_memory":false,"cpu_priority":"normal"},"tool_paths":{},"integration":{"context_menu":false,"fs_compression_watcher":false,"watched_folders":[]},"type_groups":{"installers":["msi","exe","iso"]}}
```

```json SettingsSetRequest
{"settings":{"general":{"undo_days":14}}}
```

#### ProtectedPathsInfo

```json ProtectedPathsInfo
{"builtin":["/bin","/boot","/etc","/usr"],"builtin_patterns":["**/.git","**/.ssh/**"],"user":["/home/me/Clients/Archive"]}
```

#### CacheInfo

```json CacheInfo
{"entries":184233,"bytes":19300000,"path":"/home/me/.cache/myjanitor/scan-cache.bin"}
```

#### Schedule

```json Schedule
{"id":"sched_9","name":"Weekly cleanup","enabled":true,"cadence":{"every":"weekly","weekday":1,"hour":8,"minute":47},"scope":{"roots":["/home/me/Downloads"]},"steps":[{"opportunity":"det:temp_files","action":{"kind":"trash"},"params":{}},{"opportunity":"det:old_installers","action":{"kind":"quarantine","purge_after_days":30},"params":{"min_age_days":180}}],"last_run":null,"next_run":1794900000000000000}
```

```json ScheduleSaveRequest
{"schedule":{"id":"","name":"Monthly archive","enabled":true,"cadence":{"every":"monthly","day":1,"hour":21,"minute":13},"scope":{"roots":["D:\\Projects"]},"steps":[]}}
```

#### ExternalTool

```json ExternalTool
{"id":"ffmpeg","found":true,"path":"/usr/bin/ffmpeg","version":"6.1.1","used_for":["Video","Screen recordings","Meeting recordings"]}
```

#### Drive

A `VolumeInfo` (flattened) plus the plan's effect.

```json Drive
{"id":"C:","mount":"C:\\","label":"System","fs_type":"NTFS","total":511101108224,"free":48318382080,"available":48318382080,"removable":false,"features":{"hardlinks":true,"reflinks":false,"wof":true,"btrfs_compression":false,"trash":true,"mft":false},"plan_saving":44023414784,"free_after_plan":92341796864}
```

#### DiskNode

```json DiskNode
{"name":"Projects","path":"/home/me/Projects","is_dir":true,"size":120000000000,"allocated":118000000000,"files":250000,"children":[{"name":"app","path":"/home/me/Projects/app","is_dir":true,"size":80000000000,"allocated":79000000000,"files":200000,"children":[]}],"other":{"size":900000000,"count":41}}
```

#### Breakdown

```json Breakdown
{"by":"age","rows":[{"key":">5y","label":"Older than 5 years","bytes":90000000000,"files":40000},{"key":"<1m","label":"Last month","bytes":2000000000,"files":3000}]}
```

#### OpportunitiesChanged, HistoryChanged

```json OpportunitiesChanged
{"generation":3}
```

```json HistoryChanged
{"run_id":1794582245123}
```

## 6. Quick-query syntax (CLN-ENG-05)

Parsed by `mj_filters::query` into a `Rule` (owner W2-C).

```text
query   := or
or      := and ("OR" and)*
and     := unary (["AND"] unary)*          juxtaposition = AND
unary   := "-" unary | "NOT" unary | "(" or ")" | term
term    := key ":" value | bare
value   := [op] text | text "," text ...   op: < <= > >= =   (list = any of)
text    := word | "quoted text"
bare    := text                             name contains, case-insensitive
```

| Key | Value | Condition |
|---|---|---|
| `ext` | `mp4` or `jpg,png` | `extension` |
| `name` / `path` | wildcard (`*`, `?`, `**`), `/regex/` | `name` / `path` |
| `size` | `>1GB`, `<=500k`, `=0`, `1MB..10MB` | `size` (`=0` → `zero_byte`) |
| `modified` / `created` / `accessed` | `<2024`, `>=2024-03`, `2024-03-15`, `>2y` | `date`: absolute dates compare with the date range (`<2024` = before 2024-01-01, `2024` = within 2024); **relative ages** compare age, so `modified:>2y` = older than 2 years |
| `type` | `video`, `installers`, ... (`Settings.type_groups`) | `type_group` |
| `attr` | `hidden`, `system`, `readonly`, `compressed`, `sparse`, `encrypted`, `offline` | `attribute` |
| `owner` | user name or `uid:1000` | `owner` |
| `depth` / `pathlen` | `>5` | `depth` / `path_length` |
| `kind` | `file`, `dir`, `link` | `kind` |
| `content` | text | `content_contains` |
| `is` | `empty`, `hidden`, `invalid`, `mismatch` | `zero_byte`, `attribute`, `invalid_name`, `extension_mismatch` |
| `top` | `50` | `largest_n` |

Errors carry byte `offset` and `len` in the query for underlining.

## 7. Mock backend rules

`ui/js/mock/backend.js` must return exactly these shapes (examples above are
good fixtures), reject unknown commands with `invalid_input`, reject
unfinished ones with `not_implemented`, and emit events through the same
names. UI tests run against it with `node --test ui/`.
