# W3-E decisions (settings, schedules, presets, paths, context menu, CLI, Settings drawer)

IDs `E-xx` are local to this file.

## E-01 Paths
- Pure `paths::resolve(os, env, exe_dir, marker)`; `detect()` feeds it the process. Windows data = `%LOCALAPPDATA%\MyJanitor`, config = `%APPDATA%\MyJanitor` (reserved: the engine has one `data_dir`, so settings, schedules, presets, journal and quarantine all live in data). Linux uses XDG with the spec's fallbacks (relative XDG values ignored). Portable: a `portable` (or `myjanitor.portable`) file beside the exe, everything under `<exe dir>/data`. `--data-dir` = `AppDirs::under(root)`, cache `root/cache`.
- Public surface for OS code is `mj_engine::paths::integration` (re-exports), because `lib.rs` keeps `settings` and `schedules` private. A `pub mod` in `lib.rs` would be cleaner (request below).

## E-02 Tolerant state files
Corrupt `settings.json` / `presets.json` / `schedules.json` are moved to `<name>.bad-<secs>` and defaults used; a parseable settings file with out-of-range values is repaired, not rejected. `settings_set` is strict, names the field, normalises first. Unknown fields ignored by serde. "Kept languages" does not exist in the frozen `Settings`, so it is not stored.

## E-03 Side effects
Toggling `integration.context_menu` runs the install/remove before storing; failure keeps the old settings. Skipped without persistence, under `MYJANITOR_NO_OS_INTEGRATION`, and refused in portable mode (portable never writes outside its folder). The FS-compression watcher is stored, shown unavailable.

## E-04 Schedules
- Reversible only: lossy or permanent actions, optimisers lossy by nature or by a lossy toggle/preset in the params, `Action::None`, empty steps and relative roots are rejected at save. Names unique (case-insensitive); ids `sched_<max+1>` (survive restarts; `next_id` restarts at 1 each process).
- `next_run` is wall-clock maths on a fixed UTC offset (`MYJANITOR_UTC_OFFSET_MINUTES`, else `date +%z`, else the registry bias on Windows, else 0). DST is not modelled. Monthly day 1-28.
- OS registration jobs run `myjanitor-cli run-profile <id>` (the id never needs quoting). Windows `schtasks.exe` per user (note: `/TR` is limited to 261 characters); Linux a systemd user service + timer; a crontab line is generated but never installed. Installation only through `schedule install` (CLI) on explicit request.
- `schedule_run_now` blocks until the scan is done, refuses while a plan is in progress, skips steps whose opportunity vanished (`nothing to do` otherwise), and applies with no confirmations. The run is journaled by `plan_apply` as kind `Apply`, not `Schedule`.

## E-05 Presets
Built-in and validation logic reused from `mj_filters::presets`; additional rule: names unique across built-in and user presets.

## E-06 Context menu
Windows: `reg.exe` argument vectors under `HKCU\Software\Classes\{Directory,*}\shell\MyJanitor.*`; Linux: a `.desktop` file with actions plus Nautilus scripts where the scripts folder exists. Files we write carry a marker and are only removed when they still carry it. The entries launch the GUI with `--optimise <path>` / `--duplicates <path>`: the shell must accept those arguments.

## E-07 CLI
- Spec-driven parser (`args.rs` table drives help, validation, errors). Exit codes 0/1/2/3.
- The engine keeps scan results and the plan in memory, so the CLI stores the last scope and plan templates in `<data>/cli-session.json` and re-runs the scan (cached) and replays the plan in later invocations. `clean <roots> --detectors a,b [--dry-run|--apply]` does it in one process.
- `apply` refuses without `--yes` on a non-terminal, asks for a typed `APPLY` on a terminal, and lossy/permanent steps need `--confirm-lossy` / `--confirm-permanent`. `--originals` accepts trash or quarantine only.
- Refused roots: filesystem or drive root, the home folder, anything that contains the data or cache folder.
- `settings set` edits the settings JSON by dotted key and lets the engine validate; a configured temp folder becomes the process temp folder.

## E-08 Settings drawer
Own `copy.js` and `controller.js` in the view folder (shell `copy.js`/controller are not ours). The controller keeps drawer-only state per shell `actions` and calls `api.js` directly (default import, injected in tests); it refreshes the store through the shell's `loadSettings`. The picker is inline (the shell's browser modal only adds scope roots). Existing classes only: no new CSS file is allowed. "Delete originals" is not offered as a default (per step only).

## Requests to the orchestrator
1. `lib.rs`: `pub mod settings; pub mod schedules;` (then drop the re-exports in `paths::integration`).
2. IPC: commands to install/uninstall an OS schedule and the context menu would let the GUI do it; today the drawer shows the `myjanitor-cli schedule install <id>` commands and the toggle uses `settings_set` side effects.
3. Shell: accept `--optimise <path>` and `--duplicates <path>` launch arguments; call `Engine::schedules_due` on a timer while open if in-app runs are wanted.
4. Mock backend: `schedule_save`/`schedule_delete` are `notYet`, `settings_set` does not validate, `protected_paths_set` is not persisted.
5. Controller: `miscActions` could drop `clearCache`'s duplicate if the drawer controller is wired through `createController`.
