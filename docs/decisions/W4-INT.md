# W4-INT decisions (Wave 4 integration pass)

Format of `docs/DECISIONS.md`; ids D-4Ixx. Rule applied throughout: user data safety
first (reversible over irreversible, skip over guess, report over act).

## D-4I01 Optimise path: what was already wired, what was broken at the seams
- **Context:** the engine facade already injected `mj_optimise::optimisers()` and
  `archiver()` into `mj_actions::Services`; no test drove a real optimiser through
  scan → card → plan → dry run → apply → undo.
- **Found and fixed:**
  1. A step's parameters never reached the optimiser when they were given to
     `plan_add_step` / `plan_update_step` (only `step_params_set` copied them into
     `Action::Optimise::params`), so a chosen preset (e.g. audio `speech`) silently ran
     with the card defaults, and a lossy preset did not make the step `lossy_opt_in`.
     The step's params are now the single source of truth: values given inside the
     action, then the request's `params`, are merged into the step and copied into the
     action; `lossy` is recomputed from the params (`steps::params_lossy`).
  2. A conversion (WebP, Opus, MP4) replaced `x.wav` with Opus bytes under the old
     name (the type travelled only in a note). The action engine now names the result
     `stem.<new ext>` (unique), in replace, delete and light-copy modes; undo already
     handled an output path different from the original.
  3. The dry run compared the optimiser output (file length) with the *allocated* size
     of the original, so a file a few bytes larger after optimisation could be promised
     as a saving and then refused at apply. Both sides now use file lengths.
- **Tests:** `crates/engine/tests/optimise_e2e.rs` (PNG, JPEG, EPUB, PPTX with duplicate
  media + thumbnail, PDF; replace + undo byte-exact, light copy + undo, keep-if-not-smaller,
  labelled deck locked, lossy gate with real ffmpeg).

## D-4I02 Additive `OptimiseOutput` fields
- `output_extension: Option<String>`, `breakdown: Vec<SizePart>` (OPT-ENG-09),
  `privacy: bool`; the struct now derives `Default` and every literal uses
  `..OptimiseOutput::default()` so further additions break nobody.
- The legacy `output_extension=<ext>` note is still read
  (`OptimiseOutput::converted_extension`), so an optimiser written against the old
  contract (e.g. the concurrent `archive.rs`) keeps working. Only short ASCII
  alphanumeric extensions are accepted (never a path fragment).
- `privacy`: a GPS strip that zeroes bytes in place keeps its size; keep-if-not-smaller
  keeps such an output only when it is exactly the same size, never larger
  (`mj_actions::engine::keeps_output`). The JPEG optimiser sets it for `strip_gps`.
  Known gap: the JPEG *analysis* still lists only files whose estimate shows a saving,
  so a GPS-only privacy run is reachable from a step whose params are `strip_gps` on a
  file that is otherwise listed, not as a card of its own.
- Office optimisers fill `breakdown` and the review columns `part:<name>` = bytes
  (the UI's size-parts bar, D2-06); PDF fills `breakdown` from its report line.

## D-4I03 Estimate warnings travel per candidate, not on `Estimate`
- **Context:** `Estimate` is `Copy` and built literally in ~25 places, one of them in a
  file another agent is editing right now; a `Vec<String>` field would break `Copy` and
  every literal.
- **Choice:** compatibility warnings (OPT-FSC-04 game libraries / DirectStorage) stay in
  the reserved candidate column `warning`; the engine aggregates them: plan checklist item
  `compatibility_warnings` (level warning) and one line per distinct warning in
  `DryRunStep.warnings`.

## D-4I04 Other additive contract follow-ups
- `gifsicle` is a known tool id (detected and listed); there is still no GIF optimiser
  card (an `OptimiserId` variant is a contract enumeration the UI keys on; no verified
  GIF pass exists), so only the argument builder ships.
- `Settings.general.kept_languages` (normalised: trimmed, lower case, `_`→`-`, deduped;
  validated as `ll[-region]`): fills the language-pack detector's `kept_languages`
  parameter when the step leaves it empty. Empty = system language as before.
- `AppInfo.capabilities`: static per build and OS (`recycle_bin`/`freedesktop_trash`,
  `wof_compression`/`btrfs_compression`, `cloud_dehydrate`, `reflinks`, `hardlinks`,
  plus `context_menu` and `os_schedules` outside portable mode). Volume features stay
  per drive in `drives_list`. `ui/js/lib/platform.js` labels "Windows only" from it
  (platform name only as a fallback).
- Restore pre-check: `HistoryRunDetail.occupied` lists the page's entries whose original
  place is taken now (removals, moves, conversions; never in-place replacements, whose
  place holds MyJanitor's own output). The History view disables Restore for them and
  says why; the restore itself still re-checks and refuses (`conflict`).
- Not done: report paging (`RunRequest` is `deny_unknown_fields`, so paging needs a new
  request type and UI paging; reports are bounded by run size today); Btrfs zlib/lzo
  reporting (a new `FsCompressAlgo` variant the UI and journal key on; not trivial).

## D-4I05 Engine modules, `schedule_run_now` as a job, OS integration from the CLI only
- `settings` and `schedules` are `pub mod` in `lib.rs`; `paths::integration` stays as a
  set of short paths (the CLI uses them), no longer a workaround.
- `schedule_run_now` (IPC) now returns a `Job` of the new kind `schedule` at once (the
  response type changed from `ApplyStarted`; the UI never read it). Checks that need no
  scan (unknown/invalid schedule, a plan in progress, a scan running, another scheduled
  run starting) are refused synchronously; the job scans, queues the apply jobs (one run,
  default selections, no confirmations: schedules hold reversible steps only) and
  finishes, setting its `run_id`; cancelling it stops its scan. The blocking form is
  `Engine::schedule_run` (CLI `run-profile`).
- `schedules_due` is unchanged and **not** polled by the GUI: an OS registration
  (`schedule install`) is the one trigger, so a profile never runs twice (OS timer + open
  app) against the same journal.
- No IPC commands for installing an OS schedule or the context menu: the drawer keeps
  the `myjanitor-cli schedule install <id>` hint and the `integration.context_menu`
  settings side effect (E-03). Writing to the registry / systemd from a webview click
  would add a second, less visible path to the same side effect; the CLI path is
  explicit and already tested.
- Mock: `schedule_save` (validation, unique names, ids), `schedule_delete`,
  `schedule_run_now` (a `schedule` job), `settings_set` validation (engine's ranges and
  messages for the drawer's fields) and persisted `protected_paths_set`.

## D-4I06 GUI launch arguments
- `src-tauri/src/launch.rs`: `--data-dir <dir>` (every folder below one root, as the
  CLI's; used by the smoke run so it never touches real app data), `--optimise <path>`
  (alias `--optimize`) and `--duplicates <path>`. Unknown arguments are ignored (the
  webview may add its own); a flag without a value is ignored.
- The entry point travels as `EngineConfig::launch` → the scope is set to that path at
  start-up (only if it exists) and `AppInfo::launch` reports it; the UI opens the
  Optimise view or the Clean view (duplicates live there). No scan and no apply start by
  themselves: the user still presses Scan and reviews.

## D-4I07 UI ↔ real backend shapes
- `crates/engine/tests/ipc_shapes.rs` runs a real session (scan of a generated tree, a
  detector and an optimiser step, dry run, apply) and compares the field names of 28 real
  responses (every read command the UI calls) recursively with the IPC.md example of the
  same type, both directions; fields that are `skip_serializing_if` or data-dependent are
  listed as optional in the test. Result: no field name drift on the main shapes; the
  only differences were optional fields absent from examples (`StepDetail.step_id`,
  `Preview.reason`, `JournalEntry.lossy/original_mode/original_sha256`,
  `DiskNode.other`), recorded as optional.
- `MJ_WRITE_UI_FIXTURES=1 cargo test -p mj-engine --test ipc_shapes` writes those
  responses to `ui/js/testing/real/*.json` (temp roots replaced by `/fixture/...`);
  `ui/js/real.test.js` mounts the real controller and views on them (overview cards and
  plan, step review rows, history runs and entries, settings, disk usage) and fails on
  `undefined`/`NaN`/`[object Object]` text or console errors.
- Limits: only field names are compared (not value types or enum spellings, which the
  `ipc_examples_parse` test covers by deserialising the examples); events are not
  replayed from a real session.

## D-4I08 Scan orchestration (item 7): confirmed, one gap left
- Confirmed: the engine scans through `mj_scan::scan_with_cache`, prunes and saves the
  cache (temp file + `sync_all` + rename) only when the generation is still current;
  detectors receive real sync roots, Downloads, font folders, the protected-path list
  (built-in + user + MyJanitor's own folders) and now the kept languages.
- Not done: the Windows one-pass list-with-metadata (`FindFirstFileExW` data reused
  instead of a per-entry `GetFileAttributesExW`). It needs a new `Platform` listing method
  and unverifiable FFI on this Linux host; the MFT fast path already avoids per-file opens
  when elevated. Logged as a perf follow-up.

## D-4I09 Error paths and control (item 8)
- `crates/engine/src/fault_tests.rs` swaps analysers for doubles through
  `Arc::get_mut` right after `Engine::new`, so the real engine runs: a panicking detector
  becomes a disabled card (scan survives); a panicking optimiser fails its item only
  (original intact, no `.mj-` temp left, no `intent` left in the journal); a cancel in the
  middle of a file removes the half-written temp output, keeps the original and never
  starts the next file; a lossy optimiser is refused without confirmation and runs with
  it (no ffmpeg needed); a second apply is `conflict`; pause/resume/stop in the detector
  phase publish nothing and the next scan works.
- Not exercised here: permission-denied paths (the test container runs as root, so
  `chmod 000` does not deny anything; the reporting code is covered by `mj-scan` /
  `mj-actions` unit tests) and a full destination disk (no way to fill a volume safely in
  this environment; `ENOSPC` takes the same `io` error path as any failed write, which
  the panicking/failing optimiser tests cover).

## D-4I10 Large-tree performance (item 9)
- `crates/engine/tests/perf_large.rs` (`#[ignore]`): 200 000 files / 211 000 folders
  generated in a temp dir (60 % in `node_modules` folders, temp files, 20 000 duplicate
  pairs, plain files); scan → detectors → opportunities → paging → plan → dry run → warm
  rescan, peak RSS from `/proc/self/status`. Run:
  `CARGO_INCREMENTAL=0 MJ_TEST_WAIT_SECS=900 cargo test --profile ci -p mj-engine --test perf_large -- --ignored --nocapture`.
- Hot spot found: per-candidate badging (`ProtectedPaths::is_protected`, also used by the
  guard for every item of a dry run/apply). Each call re-normalised ~50 built-in folders
  and exact paths (`path_key` twice per `is_within`) and ran the O(pattern × text) glob
  table for 11 built-in patterns. Fixes: folder prefixes and exact keys computed once per
  `ProtectedPaths`; `Glob` keeps its longest literal run and rejects a text that lacks it
  before building the table. Same semantics (existing tests unchanged).
- Numbers (profile `ci`, 4 cores, this container), before → after:
  scan + detectors cold 11.4 s → 5.7 s; warm rescan 11.3 s → 5.3 s; dry run over 211 000
  selected items 20.0 s → 6.9 s; exact-duplicates badging at 48 600 candidates 1.86 s →
  0.32 s. Paging stays 2–95 ms per page (194 000-row card), `opportunities_list` 15 ms,
  `plan_add_step` ≤ 40 ms. Peak RSS 1.6 GiB, inflated by the test's event recorder (it
  clones every event); not measured separately.
- Left: the dry run reads 1 KiB of every selected file for content badges (by content,
  not by extension, so a renamed signed PDF is still caught; kept on purpose).
