# Decisions

One entry per non-trivial decision: *context → options → choice → why*.

## D-001 Working branch
- **Context:** the brief suggests `build/initial-app`; the session environment only permits pushing to `claude/eloquent-shannon-4pcqld`.
- **Options:** (a) `build/initial-app`, (b) the session branch.
- **Choice:** (b).
- **Why:** pushing elsewhere is not permitted in this session; the brief's name was an example.

## D-002 Whole-page wireframe preview
- **Context:** the brief expects `docs/design/Option B.png`; only the individual frames and the `.figcraft` source were supplied.
- **Choice:** work from `docs/design/option-b-frames/*.png` and the parsed `.figcraft` (the brief's stated fallback).

## D-003 Subagent isolation
- **Context:** the brief asks for one git worktree per code-writing subagent.
- **Choice:** worktrees are created by the orchestrator under `/home/user/myjanitor-wt/<workstream>` on `ws/<workstream>` branches and merged one at a time into the working branch.
- **Why:** the session's primary directory is not itself a git repository, so automatic worktree isolation is unavailable; manual worktrees give the same isolation.

## D-004 Light theme only
- **Context:** the brief asks for light and dark themes if the palette has both.
- **Choice:** ship light only.
- **Why:** the doc-anonymiser palette defines a single light `:root` and no dark variant (see `docs/research/palette.md`).

## D-005 Partial hash windows
- **Context:** dedup-photos partial-hashes only the first 64 KB; general files (video, disk images, archives) often share headers.
- **Choice:** partial hash covers head, middle and tail windows of 64 KB each.
- **Why:** fewer false partial collisions on large files for three bounded reads; full-hash confirmation is unchanged, so correctness does not depend on it.

## D-006 Known flaws in the reference cache are not copied
- **Context:** R2 found that dedup-photos buckets only cache misses by size (a new identical copy of a cached file can be missed), never prunes deleted entries, and does not count some failed opens.
- **Choice:** MyJanitor buckets all files by size regardless of cache state, prunes entries not seen in a completed scan of their root, and reports every failed read.
- **Why:** user data safety and "a failed read must never look like no duplicates".

## D-101 Workspace layout and names
- **Context:** the brief suggests a layout; binaries must be `myjanitor` and `myjanitor-cli`.
- **Choice:** the suggested 11 crates under `crates/` with an `mj-` package prefix (`mj-core` ... `mj-engine`), `crates/cli` = package `mj-cli` with `[[bin]] myjanitor-cli`, `src-tauri` = package `myjanitor` (`[lib] myjanitor_lib`, rlib only: no mobile targets). Edition 2021, resolver 2, `rust-version` 1.88, rustfmt `max_width = 120`.
- **Why:** short unambiguous crate names; the package name gives the GUI binary name directly.

## D-102 Lossless path codec on the wire and in persisted files
- **Context:** `serde`'s `PathBuf` support fails on non-UTF-8 paths, which would make a whole response or journal line fail; undo needs exact paths.
- **Options:** (a) lossy strings, (b) byte arrays, (c) plain string when valid, escaped form otherwise.
- **Choice:** (c): valid Unicode paths are plain strings; others are `"\u0000"` + `%XX` (Unix bytes) / `%uXXXX` (Windows surrogates). NUL never occurs in a real path, so no collision. `mj_core::pathutil::serde_path` on every `PathBuf` field.
- **Why:** readable JSON for the 99.99 % case, exact round trip for the rest (undo, journal).

## D-103 Timestamps are integer nanoseconds
- **Choice:** `Timestamp(i64)` ns since the epoch everywhere, also on the wire. JS divides by 1e6; its ~256 ns rounding is irrelevant for display, and the UI never sends a scan timestamp back.
- **Why:** one type for cache stamps (ns precision needed), journal and IPC; no conversion bugs.

## D-104 No dialog plugin: in-app folder browser
- **Context:** folder picking is needed for "+ Add location", move/copy/backup destinations and export targets. Options: `tauri-plugin-dialog` 2.8.2 (pulls `tauri-plugin-fs`, `rfd`, adds `dialog:default`), or an in-house browser.
- **Choice:** in-house: the `fs_list_dir` command (read-only listing with places: home, common folders, drives/mount points) and a browser dialog in the UI. Save targets are a folder plus a file name in the same dialog.
- **Why:** under 5 h of work (dependency rule), capabilities stay `core:default` only, no webview filesystem scope, no extra transitive tree. Trade-off: not the native OS dialog.

## D-105 IPC style
- **Choice:** one Tauri command per engine operation, named like the engine method; a single `req` object argument (`invoke(name, {req})`), snake_case JSON identical to the Rust names, requests reject unknown fields; errors are an `ErrorInfo` object (`{code, message, hint, path?}`) instead of the reference's plain string; events named `<area>://<name>`; long operations return at once and report through events. `docs/IPC.md` is machine-checked against `api::COMMANDS`, the Rust types (every JSON example deserialises) and `ui/js/api.js`.
- **Why:** a stable code lets the UI choose the right state (disabled feature vs error); one argument keeps wrappers, CLI and mock trivially identical; checks keep three implementations in sync.

## D-106 One global scan cache file
- **Context:** dedup-photos keeps one cache file per scan root and merges others' entries.
- **Choice:** one store `<cache dir>/scan-cache.bin` keyed by `path_key`, binary format with presence bits and an xxh64 trailer (ARCHITECTURE.md §6).
- **Why:** MyJanitor scopes overlap often (`C:\Users` and `Downloads`); a global store avoids duplicated entries and the merge logic; size is fine to ~10^6 entries (~100 MB worst case, typically 20 MB at 200k files).

## D-107 Size units
- **Choice:** binary units with Windows Explorer labels (`KB`, `MB`, `GB` = 1024^n), one decimal below 100.
- **Why:** numbers must match Explorer and the drive bars on the primary platform.

## D-108 Accent is orange for the primary action only; selection and "in plan" are blue
- **Context:** the palette's accent is orange `#FD5108` with the rule "one orange hero element per view"; the wireframe uses one blue accent for actions, selection, in-plan borders, toggles and checkboxes.
- **Choice:** `--accent` (orange) for the primary button, progress and focus; a separate `--select` family (palette blue `#205FAE` / `#EEF5FF` / `#CFE0FB`) for selection, checkboxes, toggles, active tab, selected rows and the "in plan" state.
- **Why:** follows the palette's own guidance (palette.md §5 notes); four orange in-plan cards plus an orange Apply button would break the hero rule and blur the primary action.

## D-109 Lossy acting button vs primary button
- **Context:** the palette's warning amber `#E9B01F` and accent orange `#FD5108` are both warm; the lossy dialog's acting button must be visibly different from the primary button.
- **Choice:** primary = orange fill, **black** text (6.36:1; white would be 3.30:1 and fail AA), no border, no icon. Lossy acting button = amber fill `--warning`, black text (10.69:1), 1 px `--warning-strong` `#6B5310` border (7.32:1 vs white), a leading warning glyph, hover `#F2C649` (12.95:1). Permanent acting button = `--danger` `#DC2626` fill with white text (4.83:1), hover `#B91C1C` (6.47:1). The lossy dialog never shows an orange button.
- **Why:** hue, luminance (amber is much lighter), border and icon all differ, so the difference does not rely on colour alone; every text pairing passes WCAG AA.

## D-110 Focus ring colour
- **Choice:** `--focus-ring` = `#FD5108` instead of the palette's `#FE7C39`.
- **Why:** `#FE7C39` is 2.57:1 against white, below the 3:1 non-text contrast minimum; `#FD5108` is 3.30:1.

## D-111 Typography
- **Choice:** the palette's system fonts (Helvetica/Arial, Consolas) instead of the wireframe's Inter; the wireframe's pixel sizes and weights (titles 700, body 12 px).
- **Why:** offline app, no font files to vendor; Option B wins on layout and hierarchy (brief §4 precedence).

## D-112 Journal format
- **Choice:** JSON Lines, append-only, one full `JournalEntry` snapshot per line, last line per id wins, torn last line ignored, monthly files plus `runs.jsonl`; `intent` written and synced before acting (ARCHITECTURE.md §7.3).
- **Why:** crash-safe without a database, human-readable for audits, trivially replayable; serde_json is already a dependency.

## D-113 Optimisers are pure transforms
- **Choice:** `Optimiser::optimise` only reads `src` and writes a new `dst`; keep-if-not-smaller, light copies, originals policy, timestamps and the journal are applied by `mj-actions`. `Detector`, `Optimiser` and `Archiver` traits live in `mj-core` so `mj-actions` receives optimisers by injection (no crate cycle).
- **Why:** every write to a user path goes through one audited code path.

## D-114 Safety classes and hero buckets
- **Choice:** six classes (`reversible`, `rebuildable`, `light_copy`, `lossy_opt_in`, `review_only`, `permanent`); hero bucket Reversible = reversible + rebuildable + light copy, Opt-in = lossy + permanent, Review only = review only. Confirmation is required for `lossy_opt_in` and `permanent`.
- **Why:** matches the three hero segments of Option B while keeping irreversible deletions out of the "reversible" number.

## D-115 Auto-unchecked rows
- **Choice:** `Badges::AUTO_UNCHECK` = confidential, protected, encrypted, signed, labelled, keeper, in-use; `Candidate::normalise_selection` enforces it; protected rows are never actionable.
- **Why:** SAF-07/09 and the B2 "confidential = auto-unchecked" rule; keepers must never be deleted by default.

## D-116 Schedules never contain confirmation-requiring steps
- **Choice:** `schedule_save` / `schedule_from_plan` reject lossy and permanent steps; schedules store opportunities + actions + params, not per-candidate selections.
- **Why:** SAF-11 cannot be satisfied unattended; candidate ids are not stable across scans.

## D-117 `node --test ui/` entry point
- **Context:** Node 22 treats a directory argument as a module path, not a test folder.
- **Choice:** `ui/package.json` (`"type": "module"`, `"main": "test-all.js"`, no dependencies); `test-all.js` imports every `ui/**/*.test.js`.
- **Why:** the gate command from the brief works unchanged; adding a test needs no registration.

## D-118 Wave 1 stubs and safe defaults
- **Choice:** real minimal implementations for: sequential walker, scan orchestration and opportunity building, in-memory journal, guard (allowlist, protected, link checks), protected-path lists, plan CRUD, settings/presets/schedules persistence, tool detection on PATH, folder browser, disk-usage tree. Everything else returns `NotImplemented`. `sync_roots` returns `NotImplemented` until W2-E lands it, and callers must treat that as "unknown" (a checklist warning), never as "not synced".
- **Why:** start-up and the end-to-end smoke path work now; unfinished parts can never pretend to have found nothing.

## D-119 Windows cross-check of the Tauri crate
- **Observation:** `cargo check --workspace --target x86_64-pc-windows-msvc` passes, including `myjanitor`; tauri-build prints "GNU compiler is not supported for this target" because it cannot compile the Windows resource file on Linux. It is a warning in check mode; the real Windows build runs in CI (Wave 4).

## D-120 Adding dependencies during parallel waves
- **Choice:** a workstream adds a new external crate to its own crate's `Cargo.toml` with an exact `=` pin, and appends its row under its own heading in `docs/DEPENDENCIES.md`; the orchestrator hoists it to `[workspace.dependencies]` and regenerates `Cargo.lock` at merge.
- **Why:** no two concurrent workstreams edit the same lines.

## D-121 Cargo.lock seeded from pdf-to-markdown
- **Choice:** the first `Cargo.lock` was copied from the reference repo, then pruned by cargo.
- **Why:** reuses a resolution of Tauri 2.12.3 known to build (the reference hit a semver-compatible breakage once).

## D-122 Bundle targets and CSP
- **Choice:** `bundle.targets = ["appimage", "deb"]` (Linux); Windows ships as a portable exe built with `--no-bundle`, as in the reference. CSP drops `'unsafe-inline'` for styles: the UI sets dynamic sizes through the CSSOM only.
- **Why:** proven packaging path; stricter CSP costs nothing for a vanilla UI.
