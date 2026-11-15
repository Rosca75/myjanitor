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
