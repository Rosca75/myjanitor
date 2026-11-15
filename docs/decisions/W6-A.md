# W6-A: the operating system's file dialogs replace the in-app folder browser

Owner requirement: "The file explorer should be the std one from windows (not a custom one
made for the app, so this should be dropped)". Decisions taken without the owner (nobody
could be asked), by what a file cleaner that people trust with their real files needs,
Windows first.

## D-6A1 Native dialogs through two shell commands (supersedes D-104)

- **Context:** D-104 chose an in-app folder browser (`fs_list_dir` plus a browser dialog in
  the UI) to avoid a dialog plugin. The owner wants the standard OS dialog.
- **Choice:** every place that opened the in-app browser now opens the OS dialog: a folder
  dialog for "+ Add location", move/copy destinations, the preferred folder of a keep rule
  and the folder settings (protected paths, backup folder for originals, temporary folder);
  a "Save as" dialog with a suggested file name and a one-entry file-type filter for the
  exports (report, audit, results). Two commands, `dialog_pick_folder` and
  `dialog_save_file`, return the chosen path (lossless `serde_path` codec) or `null` on
  cancel. They are *shell commands*: implemented in `src-tauri/src/dialogs.rs` because they
  need the window, listed in `mj_engine::api::SHELL_COMMANDS` (request and response type
  names), documented in `docs/IPC.md` §3.1 after the engine commands and in `ui/js/api.js`
  after them, and checked by the same tests as the engine commands
  (`every_command_is_unique_and_documented`, `ipc_examples_parse`, the new
  `shell_commands_name_documented_types`, `ui/js/api.test.js`, and the shell's
  `every_command_is_registered` / `the_handler_list_matches_the_registered_names`). The
  CLI has no dialogs and does not get them.
- **Dropped completely:** `fs_list_dir` and its types (`FsListDirRequest`,
  `FsListDirResponse`, `FsDirEntry`, `FsPlace`), `crates/engine/src/fsbrowse.rs`, the Tauri
  wrapper, `ui/js/views/shell/browser.js`, `ui/js/views/settings/picker.js`, the `browser`
  state slice and its actions (`openBrowser`, `browseTo`, `closeBrowser`, `chooseFolder`,
  `setExportName`, `finishExport`), the settings picker state and actions, `joinPath`, the
  browser CSS, copy strings, mock handler, fixtures (`FS`, `testing/real/fs_list_dir.json`)
  and tests. Nothing else used `fs_list_dir` (checked: the CLI never exposed it; the
  engine, shell, UI and mock users were all browser code).
- **Why:** the requirement, and the native dialog is what Windows users know (recent places,
  Quick access, network locations, libraries, search, "new folder", typing a path),
  which the in-app browser never matched.

## D-6A2 `rfd` directly, not `tauri-plugin-dialog`

- **Context:** the brief recommended `tauri-plugin-dialog` called from Rust, and to deviate
  only with a written rationale. Read in `~/.cargo/registry`: `tauri-plugin-dialog` 2.8.4
  (the release for tauri 2.12) implements its desktop file dialogs in `src/desktop.rs` as
  `run_on_main_thread` + `rfd::AsyncFileDialog` + a spawned thread that `block_on`s the
  future, and depends on `rfd` 0.17, `tauri-plugin-fs` 2.7.1 (for its `FilePath` type and
  scope) and `tauri-plugin` (build).
- **Choice:** depend on `rfd` `=0.17.2` (default features off, `gtk3` only) and do the same
  thing in our own two commands.
- **Why:** the plugin adds nothing we use over `rfd`: its value is its webview commands
  (`plugin:dialog|open`, `save`, `message`, `ask`, `confirm`) and their permission
  schemas, which we deliberately do not want reachable. Registering the plugin would put
  those handlers in the app (denied today by `core:default`, but one capability edit away)
  and bring `tauri-plugin-fs` code into the binary. Measured with `cargo tree --offline`:
  the plugin would add 3 crates more (`tauri-plugin-dialog`, `tauri-plugin-fs`,
  `tauri-plugin`); `rfd` alone added exactly one package to `Cargo.lock` (all its Linux and
  Windows dependencies were already there through Tauri). The weight difference is small;
  the surface and control are the reasons. Our command awaits the dialog future on the
  async runtime instead of parking a thread in `block_on`. Same `rfd` line as the plugin, so
  switching later is a local change in `dialogs.rs`.
- **Features:** `xdg-portal` and `wayland` (rfd defaults) are off: they would pull
  `ashpd`/`zbus` and an async D-Bus stack. `gtk3` uses `GtkFileChooserNative`, which itself
  goes through the desktop portal when the app runs sandboxed (Flatpak) or with
  `GTK_USE_PORTAL=1`. `common-controls-v6` (enabled by the plugin) only affects
  `rfd`'s message boxes, which we do not use.

## D-6A3 Security posture

- Capabilities stay `core:default` (`src-tauri/capabilities/default.json` unchanged): the
  webview gets no `dialog:*` and no `fs:*` permission and no filesystem scope. It can only
  ask our command for "a folder" or "a file name to save to" and receives a path string.
- A dialog chooses a path and nothing else. What is then done with it goes through the
  engine command that always handled it, with its own checks (`scope_add_root` refuses
  what is not an existing folder; move/copy destinations that are relative, protected or
  inside a scanned location are refused; exports into protected locations are refused).
- Request values are cleaned before they reach the OS dialog: titles and filter labels lose
  control characters and are capped (a NUL would make `rfd`'s GTK back end panic in
  `CString::new(..).unwrap()`); a relative start folder is dropped (it would resolve
  against the process's working directory); a suggested file name must be one portable
  name (no separator, no control character, nothing Windows refuses) or it is dropped, not
  repaired; extensions must be 1 to 16 ASCII letters or digits, so nothing can alter the
  `*.ext` pattern the Windows dialog builds. Requests reject unknown fields like every
  other request.
- At most one dialog is open at a time (`DialogGate`; a second request fails with
  `conflict`). On Windows the dialog disables its owner window anyway; `rfd` 0.17's GTK
  back end creates `GtkFileChooserNative` without a transient parent (verified in
  `backend/gtk3/file_dialog/dialog_ffi.rs`), so on Linux the window stays clickable while
  the dialog is open and the gate prevents a second dialog.

## D-6A4 An export chosen in the save dialog replaces the file the dialog asked about

- **Context:** exports never overwrote (`name (1).csv`). The OS save dialog asks "replace
  it?" itself (Windows `FOS_OVERWRITEPROMPT` is on by default for `IFileSaveDialog`; `rfd`
  sets `gtk_file_chooser_set_do_overwrite_confirmation`). Writing `name (1).csv` after the
  user said "Replace" would contradict the answer they just gave.
- **Choice:** `report_export` and `results_export` take `overwrite` (default `false`). The GUI
  sends `true` only with a path from `dialog_save_file`; the CLI and every other caller keep
  never overwriting. With `overwrite`, `mj_actions::report::write_export_replacing` writes a
  temporary sibling (`.name.<pid>-<n>.mjtmp`, created with `create_new`), syncs it and
  renames it over the target; a failed export leaves the old file untouched and no
  temporary file. Only a regular file is replaced (checked with `symlink_metadata`): a
  folder or a link at the target gets a free name instead, so the file behind a link is
  never written. `write_export` now also removes its half-written file when the writer
  fails.
- **Note for the merge:** this touches `crates/actions/src/report.rs` (lane-b's crate). The
  change is additive (one new public function, the shared create/fill steps factored out)
  and does not touch the savings code.

## D-6A5 Threading

The command takes the gate, then asks the main thread (`run_on_main_thread`) to build the
dialog with the owner window (`set_parent(&window)`) and start it; that closure returns at
once and hands the dialog's future back over a channel; the async command awaits it. On
Windows `rfd` runs the dialog on its own COM (STA) thread, modal to the main window; on
Linux the GTK main loop drives it. Neither the main thread nor the async runtime is ever
blocked, and no `blocking_*` call is used.

## D-6A6 Mock backend stand-in

`ui/js/mock/dialogs.js` answers both commands at once and deterministically: by default as
if the user accepted what the dialog offered (`start_dir`, else the mock platform's
Documents folder, plus the suggested file name for a save); a fixed path or `null`
(cancel) when configured through `createMockBackend({ dialog })`,
`backend.answerDialogs(...)` or the page URL (`?dialog=cancel`, `?dialog=<folder>`).
Requests are validated like the shell does (unknown fields rejected), so a wrong field name
fails a UI test instead of being ignored.

## D-6A7 UI behaviour

- A cancelled dialog changes nothing, sends no further command and shows nothing (tests
  for add location, move/copy, keep rule, every folder setting and exports).
- A failing dialog (for example `conflict`) shows like any failing command: a toast in the
  main window, the inline error under the control in the Settings drawer.
- The dialog opens in the value in force when there is one (current move/copy destination,
  preferred folder, temporary folder, backup folder), else where the OS chooses (usually the
  last folder used).
- "Preferred folder..." in the keep-rule list: on cancel the list goes back to the rule in
  force. The `select` component now sets its `value` to the option in force explicitly, so
  a redraw after a choice the controller did not take shows the right option.
- Each dialog has a title saying what the folder or file is for.

## D-6A8 Folder drag-and-drop

The brief asked that folder drag-and-drop onto the window keep working. Nothing in this
change touches it (`dragDropEnabled: true` in `tauri.conf.json` and the `core:default`
capability, which allows listening to `tauri://drag-drop`, are unchanged). Checked at the
baseline (66375fb): the UI has no drop handler, so dropping a folder on the window does
nothing today, before and after this change. Adding one (drop a folder to add it to the
scope) is a feature of its own and is left as an open item rather than slipped in here.

# W6-A (second task): duplicate review with thumbnails and groups kept together

Owner requirements: (a) "For duplicates, thumbnails should be displayed to the user to help
him validating which one to remove"; (b) "For duplicates, when filtering on size, you should
be able to keep the notion of duplicate groups (filter on the biggest file of the group). We
need to be able to recover the group view somehow." Decided without the owner, by what a
file cleaner people trust with their real files needs, Windows first.

## D-6A9 Groups are kept together by the engine, not by the UI

- **Choice:** `candidates_page` takes `keep_groups` (and a new `min_size`). The order, the
  filters and the paging are computed in `crates/engine/src/review_order.rs` over all
  candidates, not in the UI over one page: a UI-side regrouping of 50 rows would show
  groups cut by the page edge and a size order that is wrong across pages.
- **Mode:** on by default in the UI for every step with groups (the response's `has_groups`),
  shown as a visible "Keep groups together" checkbox next to the filters. Off gives the flat
  list exactly as before; on again restores the group view. The flag lives in the step state:
  it survives paging, sorting, filtering and selection changes and resets when another step
  is opened (`freshStep`). The engine default stays `false` so the CLI and older callers see
  no change.
- **Rejected:** a separate "group view" tab (two lists to keep in sync, and the owner asked to
  keep the groups *while* filtering on size, not to switch views).

## D-6A10 Group aggregates per column

The owner fixed size; the others are chosen so the order reads the way the column does:

| Column | Group key | Why |
|---|---|---|
| Size | largest member | owner's rule; equal to the file size for exact duplicates |
| After / saving | sum of member savings | what the group can free, the reason to review it first |
| Name, Folder (path) | the group's lead row: kept copy, else first path | the first row shown under the header is the one the order is read from |
| Modified | most recently modified member | "groups touched recently" first or last |
| Group | group id | unchanged |

Ties fall back to the lead path, then the group id (a total order: pages are stable).
`descending` reverses the group order only; inside a group the kept copy comes first, then
the column's natural order (biggest first for size and saving), then the path. Rows without a
group are single-row blocks ordered by the same keys and get no header.

## D-6A11 Filters keep or drop whole groups

With groups kept together a group is shown, all of its members, when any member matches the
path filter and its largest member is at least `min_size`. "Any member" for the path filter
keeps the owner's intent (find a group by one of its copies, then see every copy before
choosing). `min_size` is a new "At least" list (Any, 1 MB, 10 MB, 100 MB, 1 GB) on every
results table; in flat mode it applies per row. The table had no size filter before, and a
list of round sizes is enough for review (a free number field invites typos in bytes).

## D-6A12 Pages hold whole groups

Pages are packed by groups with the page size (50) as a row budget: a group that does not fit
starts the next page, so a page holds at most 50 rows and never splits a group that fits on
one page; a group of more than 50 rows is split (unavoidable), and the rows after it share
its last page. The engine returns `offset` (the start of the page holding the requested
offset), `next_offset` / `prev_offset`, `page` and `pages`; the UI pages with them instead of
`offset ± 50`. `total` stays the number of rows matching. The two existing UI tests that
encoded 50-row pages on the duplicates step were changed on purpose to expect group-aligned
pages (48 rows on the first page of the mock) and to check the flat 50-row paging with the
mode off.

## D-6A13 Speed at 1M candidates

The order of one request shape (opportunity, scan generation, sub-group, filters, sort,
direction, mode, page size in grouped mode) is cached in one slot and validated by an O(n)
fingerprint of the candidates (id, size, saving, group, kept-copy flag, mtime, path length),
so a keep rule, a re-analysis or an apply never serves a stale order, and paging through a
step sorts once. Measured on 1M synthetic candidates in groups of two, debug build: grouped
order 1.9 s versus 1.3 s flat (computed once per request shape), fingerprint 40 ms per page
(`review_order::tests::million_rows_order_once`, ignored by default). The flat mode uses the
same cache, which also speeds up its paging.

## D-6A14 Thumbnails are made in Rust with the decoders the project has

- **Decoders:** JPEG through `zune-jpeg` (already a dependency of `mj-dupes`, now asked for
  RGB), PNG through the in-house decoder of `mj-dupes`, refactored to walk rows for both the
  fingerprint and the thumbnail (`decode/thumb.rs`). Box downscale (alpha premultiplied),
  EXIF orientation applied (`mj_formats::exif`), encoded as PNG with the in-house filters and
  `miniz_oxide`. No image crate: the policy is in-house first, and two decoders plus a
  40-line scaler cover the files people photograph.
- **Fast path:** a row thumbnail of a JPEG first reads a 256 KiB prefix and uses the EXIF
  embedded thumbnail when it has the image's aspect ratio (within 3 %; a letterboxed one is
  refused) and is large enough; otherwise the whole file is read and decoded.
- **GIF, WebP, BMP:** not decoded in Rust. The original is sent when the file is at most
  256 KiB (row) or 1 MiB (preview pane) and its header gives a sane size; otherwise the type
  icon. The webview decodes these formats every day for web pages; a small cap keeps the
  data URL small (a 256 KiB file is about 350 KB of base64 for one row). Writing three more
  decoders for formats that are rare among duplicates is not worth it. An animated GIF
  animates in its cell; accepted.
- **Sizes:** row thumbnails are at most 128 px on the long side (shown at 48 px, 96 px on
  hover or focus: sharp on a 2x screen); the preview pane image is at most 512 px. This makes
  `candidate_preview` a real bounded thumbnail and fixes review finding IPC-9 (it used to
  send whole files up to 4 MiB).
- **Not images:** video, PDF and Office files get a type icon. Follow-up: OS shell thumbnails
  on Windows (`IShellItemImageFactory`), which would cover them without new decoders.

## D-6A15 Thumbnail limits and efficiency

- Never read an online-only file: the scan badge, then a fresh attribute check
  (`Platform::metadata`) right before reading, since a file can be dehydrated after the scan.
- Input at most 64 MiB, decoded pixels at most 48 MP (checked from the header before any
  pixel is decoded), images over 12 MP decoded one at a time (a gate), every decode under
  `catch_unwind`, every failure a reason code (`not_image`, `online_only`, `too_large`,
  `unreadable`, `undecodable`, `unknown`), never an error for the batch.
- `candidate_thumbnails` (at most 100 ids) runs on the blocking pool like every command (the
  Tauri wrapper's `spawn_blocking`): neither the UI thread nor the async runtime waits on it.
  The results lock is released before any file is read.
- One thumbnail per exact-duplicate group: the engine computes it once per batch and, when
  the asked row is online-only or unreadable, takes it from another member on this disk; the
  UI asks for one row per exact group and paints every row of the group with it. Similar-
  image groups get one thumbnail per row (their members differ, which is the point).
- Engine cache: an LRU of 32 MiB of encoded images (failures cached too) keyed by path, size,
  modification time and requested size. UI cache: 1000 entries keyed by path, size and
  modification time.
- UI loading: after the rows are drawn, batches of 10 ids, two calls in flight; a newer page
  makes older answers stale (dropped); each answer patches its own cells
  (`[data-thumb]`), never the table. Only data URLs of the five image types reach `src`.

## D-6A16 Mock parity

`ui/js/mock/review-order.js` mirrors the engine order (same aggregates, filters, packing and
group totals; a parity test runs the engine's unit-test fixture through it). The mock's
`candidate_thumbnails` answers a generated PNG for image paths and `not_image` otherwise.

# W6-A (third task): review findings on entry ids, step selection, confirmation, lossy flag

Findings IPC-1 (same defect as ACT-2 and ENG-1), ENG-2, ENG-3, ENG-6, ENG-4, ENG-7, ENG-9
and ACT-10 of the first independent review. Each was shown with a failing test first.

## D-6A17 `EntryId` crosses the wire as a decimal string (IPC-1, ACT-2, ENG-1)

- **Context:** entry ids are `run_ms * 1_000_000 + index`, about 1.8e18. The webview parses
  JSON numbers into doubles (exact only below 2^53), so up to 256 consecutive entries of a
  run collapsed onto one id: Restore on one History row restored another file (usually the
  run's first) or failed, one tick ticked a block of rows, and the occupied flag landed on
  the wrong rows. IPC.md promised ids below 2^53 while its own example was not.
- **Choice:** `mj_core::EntryId` gets hand-written serde: it writes a decimal string and
  reads a string (digits only) or a bare unsigned integer. Every carrier follows at once
  (`JournalEntry.id`, `undo_of`, `HistoryRunDetail.occupied`,
  `RestoreEntriesRequest.entry_ids`). The UI treats the id as an opaque string; the mock
  produces ids of the real magnitude (`String(BigInt(run) * 1_000_000n + index)`), and a UI
  test passes every answer through JSON text as Tauri does.
- **On disk:** new journal lines and manifests write the string; lines written before keep
  loading (the number branch). An older build cannot read the new lines; there is no
  release that would need to, so no downgrade path was kept. `crates/actions/tests/
  recovery.rs` searched the journal text for `"id":<n>`; it now looks for `"id":"<n>"`
  (deliberate format change, the assertion itself is unchanged).
- **Rejected:** shrinking the id scheme below 2^53 (a ms run id leaves about 5000 ids per
  run and needs a journal migration); BigInt in the UI (no arithmetic is ever needed).
- **Fixture:** only `ui/js/testing/real/history_run_detail.json` was regenerated; the other
  fixtures carry time-dependent values that would churn without changing what they test.

## D-6A18 A parameter change keeps the user's own ticks and unticks (ENG-2)

- **Context:** `step_params_set` re-runs the analysis and replaced the record wholesale with
  fresh defaults: rows the user had unchecked (photos that must stay untouched, a copy to
  keep) came back checked and were then applied. G-04 had fixed this for rescans only.
- **Choice:** `steps::carry_user_choices` carries the user's *explicit* choices (the
  selection overrides) to the new record by path; every other row takes the new run's
  default. This differs on purpose from the rescan carry (`results::carry_selections`, which
  carries the effective state): the files did not change, only the parameters, so a row
  the new parameters no longer pick by default must not stay ticked because the old default
  ticked it. The swap happens under the results write lock, after `recompute`, and is
  refused with `conflict` when a scan finished during the re-run (the record was built from
  the older index and must not overwrite newer results). `search_run` under an existing
  title carries the same way.

## D-6A19 "Select all" acts on the rows the table shows (ENG-3)

- **Context:** `selection_set` without ids targeted every row of the tab, while the header
  box of the table reflects the filtered page: with a text filter showing 12 rows, ticking
  the header selected all 4000 rows of the step (and unticking cleared them all).
- **Choice:** `SelectionSetRequest` takes the table's `filter`, `min_size` and
  `keep_groups` (all optional). Without ids the engine builds the same `review_order`
  query as `candidates_page` (one helper, `view_query`) and acts on exactly the rows that
  query returns, on every page: the two commands cannot disagree about what is shown.
  With groups kept together a filter shows whole groups, so "select all" covers those
  groups and never a hidden one; the kept copy is still never selected by "select all".
  The UI sends the filters only when one is active; the mock runs its `candidates_page`
  order for the same answer.
- **Rejected:** having the UI send the ids of the visible rows (only one page is loaded;
  the request would grow with the step).

## D-6A20 One rule for "is this step lossy" (ENG-6)

- **Context:** adding a step and changing its action treated a `LossyOptIn` card (audio,
  video re-encoding) as always lossy; changing its parameters looked only at the lossy
  toggles and presets. Setting the bitrate of a "Transcription-safe" audio step to 64
  therefore cleared the flag: the step showed as reversible, Apply asked nothing, every
  file was encoded and then refused by the action engine's lossy gate.
- **Choice:** `steps::step_lossy` is the only rule, used by `plan_add_step`,
  `plan_update_step` and `step_params_set` (step flag and response): a step is lossy when
  its parameters make it so, and a `LossyOptIn` card is lossy unless its parameters are
  exactly those of one of its own lossless presets (the audio card's FLAC preset; the
  output of that preset is lossless, so the action engine's gate accepts it without a
  confirmation). Changing the bitrate of an Opus step therefore keeps it lossy, and picking
  the lossless preset still removes the dialog, as the UI promised before. The earlier add
  and update paths (always lossy for such a card) asked once too often for FLAC; that is
  corrected by the same rule.

## D-6A21 The confirmation describes the items its digest binds (ENG-4)

- **Context:** the dialog took its count and sizes from the plan's cached estimate while its
  digest bound the live items. The cache could be a dry-run measurement of other items:
  it was keyed by the selection overrides (id sets: two empty sets match across entirely
  different candidate sets), recorded with the selection read when the dry run *ended*,
  and kept when a search under the same title replaced the record or another step removed
  rows. Shown failure: "Delete 1 file permanently?" confirmed the deletion of 5.
- **Choice:** (1) a dry-run measurement is keyed by `plan::measure_key`, a hash of the
  record's candidates (fresh ids at every analysis, sizes, groups, keepers), the groups'
  keepers, the selection and the skipped badges, computed by `build_step` under the same
  lock as the items it measures; `refresh_plan` reuses it only while the live record has
  the same key. Every replacement or change of the rows invalidates it without each
  mutation having to remember to (the reviewer's per-record revision counter would have
  needed a bump in every mutating path, in files the other lane owns). (2)
  `plan_confirmation_info` counts the items it digests (`files` = items built) and takes
  the bytes from the same record state (the measurement when its key matches, else the
  live estimate); if the record changed between building the items and reading the
  estimate it answers `conflict` (retry) rather than mixing two states.
- **Not covered by a dedicated test:** the in-flight race (selection changed while a dry run
  runs) is closed by construction (the key is taken with the items at queue time); the
  tests cover the search replacement end to end, including the apply count.

## D-6A22 The kept badge follows the selection (ENG-7)

- **Context:** a keep rule moved the `keeper` badge and the group's keeper, but "Reset
  selection" only cleared the overrides and a rescan carried the selection with fresh
  badges: the row labelled as kept was the one removed, and "select all except the kept
  copy" then selected it.
- **Choice:** `steps::sync_keepers` makes the badge a function of the selection: in every
  group, the member that survives the removal of the selected rows
  (`results::group_survivors`, the same rule apply uses) carries `keeper` and is the group's
  keeper; a group whose every member is selected keeps its badge (apply skips it whole).
  It runs after every selection change (`selection_set`, `selection_reset`), after a
  parameter re-run and after the rescan carry. After a reset this gives back the
  detector's keeper without storing it (the detector's defaults leave exactly that member
  unselected).
- **Not done:** the keep-rule dropdown still shows the rule saved in the step parameters
  (`opp.params.keep_rule`), which `selection_apply_keep_rule` does not update; the label
  can lag behind the rule last applied. Cosmetic, left for a later UI pass.

## D-6A23 The promised undo window never outlives the quarantine (ENG-9, ACT-10)

- **Context:** the step and search panels built the quarantine action with a fixed
  `purge_after_days: 7`, ignoring Settings (default 30), while the plan checklist and the
  confirmation promised the undo window (`undo_days`). Originals that an optimisation or
  an archive sent to the quarantine (explicitly, or through the Recycle Bin route on
  Windows) carried no retention and an undo deadline of `undo_days`: the purge then used
  whatever `quarantine_days` was at purge time.
- **Choice:**
  - UI: the quarantine action holds items for the Settings value (`quarantineDays`: the
    loaded settings, else one `settings_get`). Three UI tests that asserted the old fixed
    `7` now expect the mock's setting (30); a new test sets 12 and 9 and checks both are
    used.
  - mj-actions: `held_in_quarantine` gives an original kept in the quarantine by
    `optimise` or `archive` its retention (`quarantine_days`) and caps its undo deadline at
    the end of that retention, exactly as `quarantine_item` already did for plain
    quarantined items (F-15). The quarantine part of ENG-9 in mj-actions was already in
    place (F-15) and needed no change.
  - Plan checklist: `undo_window` names the shorter period when a reversible step keeps
    items in the quarantine for less than `undo_days` (Quarantine steps, Trash routed to
    the quarantine, originals kept there), via `plan::quarantine_days`. The confirmation's
    undo text of optimise/archive steps names the quarantine days when they are shorter.
  - `settings_set` refreshes the plan (checklist and estimates read settings) when the
    plan has steps; before, the checklist kept the old figures until the next plan change.
- **Rejected:** letting the engine read `purge_after_days: 0` as "use the setting" (a
  special value in a frozen action type, and a stored plan would change meaning when the
  setting changes).

# W6-A (fourth task): front-end review findings

Review findings UI-1 to UI-11 and DUP-2. UI-1 ("select all" under a filter selects hidden
rows) was already fixed on this lane as ENG-3 (D-6A19): `selection_set` takes the table's
`filter`, `min_size` and `keep_groups`, the UI sends them and the mock honours them; the
UI tests in `w6a-fixes.test.js` cover the UI-1 scenario (filter, then the header box), so
nothing changed for it. Every other finding was reproduced by a failing test first
(`ui/js/w6a-review.test.js`, `crates/engine/tests/w6a_fixes.rs`, `settings.test.js`,
`format.test.js`, `fakedom.test.js`).

## D-6A24 "Undo" on a finished run undoes that run (UI-2)

- **Context:** the completed banner and the queue summary called `undo_last`, which
  undoes the newest run that *has* undoable entries: when the run that just finished had
  none (all skipped, permanent delete, lossy), Undo restored an older unrelated run.
- **Choice:** both buttons send `undo_run` with the finished run's id. They are disabled,
  with the title "Nothing in this run can be undone", when History says the run is not
  undoable; before History has refreshed, a run that changed no file counts as not
  undoable (`completedUndoable`). The engine refuses an empty undo anyway, so the worst
  case of a stale list is an error toast, never another run. `undo_last` stays for the
  strip's "Undo last action", whose label says what it does. Two existing UI tests that
  asserted `undo_last` for these buttons now assert `undo_run` with the run id.
- **Rejected:** asking for confirmation as History does: the banner's Undo is the quick
  path the summary promises, and it now acts on exactly the run it sits under.

## D-6A25 The fake DOM's `querySelectorAll` is a NodeList (UI-3)

- **Context:** `focusables()` (modal Tab trap) and the table's arrow-key handler called
  `filter` / `indexOf` on `querySelectorAll` results; a real NodeList has neither, so in
  the WebView Tab escaped every dialog and the arrow keys threw. The fake DOM returned an
  Array, which hid it.
- **Choice:** `FakeNodeList` (frozen; `length`, indices, `item`, `forEach`, `entries`,
  `keys`, `values`, iteration; no Array methods), and both production sites use
  `Array.from`. A grep of `ui/js` for array methods on `querySelectorAll` results found no
  other production use (`queue.js` and `thumbs.js` only iterate with `for…of`). Tests
  that called Array methods on the result now use `Array.from`; `env.$$` returns an Array
  (it is a test helper, documented as such).

## D-6A26 The keep rule is a recorded parameter (DUP-2)

- **Context:** `selection_apply_keep_rule` changed the selection and the kept badges but
  not the opportunity's parameters: the dropdown showed the detector default, and a rescan
  (which runs the detector with `params_for`) silently went back to it. Only the mock
  recorded the rule. The dropdown also fell back to a fixed "newest" while the exact
  duplicates default is "oldest".
- **Choice:**
  - When the analysis has a `keep_rule` setting, the engine records `keep_rule` (and
    `preferred_folder` for that rule) in the opportunity's params and the plan step's
    params. A preferred folder that is not valid Unicode is not recorded (parameters are
    text; a mangled folder would make a rescan keep another copy): the selection still
    follows the rule, the recorded rule stays the previous one.
  - `PlanState::chosen` keeps, for the session, the parameters the user chose for an
    opportunity in the review (`step_params_set` and the keep rule), in the plan or not;
    `params_for` applies them between the detector defaults and the plan step's params.
    Before, any parameter set on an opportunity that was not in the plan was lost on the
    next scan; it is the same defect as DUP-2 and is fixed by the same map. They are not
    persisted across restarts (the plan is the persisted form, as before).
  - UI: the dropdown shows the recorded rule, else the default of the card's keep-rule
    setting; "Preferred folder…" opens the dialog at the recorded folder. The mock records
    `preferred_folder` like the engine.
- This also closes the note at the end of D-6A22 (the label lagging behind the rule).

## D-6A27 Scan command answers never overwrite a newer scan://done (UI-4)

- **Context:** events and command answers arrive on separate channels. A `scan_stop`
  answer (`stopping`) arriving after `scan://done` left the overview "Stopping…" with both
  buttons disabled; a `scan_start` answer after its own done left it "scanning".
- **Choice:** the store keeps `scanDoneGen`, the newest generation whose `scan://done`
  arrived. A pause/resume/stop answer in an active state for that generation or an older
  one is dropped; a start answer for a generation already done changes nothing but
  clearing the completed banner.
- **Rejected:** re-reading `scan_status` after each command: one more round trip that can
  race the same way.

## D-6A28 Open views follow rescans, applies and plan changes (UI-5, UI-6, UI-10)

- **Choice:**
  - The `refreshOpps` action (used by `scan://done`, `opportunities://changed`, "+ Add"
    and remove) also re-reads the Clean / Optimise / Archive list when one is shown, so a
    card becomes "In plan" at once and sizes follow a scan (UI-5).
  - `scan://done` and `history://changed` (sent when an apply or undo run ends) re-read the
    open step review and the Custom-search results, coalesced into one refresh per burst of
    events. Paging, sort and filters stay; a page that no longer exists falls back to the
    first (UI-6). `opportunities://changed` alone does not refresh the review: it also
    follows every tick, and the review already reloads after its own changes.
  - A failed `step_detail` replaces the review with an error block and Retry (the stale
    table is dropped: a step a rescan removed must not offer its old rows); a failed
    `candidates_page` shows the error with Retry in the table (UI-10). Background refreshes
    show failures in the view, not as toasts.

## D-6A29 Settings changes run in order (UI-7)

- `settings_set` replaces the whole object, so two overlapping commits built from the
  same snapshot undid each other (a number committed on blur, then a switch). Commits now
  run one at a time and each `mutate` applies to the settings the previous one saved.
- **Rejected:** a partial-patch command: a new IPC surface for a problem the UI can solve
  by ordering its own writes.

## D-6A30 Local time for every date (UI-8)

- `formatDate` and `formatDateTime` use the local time zone: the engine computes
  schedules in local time, and History, "undo until" and "next run" must agree with the
  user's clock. The format tests pin `TZ` (UTC for the existing expectations, Sydney and
  Los Angeles for the new one).

## D-6A31 The text box being typed in survives a re-render (UI-9, UI-11)

- **Context:** every store change rebuilds the main view and `replaceKeepingFocus`
  re-focused a *new* input: an IME composition is cancelled when its element leaves the
  document, and screen readers announce the field again on every key.
- **Choice:** when the focused element is a text box whose `data-fk` exists in the new
  tree with the same path shape, the box itself is never detached: each ancestor on its
  path adopts its counterpart's attributes and handlers and takes its other children; the
  box adopts the new attributes and handlers and the store's value (unless an IME is
  composing). Handlers made by `h` go through one dispatcher per event type, so a kept
  element runs the new closures. Anything else falls back to the previous behaviour
  (replace, then focus the new element). Text boxes that send while typing (the step
  filter, the Custom-search box) skip `input` events of a running composition and send on
  `compositionend`; the query box leaves Enter, Tab and the arrows to the IME meanwhile.
- **UI-11:** the focused element is found by comparing `data-fk` values, never through a
  selector built from them (keys may hold Windows paths with backslashes or quotes). The
  keys themselves stay raw paths: passing them through `fk()` would make `C:\a` and `C:/a`
  the same key. The store runs every subscriber even when one throws, then rethrows the
  first error, so one failing region cannot leave the others stale.
- **Not done:** debouncing the filter. With the element kept, a keystroke costs one view
  rebuild per answer, and the generation counters already drop stale answers.

## D-6A32 The engine is built off the main thread; the start-up purge is a job (IPC-2)

- **Context:** `Engine::new` ran inside Tauri's `setup`, on the main thread, before the
  event loop: journal replay, scan-cache load, tool probe (up to 3 s) and the automatic
  quarantine purge (one fsync per expired item). On Windows every default removal is a
  quarantine item, so a start 30 days after a large clean-up showed a blank, "Not
  responding" window for as long as the purge took.
- **Choice:** `setup` only manages the state and starts an `mj-start` thread that builds
  the engine with `purge_at_start = false`. `AppState` holds an `EngineSlot`; the single
  command helper (`commands::run`) waits for it on the blocking pool, so a command sent
  while the engine is being built is answered once it is ready, and none runs on the main
  thread. Once the engine is ready, `Engine::quarantine_purge_if_due` queues the ordinary
  `purge_quarantine` job (each item's own retention, the same rule as before) only when an
  item has expired: it runs in queue order with applies and undos (no race), reports
  through `job://` events, and a normal start shows no empty job.
- **Rejected:** moving only the purge to a thread. The other start-up work (tool probe,
  journal replay on a large journal) also blocks the window; moving the whole constructor
  costs one wait per command and nothing else. Purging inside the engine on a private
  thread was rejected too: it could race an apply or an undo the user starts at once.
- **Unchanged:** the CLI already built its engine without the start-up purge and purges
  at the start of acting commands.

## D-6A33 A second start says why it does not open (IPC-6, XC-2)

- **Context:** with the data folder held by another MyJanitor (the open app, a CLI
  command, a scheduled run), the GUI printed to stderr and exited. A Windows GUI process
  has no console, so a double-click or an Explorer "Find duplicates here" did nothing
  visible.
- **Choice:** a visible message, not focusing the running window. The shell hides the
  window and shows a native warning box (`rfd`, already a dependency for the file
  dialogs, built on the main thread like them) naming who holds the folder and, for a
  context-menu launch, which request was not carried out; the app exits when the box is
  closed. An engine that cannot be built at all (an app folder cannot be created) is
  reported the same way instead of a panic in `setup`. The decision is a pure function
  (`startup::decide`) with unit tests.
- **Why not focus the running instance:** the lock is held as often by a CLI command or a
  scheduled run as by the window, and those have no window to focus; a single-instance
  channel (`tauri-plugin-single-instance` or a hand-written named pipe / socket) would be
  a new dependency or new platform code, would only cover the GUI-holds-it case, and
  forwarding a context-menu request into a running session that may hold a plan in
  progress needs its own design (what happens to the current scope and results). A clear
  message covers every holder now; forwarding stays an open item.
- **Why not a read-only second window (XC-2's suggestion):** two GUIs on one data folder
  would both write `settings.json`, presets and schedules; the engine guards only file
  changes, not its own state files.
- **CLI (same finding):** a command that changes files (`apply`, `clean --apply`, `undo`,
  `run-profile`, `quarantine purge`) now refuses in `Ctx::open`, before it rescans,
  measures or asks for the typed confirmation, instead of failing at the very end.

## D-6A34 Pause and cancel are real for scheduled runs and purges (ENG-10)

- **Choice:**
  - A paused `schedule` job pauses its scan and blocks before queuing the apply until it
    is resumed; a cancel stops the scan. The job also checks its token after the scan, so
    a pause or cancel that arrives as the scan ends still holds the apply.
  - The purge job passes its token to `mj_actions::quarantine::purge_with` (new, additive:
    `purge` and `purge_expired` keep their signatures), which checks it between items. A
    stopped purge ends `cancelled` with the items deleted so far in its outcome
    (`succeeded`, `bytes_saved`); the rest stay held.
  - `JobResult::complete`: a job whose token was cancelled after it had done all its work
    (every item processed, or the body finished) ends as `completed` /
    `completed_with_errors`, not `cancelled`. Apply and undo bodies count processed items
    against their item list; the dry run keeps the old rule.
- The actions change is the smallest that lets the engine stop between items without
  copying the guarded purge loop into the engine.

## D-6A35 A scheduled run applies only its own steps (ENG-8)

- **Context:** the run checked once that the plan was empty, scanned for minutes, then
  added its steps and applied the whole session plan, including a step the user added in
  the meantime; on failure it cleared the plan, user steps included.
- **Choice:** the run remembers the steps it added and applies only those
  (`apply_plan_steps` with an id filter; `plan_apply` is the unfiltered form). A user step
  stays in the plan, untouched. On any failure the run removes only its own steps.
- **Rejected:** refusing the run when the plan changed during the scan. The schedule's
  steps are safe by construction (SAF-11) and the user did not ask for the run to be
  dropped; their own step is simply left for them.

## D-6A36 The scope follows the scheduled run's scan (IPC-7)

- **Context:** "Run now" set the session scope to the schedule's and replaced the
  results; the UI never re-read the scope, so the chips showed one folder and the cards
  another.
- **Choice:** keep the engine behaviour (the session scope is the scope of the results)
  and make the UI follow it: `scan://done` re-reads `scope_get` and replaces the chips
  when they differ. The chips, the cards and the Scan button then all describe the same
  folders.
- **Rejected:** restoring the user's previous scope after the run's scan (the reviewers'
  proposal). The results on screen would then belong to the schedule's folders while the
  chips named others, and the next "Scan" would silently replace them: a file cleaner
  should never show results for folders other than the ones it names. Re-adding a folder
  is one click; a mismatch between what is shown and what is acted on is not visible at
  all. The mock does the same as the engine.

## D-6A37 Applied items stay gone until the next scan (ENG-11)

- **Choice:** after each applied step, the rows done leave every card (as before) and a
  duplicate/pair group that loses members and is left with fewer than two leaves its card
  with its last row; kept badges and keepers follow the selection again (`sync_keepers`).
  The paths of done items that are no longer at their place (checked on disk: trashed,
  quarantined, deleted, moved, converted under another name; an in-place optimisation is
  still there) are remembered in `Results::removed` (path keys, ancestor lookup), and a
  re-analysis (`step_params_set`) or a search on the unchanged index leaves them out. A
  scan clears the set.
- **Not done:** the disk-usage and breakdown views still count the old index until the
  next scan (`disk.rs` belongs to the other lane this wave); no "stale" flag was added to
  `opportunities_list`.

## D-6A38 Custom-search cards survive a rescan; schedules refuse them (ENG-12)

- **Choice:** `search_run` keeps its request per `custom:<n>` id (`Results::custom_rules`,
  session only). A scan runs each rule again on its new index before publishing, so the
  card keeps its id and title, the rows the user ticked or unticked are carried like any
  card's (G-04), and its plan step is no longer dropped by `plan_after_rescan`. A rule
  that no longer evaluates loses its card (and so its step), as an opportunity that
  disappeared does.
- **Schedules:** a schedule stores step templates, not rules, so a custom-search step
  could never run. `schedule_save` and `schedule_from_plan` now refuse `custom:` steps
  with a message instead of saving a schedule that reports "nothing to do" forever.
  Storing the rule in `ScheduledStep` would change the core type and the schedules file
  format; left as an open item.

## Smaller IPC, CLI and diagnostics findings (IPC-4, IPC-5, IPC-8, IPC-10, IPC-11, IPC-12, XC-3)

- **IPC-4:** `scan://progress` after the walk now emits the corrected event (the file count of
  the walk), not the zero it stored a corrected copy of. `dirs` is left 0 (not claimed anywhere
  in the UI).
- **IPC-5:** `loadSettings` also refetches `app_info`, so the trash route label follows
  "Use the Windows Recycle Bin" without a restart. The mock already derived the route.
- **IPC-8:** the strip's "Last report" button is enabled from the history list already
  fetched (a non-undo run with items); the report is fetched on click only. The engine-side
  clone in `history_run` is left as an open item (needs a journal iteration helper in the
  actions/core lane).
- **IPC-10:** `plan://dry_run` with `error` shows a cancelled notice or an error toast, never
  the success toast; `job://done` of a dry run clears the busy flag even when the body
  panicked and sent no event.
- **IPC-11:** the Nautilus script makes the name absolute and passes `--duplicates=<abs path>`,
  so a folder named like a launch option is a path. `launch.rs` already parsed the inline form.
- **IPC-12:** `undo` prints the run the waited job really started and no "Undo with" hint
  (undo runs are not undoable). A run whose items were only skipped exits 0 and says how many
  were skipped on stderr: exit 3 is documented as "some failed items", and skipped items are
  the safety checks working (a scheduler must not record them as failures). Only failed items
  exit 3.
- **XC-3:** one std-only log, `myjanitor.log` in the data folder, 256 KiB with one rotated copy.
  Written by a process-wide panic hook (installed by `Engine::new` so the GUI, CLI and
  scheduled runs all get it) and by the engine's catch sites (job, scan, detector), which also
  put the payload into the message the UI shows. No dependency, no file contents, paths only
  as the failing operation's message carries them. Not done: surfacing the log path in
  `app_info` and an "Open log folder" action (an IPC contract change), a log line for cache
  resets and journal replay, and the payload text at the `parallel_map` and thread pool sites
  (the hook already records every panic there).

## D-6A39 Merging lane-b: textual conflicts resolved by keeping both behaviours

The two lanes of this wave (W6-A here, W6-B in `W6-B.md`) were merged on `lane-a` with one
merge commit; every conflict was resolved by keeping both lanes' behaviour and tests, never
one side wholesale.

- **PNG decoder (`crates/dupes/src/decode/png.rs`).** W6-A had split it into a shared
  `Prepared` (fingerprint and review thumbnail) that inflated the whole image; W6-B made the
  fingerprint inflate one scanline at a time (DUP-5). `Prepared<'a>` now borrows the IDAT
  chunks and `for_each_row` streams them, so both the fingerprint and the thumbnail keep
  O(width) memory. Test: `crates/dupes/tests/thumb_memory.rs` (same counting allocator as
  W6-B's `png_memory.rs`, its own binary so the count sees one test).
- **Dry-run measurements (`plan.rs`, `apply.rs`).** W6-A (ENG-4) keyed a measurement by the
  identity of the items it measured (`measure_key`); W6-B stored it with the selection and
  the step's overlap with the other steps (`Measured::shared`). `Measured` now holds the
  action, `key`, `shared` and the estimate, and stands only while all three still match.
  The selection is part of `key`, so the separate selection field was dropped.
- **Confirmation figures (`plan_confirmation_info`).** W6-A computed the fallback with
  `plan::step_estimate`, which W6-B removed in favour of `savings::tally`. The dialog now
  shows the step's share of the plan's one account (`plan::step_tallies`), the same figure
  as the plan card: a file two steps list counts in one dialog only. `files` stays the
  number of items the digest binds (W6-A). Test:
  `savings_invariant::the_confirmation_shows_the_steps_share_of_the_plan_account`.
- **After an applied step (`after_apply_step`).** W6-B's `DoneSet` (O(depth) per row) drops
  the gone rows; W6-A's ENG-11 bookkeeping (`Results::removed`, groups left too small) is
  kept; the function returns a set (W6-B) so the reclaimed-bytes sum is not quadratic.
- **Settings.** `settings_set` applies Settings > Scanning to the scope (W6-B SCAN-5) and
  then refreshes the plan (W6-A ENG-9).
- **Struct literals.** W6-B's new tests build `CandidatesPageRequest` and
  `SelectionSetRequest`, which W6-A extended (`min_size`, `keep_groups`, `filter`); they
  now pass the neutral values (no filter, flat table), which is what W6-B's tests meant.

## D-6A40 Semantic conflicts checked after the merge

- **Filtered select-all, grouped table and recorded keep rule against the one account.**
  The engine paths compose (both change the selection, then `recompute` runs the tally);
  the mock does the same. Test: `ui/js/merge.test.js` (select all on a filtered grouped
  table, then the keep rule: the card is what the selection frees, the hero the sum of the
  cards, the rule recorded).
- **String entry ids against W6-B's journal changes.** W6-B changed how records are
  written (ACT-12) and queried (XC-4), not the id type; `EntryId`'s string codec applies to
  every new caller. Nothing to change.
- **`purge_with` (W6-A) against W6-B's quarantine changes.** `purge_with` goes through
  `purge_by`, which now reads `held` through W6-B's `entries_where`; the cancel token and
  the per-item retention are unchanged, and the ACT-9 one-day minimum applies to what is
  quarantined, not to an explicit purge. Both lanes' purge tests pass.
- **Overview cards.** W6-B changed what a card shows (what it frees) and added `to_review`;
  W6-A's overview changes (undo of the finished run, the native folder dialog) touch other
  elements. Both sets of UI tests pass against fixtures regenerated on the merged tree.
- **Real-response fixtures.** Regenerated once (`MJ_WRITE_UI_FIXTURES=1`). They had been
  stale since before the wave: the two archive cards were still "not implemented"; they are
  real analyses now and, empty in the fixture tree, are not listed.

## D-6A41 A PNG's image stream must end exactly after its last row (review suggestion)

- **Finding (reviewer, verified):** the streaming decoder stopped as soon as the last
  scanline was produced. A zlib stream cut short (checksum missing), holding more rows than
  the header or followed by more data was fingerprinted, so a malformed file could join a
  similar-image group and be pre-selected.
- **Choice:** after the last row the decoder drives the stream to its end through a one-byte
  probe: any further output, a missing end or data after the end is `Corrupt`. The thumbnail
  shares the check. Strict on purpose: a file refused here only loses similar-image matching
  (it is still exact-matched); a wrong fingerprint could pre-select a file for removal.
- **Test:** `decode::png::tests::image_data_must_end_exactly_with_the_last_row` (failed
  before the fix on the over-long and the truncated stream).

## D-6A42 A verified duplicate is read again after the last re-stat (review suggestion)

- **Finding (reviewer, verified):** the hash a removed exact duplicate is journalled with
  came from its byte comparison with the kept copy, before the intent. Rewritten in place
  since then (same size, same mtime, same file id), the file passed every re-stat, was
  removed although it no longer matched the kept copy, and undo later refused it ("its
  content differs"). The same window existed at the baseline (the audit hash was also taken
  before the re-stat); ACT-8 did not open it.
- **Choice:** in `act_retain`, after the kept copy's re-stat, a duplicate whose hash came
  from the comparison is hashed again; a different hash skips it as changed (nothing
  removed). The `changed` time (ctime) would avoid the read on Unix, but the Windows
  metadata path does not fill it, and Windows comes first.
- **Cost:** a removed duplicate is read twice (comparison and this check) instead of once;
  the baseline read it three times. The ACT-8 read-count test now expects three reads in
  total (both files for the comparison, the duplicate once more) and says so.
- **Not done:** files that are not duplicates keep their audit hash from before the
  re-stat (baseline behaviour): a same-size, same-mtime rewrite in that window still
  journals a stale hash. Closing it means hashing after the move, which changes the intent
  record recovery reads; left as an open item.
- **Test:** `review3_fixes::a_duplicate_rewritten_after_the_comparison_is_not_removed` (the
  test platform's new `on_metadata` hook rewrites the file at the kept copy's act-time
  re-stat; it failed before the fix).

## D-6A43 An unreadable duplicate at the act-time check is a skip (review finding)

- **Finding:** the act-time read of a verified duplicate (D-6A42) passed its read error up
  as an I/O error, so a duplicate removed or locked by another program (a share-deny lock
  on Windows) between its re-stat and that read was reported and journalled as Failed. The
  same read failing one step earlier, in the byte comparison of `check_keep`, is the skip
  "cannot be read".
- **Decision:** map it the same way: a read error there is the skip "skipped: <path> cannot
  be read; nothing was removed". Nothing was touched at that point, so a skip is the true
  outcome, and run outcomes and reports no longer count a failure for what is by design a
  skip. A different content is still `Changed` (also a skip).
- **Test:** `review3_fixes::a_duplicate_unreadable_at_the_act_time_check_is_skipped_not_failed`
  (the `on_metadata` hook deletes the duplicate at the kept copy's act-time re-stat; it
  reported `failed: 1` before the fix).
- **Status record:** the W6-B SCAN-7 open item (disk view rebuilds totals per request,
  unused `FolderTree` in `breakdown`, W6-B D-B6) was missing from the wave-6 entry of
  docs/STATUS.md; added.
