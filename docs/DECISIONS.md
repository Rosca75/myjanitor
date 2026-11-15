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

## D-007 Autonomous merge to main (owner amendment)
- **Context:** during the session the owner amended the brief: the session manages commit, push and merge autonomously, the final MVP must land on `main`, and no PR review is requested.
- **Choice:** at the end, open a PR from the working branch and merge it into `main` once every gate is green.

## D-008 Per-workstream decision and dependency files
- **Context:** parallel worktrees appending to `docs/DECISIONS.md` and `docs/DEPENDENCIES.md` would conflict on merge.
- **Choice:** each workstream writes `docs/decisions/<id>.md` and `docs/dependencies/<id>.md`; the root files hold orchestrator and Wave 1 entries and point to those folders.

## D-009 W3-B and W3-C start before Wave 2 exit
- **Context:** the brief gates each wave on the previous one. W2-A (scan) and W2-E (actions) were still running when the other four Wave 2 workstreams had merged.
- **Choice:** start W3-B (PDF, archiving, FS compression) and W3-C (images, media adapters) early.
- **Why:** they own disjoint files and depend only on the frozen Wave 1 contracts, not on W2-A or W2-E output. W3-A, W3-D and W3-E wait for the Wave 2 exit.

## D-010 W3-D split and contract revision
- **Context:** Wave 2 workstreams returned contract change requests (media conditions in the rule tree, detector context roots and protected-path predicate, pixel counts for keep rules, file id in the changed-since-scan check, error codes `needs_elevation`/`confidential`, extra journal fields, cache-aware scan call, confirmation pass-through). The engine facade also decides what gets deleted, so it is data-loss relevant.
- **Choice:** W3-D1 (Opus) applies the contract revision and builds the engine facade and Tauri commands; W3-D2 (Sonnet) builds the derived screens against `docs/IPC.md`.
- **Why:** the brief's rule of thumb assigns code that could lose user data to Opus; UI screens stay with Sonnet.

## D-011 CI toolchain pinned to the development toolchain
- **Context:** the first CI runs failed in clippy: CI pinned Rust 1.92.0, whose clippy reports `nonminimal_bool` on an expression that 1.97.0 (used for every local gate) accepts.
- **Options:** (a) rewrite code to satisfy every clippy version between the MSRV and current; (b) pin CI to the toolchain the gates are run with.
- **Choice:** (b), `RUST_VERSION: 1.97.0` in `.github/workflows/ci.yml`.
- **Why:** lints differ between releases, so one pinned version is the only stable definition of "clippy clean". `rust-version = 1.88` remains the declared minimum for building, not for linting.

## Index of workstream decisions

Each workstream wrote its own file under `docs/decisions/` (D-008). This index was written at the end of the build and refreshed after the Windows CI fixes, the review fixes and the gap-closure pass, from those files and from the code. Where a decision file and the code disagreed, the code was checked; the differences are listed at the end of this section.

| File | IDs | Scope | Most consequential decisions |
|---|---|---|---|
| This file | D-001 to D-122, D-007 to D-014 | Orchestrator, Wave 1 and the later process decisions (CI toolchain, review outcomes, PR structure): layout, contracts, design tokens, process | [D-102](#d-102-lossless-path-codec-on-the-wire-and-in-persisted-files) paths survive non-UTF-8 names exactly; [D-105](#d-105-ipc-style) one command per engine operation with typed error codes; [D-113](#d-113-optimisers-are-pure-transforms) every write to a user path goes through the action engine; [D-013](#d-013-land-a-green-checkpoint-on-main-before-the-second-fix-pass) and [D-014](#d-014-follow-up-pr-after-the-mvp-merge) land the MVP first, then fixes in a second PR |
| [W2-A](decisions/W2-A.md) | D-2A01 to D-2A14 | Scan engine, cache, platform scanning | [D-2A01](decisions/W2-A.md#d-2a01-parallel-walk-shape) level-by-level parallel walk; [D-2A04](decisions/W2-A.md#d-2a04-mft-fast-path--usn-enumeration--file-records) NTFS fast path is USN plus FILE records, discarded on any error; [D-2A06](decisions/W2-A.md#d-2a06-trust-window-anchored-at-the-scan-start) files changed in the last 2 s never hit the cache |
| [W2-B](decisions/W2-B.md) | D-2B01 to D-2B10 | Duplicates and similarity | [D-2B01](decisions/W2-B.md#d-2b01-exact-group-confirmation-is-sha-256-computed-in-the-full-hash-pass) SHA-256 confirmation in the same read; [D-2B04](decisions/W2-B.md#d-2b04-jpegs-without-an-embedded-thumbnail-are-fully-decoded-by-default-deviation) bounded full decode of thumbnail-less JPEGs (deviation from the reference); [D-2B08](decisions/W2-B.md#d-2b08-version-chain-rules) version chains are review-only and conservative |
| [W2-C](decisions/W2-C.md) | D-2C01 to D-2C15 and the query grammar | Filters, rules, presets, quick query | D-2C01 three-valued evaluation (unreadable never matches, even under NOT); D-2C14 an empty query is an error; D-2C06 exclusions apply before the rule |
| [W2-D](decisions/W2-D.md) | D-2D01 to D-2D14 | Simple detectors | [D-2D03](decisions/W2-D.md#d-2d03-selection-policy) pre-select only what nothing the user wrote can be lost to; [D-2D01](decisions/W2-D.md#d-2d01-rules-match-index-paths-not-environment-folders) rules match index paths, not environment folders; [D-2D06](decisions/W2-D.md#d-2d06-empty-folders-confirmed-on-disk) empty folders are re-checked on disk |
| [W2-E](decisions/W2-E.md) | D-2E01 to D-2E09 | Action engine and safety | [D-2E01](decisions/W2-E.md#d-2e01-windows-recycle-bin-goes-to-the-quarantine-by-default) Windows trash goes to the quarantine; [D-2E02](decisions/W2-E.md#d-2e02-saf-11-is-enforced-inside-mj-actions) confirmation is enforced inside the engine; [D-2E04](decisions/W2-E.md#d-2e04-journal-durability) journal synced after every record |
| [W2-F](decisions/W2-F.md) | F-01 to F-11 | UI shell, Overview and Step review | [F-01](decisions/W2-F.md#f-01-views-are-plain-functions-over-one-store-built-with-h-and-no-html-strings) no HTML strings, so file names cannot inject markup; [F-07](decisions/W2-F.md#f-07-one-lossypermanent-dialog-per-step-in-plan-order-apply-at-the-end) one lossy or permanent dialog per step; [F-08](decisions/W2-F.md#f-08-paging-not-virtual-scrolling) paging instead of virtual scrolling |
| [W3-A](decisions/W3-A.md) | D-3A01 to D-3A08 | Optimisation framework, ZIP, XML, Office | [D-3A02](decisions/W3-A.md#d-3a02-zip-writer) in-house ZIP writer; [D-3A03](decisions/W3-A.md#d-3a03-repack-refusals) repack refuses APKs, signed ODF/OOXML and any data outside the entries; [D-3A05](decisions/W3-A.md#d-3a05-ooxml-verification-tolerates-pre-existing-problems-only) only problems not already in the source fail verification |
| [W3-B](decisions/W3-B.md) | D-3B01 to D-3B16 | PDF, archiving, file-system compression | [D-3B01](decisions/W3-B.md#d-3b01-pdf-readerwriter-is-in-house-no-lopdf--pdf) in-house PDF reader and writer; [D-3B02](decisions/W3-B.md#d-3b02-refuse-rather-than-repair) refuse rather than repair; [D-3B11](decisions/W3-B.md#d-3b11-integrity-test-opt-arc-02) archive integrity test cannot be skipped |
| [W3-C](decisions/W3-C.md) | D-3C01 to D-3C09 | Images, media, external tools | D-3C01 only `.exe` and `.com` count as tools on Windows; D-3C03 oxipng output accepted only if pixels match; D-3C08 hardware encoders only after a trial encode |
| [W3-D1](decisions/W3-D1.md) | D-3D101 to D-3D118 | Contract revision, engine facade, Tauri commands | D-3D101 no new fields on structs other crates build literally; D-3D106 file-changing jobs run one at a time; D-3D108 locked rows can never be selected |
| [W3-D2](decisions/W3-D2.md) | D2-01 to D2-08 | Derived screens | [D2-02](decisions/W3-D2.md#d2-02-errors-toast-inline-or-disabled) unavailable features render disabled with the hint; [D2-03](decisions/W3-D2.md#d2-03-history-confirm-before-undoing-a-run-refusals-come-back-through-the-job) confirm before undoing a run; [D2-06](decisions/W3-D2.md#d2-06-size-breakdown-opt-eng-09-and-compare-opt-eng-10-degrade) breakdown and compare degrade when the backend lacks data |
| [W3-E](decisions/W3-E.md) | E-01 to E-08 | Settings, schedules, CLI, context menu, portable | [E-04](decisions/W3-E.md#e-04-schedules) schedules hold reversible steps only; [E-07](decisions/W3-E.md#e-07-cli) CLI refuses dangerous roots and needs flags for lossy and permanent steps; [E-03](decisions/W3-E.md#e-03-side-effects) portable mode never writes outside its folder |
| [W4-INT](decisions/W4-INT.md) | D-4I01 to D-4I11 | Integration pass | [D-4I01](decisions/W4-INT.md#d-4i01-optimise-path-what-was-already-wired-what-was-broken-at-the-seams) three seam bugs fixed (step parameters, converted extensions, dry-run sizes); [D-4I05](decisions/W4-INT.md#d-4i05-engine-modules-schedule_run_now-as-a-job-os-integration-from-the-cli-only) OS integration from the CLI only; [D-4I10](decisions/W4-INT.md#d-4i10-large-tree-performance-item-9) protected-path badging was the hot spot at 200,000 files |
| [W4-CI](decisions/W4-CI.md) | none (one page) | CI workflow | Two jobs (`test`, then `bundle`) on `ubuntu-22.04` and `windows-latest`; bundle list passed on the command line so `tauri.conf.json` is untouched; the Rust pin written there (1.92.0) was replaced by 1.97.0 in [D-011](#d-011-ci-toolchain-pinned-to-the-development-toolchain) |
| [W4-WIN](decisions/W4-WIN.md) | D-W1 to D-W7 | Making the suite pass on Windows | D-W1 verbatim `\\?\` paths are normalised first (a forward slash used to fail or name another file); D-W3 MFT link counts exclude DOS 8.3 aliases (removal estimates promised nothing); D-W2 the 8.3 short-name refusal applies only below the allowed root; no guard or confirmation check was weakened |
| [W5-FIX](decisions/W5-FIX.md) | D-F1 to D-F11 | Review-2 fixes and T1 gap closure | [D-F1](decisions/W5-FIX.md#d-f1-canonical-spellings-are-added-never-substituted-g-01) protected paths are matched in the given and the canonical spelling; [D-F5](decisions/W5-FIX.md#d-f5-selection-digest-instead-of-a-plan-generation-g-04) a confirmation is bound to a digest of the shown selection; [D-F10](decisions/W5-FIX.md#d-f10-the-windows-recycle-bin-setting-never-overrides-a-fixed-route) the Windows Recycle Bin setting never overrides a fixed route, so no test can reach a real Recycle Bin |

### Top decisions for the owner to review

1. **Windows default trash route is MyJanitor's quarantine; the real Recycle Bin is an opt-in setting** ([D-2E01](decisions/W2-E.md#d-2e01-windows-recycle-bin-goes-to-the-quarantine-by-default), [D-F10](decisions/W5-FIX.md#d-f10-the-windows-recycle-bin-setting-never-overrides-a-fixed-route)). The shell API returns no handle for an exact restore. The setting `general.windows_recycle_bin` is off by default, the step wording follows the route in use, and quarantined items are purged automatically after their retention.
2. **F-12 is accepted, not fixed** ([D-012](#d-012-adversarial-review-1-f-12-accepted)). Closing it needs handle-relative operations throughout the action engine. A parent folder swapped by another process in the few system calls between the last check and the act is not caught.
3. **Hard links need a confirmation and can never be scheduled; schedules hold reversible steps only** ([D-116](#d-116-schedules-never-contain-confirmation-requiring-steps), [E-04](decisions/W3-E.md#e-04-schedules), [review-1 fixes, F-07](review/adversarial-review-1-fixes.md)). Two names of one file share edits, and an unattended run cannot give a confirmation.
4. **A confirmation is bound to the selection it was shown** ([D-F5](decisions/W5-FIX.md#d-f5-selection-digest-instead-of-a-plan-generation-g-04)). After a rescan or any change to the rows it is asked again; unchecked rows stay unchecked and an edited file must be reviewed again.
5. **PowerPoint "Remove unused slide layouts" is opt-in, lossy and skips templates** ([review-2 fixes, G-05](review/adversarial-review-2-fixes.md), [D-F6](decisions/W5-FIX.md#d-f6-templates-are-recognised-by-content-type-g-05)). It used to be on by default and could run unattended.
6. **In-house PDF reader and writer, and `zune-jpeg` as the only dependency beyond the ruled stack** ([D-3B01](decisions/W3-B.md#d-3b01-pdf-readerwriter-is-in-house-no-lopdf--pdf), [D-3B02](decisions/W3-B.md#d-3b02-refuse-rather-than-repair), [D-2B04](decisions/W2-B.md#d-2b04-jpegs-without-an-embedded-thumbnail-are-fully-decoded-by-default-deviation)). The PDF pass refuses rather than repairs, so 9 of 34 corpus PDFs are skipped; the JPEG decode deviates from the reference rule of no full-decode fallback.
7. **In-app folder browser instead of the dialog plugin** ([D-104](#d-104-no-dialog-plugin-in-app-folder-browser)). It keeps capabilities at `core:default`, at the cost of not being the native dialog.
8. **Orange accent only for the primary action, blue for selection, light theme only** ([D-108](#d-108-accent-is-orange-for-the-primary-action-only-selection-and-in-plan-are-blue), [D-004](#d-004-light-theme-only)). The reference palette has no dark variant.

### Where decision files and code disagree

Checked against the code when this index was refreshed. The code is right in each case.

- **W4-CI** records the Rust pin as 1.92.0 and the first run as unproven. The workflow pins 1.97.0 (D-011) and has run green on both platforms since.
- **W5-FIX Part B** grades AUT-06 (low-memory mode) as done under AUT-05/06/07. The code only shows the toggle off and disabled, with a reason; `docs/FEATURE-STATUS.md` grades it disabled-with-reason. The same table calls CLN-ACT-02, CLN-ACT-06 and CLN-CNT-01 done; FEATURE-STATUS keeps them partial (the real Recycle Bin route was never exercised, ReFS and real Btrfs reflinks were not tested, and PDF text extraction misses CID-font PDFs).
- **Settings stored but read by nothing:** only `optimise.lossless_only` and `optimise.preserve_ads` still have no effect and no control. `low_memory`, `parallel_jobs` and `cpu_priority` are shown disabled with a reason (D-F9); `threads` and `temp_dir` now work.
- **Earlier disagreements now resolved in the code:** quarantine retention is enforced automatically (CLN-ACT-04), the temporary folder setting is used for scratch folders, and the `miniz_oxide` "same version as Tauri's PNG stack" remark in DEPENDENCIES.md was corrected.
- **Adversarial reviews.** Review 1 (20 findings) and review 2 (14 new findings and 5 partly closed items) are resolved: all fixed except F-12, accepted in D-012. No third review was run on the review-2 fixes.

## D-012 Adversarial review 1: F-12 accepted
- **Context:** review 1 found 20 issues in destructive paths (`docs/review/adversarial-review-1.md`). 19 are fixed with regression tests (`docs/review/adversarial-review-1-fixes.md`).
- **Accepted:** F-12 (Low): check-then-act by path without directory-handle anchoring; a parent directory swapped between the last re-check and the act is not caught.
- **Why accepted:** closing it needs handle-relative operations (`openat`/`NtCreateFile` with a root handle) throughout the action engine, a rewrite too large for this pass. Mitigations in place: the guard check is repeated immediately before acting, links are never followed, file identity and folder stamps are compared, and the window is a few system calls wide. Exploiting it requires another process on the same machine racing the user's own action inside the user's own folders.
- **Also partly accepted:** F-09: cross-volume copies on Windows do not carry alternate data streams or ACLs (documented in README limitations).

## D-013 Land a green checkpoint on main before the second fix pass
- **Context:** the owner requires the MVP to land on `main`. Usage limits interrupted the session five times, each for several hours. CI is green on both platforms. Review 2 left open findings: G-01 and G-02 (High), G-03 to G-05 (Medium), nine Lows, and five partly closed review-1 items.
- **Options:** (a) merge only after the second fix pass; (b) merge the green checkpoint now and follow with a second PR.
- **Choice:** (b).
- **Why:** an interruption during the fix pass would otherwise leave nothing on `main`. The open findings are documented in `docs/review/adversarial-review-2.md` and tracked in the follow-up; both High findings need unusual conditions (a symlinked scan root or a protected folder registered under a different path spelling; files modified during a cross-volume move).

## D-014 Follow-up PR after the MVP merge
- **Context:** the MVP was merged into `main` as PR 1 (merge commit `92ba392`) with the review-2 findings still open, as decided in D-013. CI was green on both platforms for that state.
- **Options:** (a) keep fixing on the same pull request; (b) a second PR from the follow-up branch.
- **Choice:** (b). PR 2 carries the review-2 fixes (G-01 to G-14 and the five partly closed review-1 items), the T1 gap closure (`docs/decisions/W5-FIX.md`, Part B) and this documentation refresh.
- **Why:** PR 1 had to land first so an interruption could not leave `main` empty, and a separate PR keeps the fixes reviewable on their own.
- **Verification:** the review-2 fixes were verified by their regression tests (825 Rust and 214 UI tests) and by the Linux and Windows-target clippy gates, not by a third adversarial review. The new tests that are not Unix-gated will run on `windows-latest` for the first time in the CI run of PR 2.

## D-015 A pinned trash route is explicit
- **Context:** Windows CI showed that with the `windows_recycle_bin` setting on, the engine switched to the real Recycle Bin even when the caller had fixed the route to the quarantine: on Windows the quarantine is also the OS default, so "fixed by the caller" could not be told apart from "default".
- **Choice:** `EngineConfig.trash_route` is `Option<TrashRoute>`: `None` follows the OS default and the setting; `Some(route)` pins it and no setting can change it. Tests pin the quarantine.
- **Why:** user data safety: a caller (or test) that pins a route must never have removals redirected to another place.

## D-016 Long text in PDF exports is cut in the middle
- **Context:** exported PDF tables cut long paths at the end, dropping the file name (seen on Windows, where temp paths are long).
- **Choice:** cut in the middle (`head...tail`), keeping the end of the path.
