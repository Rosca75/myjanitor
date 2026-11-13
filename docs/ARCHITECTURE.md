# MyJanitor architecture

**Frozen in Wave 1** (contracts: `crates/core`, the platform traits,
`crates/engine/src/api`, `docs/IPC.md`, `ui/styles/tokens.css`, `ui/js/api.js`).
Changes to frozen files go through the orchestrator. Decisions referenced as
D-xxx are in `docs/DECISIONS.md`; dependencies in `docs/DEPENDENCIES.md`.

## 1. Overview

MyJanitor is a Rust + Tauri 2 desktop app (Windows 10/11 x64, Linux x64) that
finds files to clean, makes files smaller, and is safe by construction:
estimate first, reversible by default, lossy only on opt-in, fully offline.

Two front ends share one engine:

```text
 ui/ (HTML/CSS/ES modules)          myjanitor-cli (crates/cli)
   └ js/api.js  (only bridge)             │
        │ invoke / listen                 │ direct calls
 src-tauri (myjanitor, thin wrappers)     │
        └──────────────┬──────────────────┘
                   mj-engine  (session, orchestration, command + event API)
     ┌──────┬──────┬───┴────┬────────┬──────────┬──────────┐
  mj-scan mj-dupes mj-filters mj-detect mj-actions mj-optimise
     │      │       │          │        │          │
     └──────┴───────┴─── mj-formats ────┴──────────┘
     └────────────── mj-platform (traits + FFI) ───┘
                        mj-core (types, errors, pure utilities)
```

## 2. Crates and dependency graph

| Crate (package) | Path | Depends on | Role |
|---|---|---|---|
| `mj-core` | `crates/core` | serde | Domain types (the contract), error model, xxHash64, SHA-256, calendar maths, glob, lexical paths + lossless path codec, size formatting, thread pool, cancel/pause token, progress sink, `testutil` (feature) |
| `mj-platform` | `crates/platform` | core | `ScanPlatform` / `ActionPlatform` traits, `NativePlatform`, hand-written FFI |
| `mj-formats` | `crates/formats` | core, miniz_oxide | ZIP, XML, OOXML, PDF, PNG, JPEG, EXIF, CFB, media tags, CRC-32, content sniffing |
| `mj-scan` | `crates/scan` | core, platform | Walker, index building, scan cache |
| `mj-dupes` | `crates/dupes` | core, platform, formats | Exact duplicates, similar images, version chains, derived files, attachment copies, duplicate fonts |
| `mj-filters` | `crates/filters` | core, formats, regex | Rule evaluation, quick-query parser, presets, type groups, content search |
| `mj-detect` | `crates/detect` | core, platform, formats | Simple detectors |
| `mj-actions` | `crates/actions` | core, platform, formats, serde_json | Action engine, guard, journal, undo, quarantine, safety inspection, keep-rule selection, report writers |
| `mj-optimise` | `crates/optimise` | core, platform, formats | Optimisers, archiver, external tool detection and process runner |
| `mj-engine` | `crates/engine` | all of the above, serde_json | `Engine` facade + `api` (commands, events, types) |
| `mj-cli` (bin `myjanitor-cli`) | `crates/cli` | engine | CLI with in-house argument parsing |
| `myjanitor` (lib `myjanitor_lib`) | `src-tauri` | engine, tauri | GUI shell: one `#[tauri::command]` per engine command, events forwarding |

Rules:

- The dependency direction is strictly downward in the table above. No crate
  depends on `mj-engine` except the two front ends; `mj-actions` never depends
  on `mj-optimise` (optimisers are injected as `&[Box<dyn Optimiser>]`).
- `#![forbid(unsafe_code)]` everywhere except `mj-platform` (FFI) and
  `src-tauri`. Every `unsafe` block carries a `// SAFETY:` comment.
- No `windows`, `windows-sys`, `libc`, `nix`, `clap`, `chrono`, `glob`, `rayon`,
  `tempfile`. Allowed externals: tauri stack, serde, serde_json, miniz_oxide,
  regex; anything else needs a `docs/DEPENDENCIES.md` entry first (§7 of the brief).
- Exact pins (`=`), `Cargo.lock` committed, `panic = "unwind"` in release
  (panics are isolated with `catch_unwind`).

## 3. Data flow

```text
ScanScope ─► mj_scan::scan ─► FileIndex ─┬─► Detector::run ─────┐
 (roots,      (walk / MFT,    (sorted by  ├─► Optimiser::analyse ┼─► DetectorOutput
  includes,    stat, cache)    path_key)  └─► mj_filters::select ┘   (Candidates, Groups, Diagnostics)
  excludes)                                                         │
                                  mj-engine: badges (SafetyInspector), ids, auto-uncheck
                                                                    ▼
                                                     OpportunityRecord (card + candidates + selection)
                                                                    │  user: Review / + Add / select / params
                                                                    ▼
                                                        Plan ─ PlanStep (opportunity + Action + params)
                                                                    │  dry run / confirmation (SAF-11)
                                                                    ▼
                                         ActionItem[] (selected, actionable, with scan stamps)
                                                                    │  job queue (one job per step)
                                                                    ▼
                                       mj_actions::execute ─► JournalEntry (intent → done/failed)
                                                                    │
                                                         Report, History, Undo (mj_actions::undo)
```

1. **Scope** (`ScanScope`): roots, include/exclude globs, options. Roots are
   normalised; nested roots collapse into their parent. Global exclusions from
   `Settings::safety::exclusions` are added at scan time.
2. **Scan** (`mj_scan::scan`): produces a `FileIndex` whose entries are sorted
   by `path_key`, so every subtree is a contiguous range (folder totals,
   children, topmost match are binary searches). Links/junctions are recorded
   as `EntryKind::Link` and never followed. Every unreadable entry is counted
   in `ScanStats` with a bounded sample.
3. **Find**: each `Detector` (mj-dupes, mj-detect), each `Optimiser::analyse`
   (mj-optimise) and custom searches (mj-filters) read the index through a
   `DetectContext` (index, params, fixed `now`, fingerprint store, cancel,
   progress, threads) and return a `DetectorOutput`. They never modify files and
   must report what they could not cover in `Diagnostics`.
4. **Rank**: the engine turns each output into an `OpportunityRecord`: assigns
   candidate and group ids, adds path badges (protected, synced, online-only,
   encrypted attribute) from `mj_actions::SafetyInspector`, applies the
   auto-uncheck rule (`Badges::AUTO_UNCHECK`), and computes the card numbers
   (`opportunities::recompute`). Default sort: biggest saving first.
5. **Plan**: steps reference opportunities; a step carries the action, the
   parameters, an estimate and its `SafetyClass`. The checklist is computed on
   every change.
6. **Apply**: `plan_apply` checks confirmations, expands each step into
   `ActionItem`s (selected, non-blocked candidates with their scan stamp and
   root), allocates a `RunId`, writes a `RunRecord`, and queues one job per
   step. Jobs run `mj_actions::execute` with a `Guard` whose allowed roots are
   the scope roots.
7. **Journal / undo**: every item is journaled before and after acting;
   History lists runs; undo restores entries in reverse order.

## 4. Threading model

- **No async in the engines.** Tauri's async runtime is used only at the IPC
  boundary: every command wrapper is `async` and runs the engine method on
  `tauri::async_runtime::spawn_blocking`, so nothing runs on the UI thread.
- **Engine methods are fast.** Long operations (`spawns: true` in
  `api::COMMANDS`: scan, dry run, apply, undo, purge, scheduled runs) take
  `self: &Arc<Engine>`, start one dedicated thread with a 16 MiB stack
  (`mj_core::pool::spawn_large`), return immediately (a generation or a `Job`)
  and report through events.
- **Fan-out** inside an operation uses `mj_core::pool::parallel_map`: scoped
  threads with an atomic index counter writing into index-aligned result slots
  (no per-item lock), a cancel/pause checkpoint before every item, and
  `catch_unwind` per item so one hostile file cannot kill a worker. Worker
  count = `Settings::performance::threads` (0 = logical CPUs). Full-file reads
  on network filesystems use the adaptive fallback (parallel, then serial
  retry) from the research file; bounded prefix reads stay parallel.
- **Persistent workers** (job queue) use `mj_core::pool::ThreadPool`
  (channel-fed, large stacks, panics caught). Jobs of the same volume run one
  at a time; `parallel_jobs` limits optimisation jobs across volumes.
- **Cancellation and pause**: one `CancelToken` per scan / job.
  `checkpoint()` blocks while paused and returns `Err(Cancelled)` once
  cancelled. Destructive work checks it between items, never inside one.
- **Generations**: every scan increments a generation; results and events
  carry it; a finished scan publishes its results only if its generation is
  still current, so a stale run can never overwrite a newer one.
- **Progress**: engines report to a `ProgressSink`; the engine wraps sinks in
  `ThrottledSink` (50 ms, final and phase-change updates always pass) before
  emitting `scan://progress` / `job://progress`.
- **Locks**: `std::sync::Mutex`/`RwLock` via `mj_core::lock` (poison-tolerant).
  Never hold a lock across I/O or while emitting an event.
- **Panic isolation**: every detector, optimiser and action item runs inside
  `catch_unwind`; a panic becomes an `Error::Internal` for that item/output.
- **Allocator**: system allocator for now; measure mimalloc on Windows in
  Wave 4 if profiles show allocator contention (lessons file rule 13; would
  need a DEPENDENCIES.md entry).

## 5. Error model

- One enum, `mj_core::Error`, with stable `code()`s and actionable `hint()`s;
  `Result<T>` alias everywhere. `Error::io(path, err)` maps `NotFound` and
  `PermissionDenied` to their own variants.
- On the wire: `ErrorInfo {code, message, hint, path?}` (IPC.md §1.1). Command
  failures reject the promise with it; per-item failures live in
  `ItemOutcome.error`, journal entries and reports, so one bad file never
  fails a batch.
- `NotImplemented` = owner has not landed it (UI shows the feature disabled);
  `Unsupported` = not possible here (platform/filesystem), shown disabled with
  the reason. Neither may ever be reported as "nothing found".
- Parsers return `Error::Parse` on malformed input and never panic; recursion
  over file-derived data has explicit depth limits; quadratic steps have caps;
  sorts over floats use `total_cmp`.

## 6. Scan cache (SCAN-04), adapted from dedup-photos

| Aspect | Design |
|---|---|
| Store | **One global store** `<cache dir>/scan-cache.bin` (D-106), keyed by `path_key` (lexically normalised, `/` separators, case-folded on Windows). Overlapping roots share entries; no per-root merge logic. |
| Validity | Stamp `{size: u64, mtime_ns: i64}` compared by **exact equality**. An entry whose mtime is within 2 s of the scan start, or in the future, is not trusted (still being written / clock skew). Optional `FileId` (Linux `st_dev`+`st_ino`) carries fingerprints across renames when `(id, size, mtime)` match. |
| Contents | `mj_core::Fingerprints`: `partial`, `full` (xxh64), `sha256`, `dhash`, `phash`, `width`, `height`, `exif` (`Some` only when really captured), `flags` (`unreadable`, `no_thumbnail`, `from_thumbnail`, `decode_failed`). Every stage is an explicit `Option`; **no zero sentinels** (D-006). Derived scores (quality score) are never cached. |
| Versions | `format_version` (layout) and `algo_version` = `FINGERPRINT_ALGO_VERSION` (meaning: window layout, seed, grid, downsampler). Any mismatch = empty cache. Bump `algo_version` whenever the meaning of a stored value changes, not only its shape. |
| Format | In-house little-endian binary, documented in `crates/scan/src/cache/format.rs`: magic `MJCACHE\0`, versions, entry count, records with a presence bitmask then only present fields, trailer = xxh64 of everything before it. |
| Load | Whole file into memory at engine start; bad magic, version, trailer or any parse error → empty cache, one log line, never fatal. |
| Save | After a completed scan (and after hash stages finish): write `<file>.tmp` in the same directory, `sync_all`, rename over the target (atomic). Cache write failures are ignored. |
| Pruning | On save, drop entries under a scanned root that the completed scan did not see, and entries under roots that no longer exist (D-006). |
| Lookup | `FingerprintStore::get(path, stamp)` returns data only on an exact stamp match; `put` merges stages under the same stamp and replaces on a different stamp, so later scans add stages lazily (e.g. a full hash to an entry that only had a partial). |
| Staged hashing | Size buckets across **all** files (cache hits included, D-006); partial = xxh64 over head + middle + tail windows of 64 KiB (D-005), whole file when ≤ 192 KiB (stored as `full` too); full hash streamed in 1 MiB buffers for partial collisions only; SHA-256 (or byte compare) confirmation of a group before deletion is offered; files sharing a `FileId` are hard links, not duplicates. |
| Diagnostics | Every failed open/read/decode counted (`Diagnostics::unreadable`, `skipped["no_thumbnail"]`, ...); the UI must say "N files could not be compared". |

Perceptual matching follows research §4-§7 unchanged: box-averaged grids,
dHash 9x8 and pHash 32x32 from the same decode, both always stored, algorithm
chosen at match time, threshold percent capped at 18 % (11 bits) and converted
once, BK-tree + Union-Find candidates, leader refinement (scan limit 256),
confidence from the group diameter, deterministic ordering, no aspect-ratio
bucketing, no full-decode fallback for files without an embedded thumbnail.

## 7. Safety model

### 7.1 Safety classes

| `SafetyClass` | Meaning | Hero bucket | Confirmation | Undo |
|---|---|---|---|---|
| `reversible` | Trash, quarantine, move, copy, links, dehydrate, FS compression, lossless rewrite with recoverable originals | Reversible | no | yes |
| `rebuildable` | Regenerable content (dev artefacts, caches) removed via Trash/quarantine | Reversible | no | yes |
| `light_copy` | A smaller copy beside the original; nothing removed | Reversible | no | yes (delete the copy) |
| `lossy_opt_in` | Lossy transform; originals follow the chosen policy | Opt-in | **yes** | originals restorable unless policy = delete |
| `review_only` | No automatic action proposed | Review only | n/a | n/a |
| `permanent` | Permanent delete, shred, wipe | Opt-in | **yes** (danger button) | no |

`Action::safety(lossy)` derives the class of a concrete action; optimisers
declare the class of their defaults; a preset or toggle marked `lossy` moves a
step to `lossy_opt_in`. Schedules may only contain steps that need no
confirmation (D-116).

### 7.2 Rules enforced inside `mj-actions` (not in the UI)

1. **Root allowlist**: `Guard::new(allowed_roots, protected)` with the scope
   roots. Any path outside them → `outside_allowed_roots`. A root itself is
   never acted on. Paths must be absolute without `..`.
2. **Protected paths** (SAF-01): built-in OS folders and patterns
   (`mj_actions::protected`, shown locked) plus the user's list. Protected
   candidates are shown with the `protected` badge, never actionable.
3. **Never follow links**: every ancestor between the allowed root and the
   item is lstat'd; a symlink, junction or mount point on the path →
   `link_not_followed`. The item itself may be a link only for removal of the
   link (broken-link detector). Folder removals walk without following links.
4. **TOCTOU guard**: right before acting, re-stat and compare with the scan
   stamp (`ActionItem::expected`); mismatch → item `skipped` with `changed`.
5. **Journal before act**: write the `intent` record and `sync_data` before
   touching the file; write `done`/`failed` after. Crash recovery settles
   dangling intents at start-up by inspecting both locations.
6. **Writes are copy-then-rename**: optimised outputs go to a temp name in the
   same folder, are verified (opens, valid structure), kept only if smaller
   (OPT-ENG-04, cannot be disabled), then swapped in; the original goes to the
   originals policy destination first. Timestamps and permissions are restored
   per settings; Windows ADS preserved when the setting is on.
7. **Never overwrite**: restore fails if the original path is occupied;
   exports and light copies use ` (n)` suffixes; Windows reserved names are
   avoided on every platform.
8. **Confidential files** (SAF-07/09): signed, sensitivity-labelled and
   encrypted files are badged and auto-unchecked; optimisers skip them
   (repacks preserve labels, SAF-08; no conversion to 97-2003 formats).
9. **Synced folders** (SAF-10): removal under a sync root is flagged in the
   confirmation dialog with the online-only alternative. If sync roots cannot
   be detected the engine says *unknown*, never "not synced".
10. **Elevation** (SAF-05): never requested by default; Linux never elevates.
11. **Tests** use only `mj_core::testutil::TempDir` trees they create; the
    action engine refuses any path outside the roots the test passes.

### 7.3 Journal format (frozen)

- Directory `<data dir>/journal/`.
- `entries-YYYY-MM.jsonl`: one complete `JournalEntry` JSON object per line,
  append-only. The **last line for an `id` wins** on replay. A torn final line
  (invalid JSON) is ignored. `sync_data` after every `intent` line and at the
  end of each item.
- `runs.jsonl`: `RunRecord` snapshots (same last-wins rule).
- Ids: `RunId` = start time in ms (bumped past the newest existing id);
  `EntryId` = `run_id * 1_000_000 + item index` (unique across sessions).
- Retention: entries older than `undo_days` are not undoable
  (`undo_deadline`) but stay for the audit log; quarantine purge marks entries
  `purged`. Paths use the lossless codec.
- Everything needed to undo: `original` + `original_stamp`, `retained_at`
  (quarantine / move / backup), `trash` ticket, `created` files, `link_source`,
  `reversible`, `undo_deadline`. Audit fields: `bytes_before/after`,
  `original_sha256`, `lossy`, `error`.

### 7.4 Quarantine

`<quarantine dir>/<run id>/<entry id>/<file name>` (default
`<data dir>/quarantine`; same volume preferred so the move is a rename).
Auto-purge after `quarantine_days`.

## 8. Platform abstraction

- `ScanPlatform` (read side, W2-A): `metadata` (lstat semantics), `read_dir`,
  `volumes`, `volume_of`, `fast_enumerate` (NTFS MFT, `Unsupported` elsewhere),
  `cloud_info`.
- `ActionPlatform` (write side, W2-E / W3-B): `trash`, `restore_trashed`,
  `hardlink`, `reflink`, `dehydrate`, `hydrate`, `sync_roots`, `is_elevated`,
  `fs_compress`, `fs_decompress`, `fs_compression`.
- `Platform` = both; `NativePlatform` implements them by delegating to free
  functions in `scan/*` and `action/*` so owners only touch their own files.
  Tests may implement the traits with fakes (e.g. failing trash mid-batch).
- Wave 1 fallbacks: std-only `metadata` (Linux: blocks, ino, dev, nlink, uid,
  ctime; Windows: attributes and FILETIMEs) and `read_dir`, std hard links,
  elevation via `geteuid`. Everything else returns `NotImplemented` (owner
  pending) or `Unsupported` (not possible here).
- FFI: hand-written `extern "system"` (kernel32, shell32, cldapi, advapi32...)
  and `extern "C"` (ioctl, xattr) declarations. Common declarations are in
  `ffi/windows.rs` and `ffi/linux.rs` (frozen); workstream-specific ones live
  next to their code. Windows paths go through `ffi::windows::wide_path`
  (adds `\\?\`).
- Platform-specific features stay visible everywhere and are disabled with a
  reason (`Availability::Disabled`), e.g. "Windows only".

## 9. Frontend

- `ui/` is served as-is (`frontendDist: "../ui"`, `withGlobalTauri: true`, no
  bundler, no npm runtime dependency). Proven by the Wave 1 spike: `cargo build
  -p myjanitor` embeds it.
- `ui/js/api.js` is the only module touching `window.__TAURI__`; it exposes
  `api.<camelCase>(req)` for every command and `api.on(event, cb)`. Without
  Tauri (plain browser, `?mock=1`, `node --test`) it loads
  `ui/js/mock/backend.js`.
- CSP: `default-src 'self'; style-src 'self'; img-src 'self' data:; connect-src
  'self' ipc: http://ipc.localhost`. No inline scripts, no inline `style`
  attributes (set dynamic sizes through the CSSOM, `el.style.setProperty`).
- Colours only in `ui/styles/tokens.css` (a test enforces it). Light theme
  only (D-004). One scroll container per view.
- Tests: `node --test ui/` (the `ui/package.json` main `test-all.js` imports
  every `*.test.js`, D-117). Render tests build DOM against the mock; wiring
  tests assert the command names and payloads sent through `api.js`.

## 10. Build, test and conventions

- Gates (every merge): `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build
  --workspace`, `cargo check --workspace --target x86_64-pc-windows-msvc`,
  `node --test ui/`.
- Tests live next to the code (`#[cfg(test)] mod tests`) and in `tests/`
  folders of each crate. Fixture trees: `mj_core::testutil::TempDir`
  (dev-dependency feature `testutil`); `testutil::snapshot` compares trees for
  "state before == state after undo".
- Icons: `python3 tools/make_icons.py` regenerates `src-tauri/icons/`.
- Packaging (Wave 4): Linux AppImage + deb (`bundle.targets`), Windows
  portable exe (`--no-bundle`) as in the reference repo.

## 11. File ownership

A workstream may create and edit only the files it owns. It may add
sub-modules **below** its own module (e.g. `walker.rs` declaring `mod
parallel;` in `walker/parallel.rs`) without touching a parent file it does not
own. New external dependencies: add to your crate's `Cargo.toml` with an exact
`=` pin and append an entry under your workstream's heading in
`docs/DEPENDENCIES.md` (D-120); the orchestrator reconciles `Cargo.lock` on
merge. Contract changes (anything frozen) are requested from the orchestrator.

| Owner | Owns |
|---|---|
| **W1 / orchestrator (frozen)** | `Cargo.toml` (root), `rustfmt.toml`, `.gitattributes`, `.gitignore`, `crates/core/**`, `crates/platform/src/{lib,traits,native}.rs`, `crates/platform/src/ffi/**`, `crates/formats/src/{lib,crc32}.rs`, `crates/formats/src/zip/mod.rs`, `crates/optimise/src/lib.rs`, `crates/engine/src/{lib,engine,store,tests}.rs`, `crates/engine/src/api/**`, `docs/ARCHITECTURE.md`, `docs/IPC.md`, `ui/styles/tokens.css`, `ui/js/api.js` (generated command list), `ui/js/api.test.js`, `ui/test-all.js`, `ui/package.json`, `src-tauri/{Cargo.toml,build.rs,tauri.conf.json}`, `src-tauri/capabilities/**`, `src-tauri/icons/**`, `tools/make_icons.py` |
| **W2-A** scan | `crates/scan/**` (keep `ScanRequest`, `scan`, `effective_roots`, `ScanCache` API), `crates/platform/src/scan/**` |
| **W2-B** duplicates | `crates/dupes/**` (keep `detectors()` and the detector structs), `crates/formats/src/{jpeg,exif}.rs` |
| **W2-C** filters | `crates/filters/**` (keep `compile`, `select`, `parse`, `suggest`, `presets::builtin`), `crates/formats/src/sniff.rs` |
| **W2-D** detectors | `crates/detect/**` (keep `detectors()` and the detector structs) |
| **W2-E** actions + safety | `crates/actions/**` (keep the public API of `engine`, `guard`, `journal`, `protected`, `safety`, `select`, `undo`, `quarantine`, `report`), `crates/platform/src/action/{mod,trash,links,cloud,elevation}.rs`, `crates/formats/src/zip/read.rs`, `crates/formats/src/cfb.rs` |
| **W2-F** UI shell | `ui/index.html`, `ui/styles/*.css` except `tokens.css`, `ui/js/**` except `api.js` / `api.test.js` (owns `ui/js/mock/**`, `ui/js/main.js`, components, views), new `ui/**/*.test.js` |
| **W3-A** optimisation framework, ZIP, OOXML | `crates/optimise/src/{framework.rs,zip_repack.rs}`, `crates/optimise/src/office/**`, `crates/formats/src/zip/write.rs`, `crates/formats/src/xml.rs`, `crates/formats/src/ooxml/**` |
| **W3-B** PDF, archiving, FS compression | `crates/optimise/src/{pdf,archive,fs_compression}.rs`, `crates/formats/src/pdf/**`, `crates/platform/src/action/fs_compress/**` |
| **W3-C** images, media, tools | `crates/optimise/src/{tools,external}.rs`, `crates/optimise/src/images/**`, `crates/optimise/src/media/**`, `crates/formats/src/png.rs`, `crates/formats/src/media/**`, `crates/engine/src/tools.rs` |
| **W3-D** IPC wiring + derived screens | `src-tauri/src/**`, `crates/engine/src/{app,scope,fsbrowse,scan_ops,opportunities,results,steps,search,plan,apply,jobs,history,reports,disk}.rs`; UI files under `ui/js/views/{history,report,queue,search,disk}/**` (created in Wave 3) |
| **W3-E** settings, schedules, CLI | `crates/cli/**`, `crates/engine/src/{settings,schedules,presets,paths}.rs`; UI files under `ui/js/views/settings/**` (created in Wave 3); OS integration code (context menu) in new files under `crates/engine/src/settings/` |

Living documents (`docs/PLAN.md`, `docs/STATUS.md`, `docs/FEATURE-STATUS.md`,
`docs/DECISIONS.md`) are edited by the orchestrator; workstreams append their
decisions to `docs/DECISIONS.md` using their own id range (W2-A: D-2Axx,
W2-B: D-2Bxx, ... W3-E: D-3Exx) so appends never collide.
