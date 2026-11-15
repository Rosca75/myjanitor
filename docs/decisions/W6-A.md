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
