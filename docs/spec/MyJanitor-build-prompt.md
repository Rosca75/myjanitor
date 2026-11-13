# Build MyJanitor: autonomous build brief

You are the **orchestrator** of a long, autonomous Claude Code session. Build **MyJanitor**, a Rust/Tauri desktop app that cleans files and makes them smaller, in the repository `myjanitor`. Nobody can answer questions during this session. When something is unclear, decide using the rules in §4, write the decision down, and keep going.

---

## 1. Mission

Deliver a working, tested, cross-platform (Windows + Linux) desktop application that:

1. **Cleans**: finds the right files to remove using filters and smart detectors.
2. **Optimises**: makes files smaller (images, Office, PDF, archives, file-system compression, and media through optional external tools).
3. **Is safe by construction**: estimate first, reversible by default, lossy actions only when the user opts in, fully offline, no telemetry.

The UI follows **wireframe Option B ("Guided plan")**. The feature set comes from the feature specification. The codebase uses as few external libraries as practical (§7).

---

## 2. Inputs

### 2.1 In the `myjanitor` repo (read these first)

| Input | Expected path | Fallback |
|---|---|---|
| Feature specification | `docs/spec/file-cleaner-optimiser-features.md` | Search the repo for `file-cleaner-optimiser-features.md` |
| Wireframe source | `docs/design/MyJanitor-wireframes.figcraft` | See §2.3 |
| Wireframe preview (whole page) | `docs/design/Option B.png` | Frames in `docs/design/option-b-frames/` |
| Individual frames + annotation notes | `docs/design/option-b-frames/*.png` (`B1-overview-plan.png`, `B2-step-review-lossy-confirm.png`, `B1-note-*.png`, `B2-note-*.png`) | The summary in §8.2 |

If a file is not at its expected path, search the whole repo for its name before using a fallback.

### 2.2 Reference repositories (**read-only**: never modify, commit to or push to them)

| Repo | What to extract | Output file |
|---|---|---|
| `pdf-to-markdown` | A small Rust/Tauri app by the same author. Collect **pitfalls and conventions**: Tauri version and config, capabilities/permissions, how the frontend loads without a bundler, IPC patterns, build/bundle settings, Windows and Linux packaging problems, test layout, anything fixed after a bug (read `git log` for "fix" commits). | `docs/research/pdf-to-markdown-lessons.md` |
| `dedup-photos` | **Cache design** (what is keyed, how it is invalidated, storage format, rescan speed) and **fuzzy duplicate matching** (staged hashing, perceptual hashing, thresholds, grouping, choosing which copy to keep). | `docs/research/dedup-photos-cache-and-matching.md` |
| `doc-anonymiser-for-.NET` | The **colour palette**: every colour with exact hex values and its role (primary, surface, text, border, success, warning, danger, info, plus dark variants if present). | `docs/research/palette.md` |

If a reference repo is not accessible, record that in `docs/DECISIONS.md` and continue: use sensible defaults (Tauri 2 stable, a cache keyed by path + size + mtime, a neutral palette with a blue accent).

### 2.3 Reading the `.figcraft` file

A `.figcraft` file is a **ZIP archive containing `document.json`** (the node tree: pages, frames, positions, sizes, fills, text and font properties). Extract it with `unzip` or Python's `zipfile`. Use **only the page named "Option B — Guided plan"** and ignore Options A and C. Produce `docs/design/option-b-spec.md`: screens, layout grid and measurements, components and their states, all on-screen copy, and the annotation notes (frames named `Note / …`, which map UI elements to feature IDs). If parsing fails, work from the PNGs and §8.2.

---

## 3. Non-negotiable constraints

1. **Stack:** Rust + Tauri 2 (stable). Use the Tauri minor version from `pdf-to-markdown` unless its lessons file says to move off it.
2. **Platforms:** Windows 10/11 (x64) and Linux (x64; Ubuntu 22.04+ and Fedora as the reference distros). Every feature either works on both platforms or is clearly marked as platform-specific in the UI and in `docs/FEATURE-STATUS.md`. The spec's macOS mentions are out of scope; record them as "not targeted".
3. **App name:** `MyJanitor` everywhere: window title, bundle identifier (`com.myjanitor.app`), binaries (`myjanitor` GUI, `myjanitor-cli` CLI), config/cache directories.
4. **Repo:** all work happens in `myjanitor`. Reference repos are read-only.
5. **Minimal dependencies:** follow the rule in §7.
6. **Offline:** no network calls at runtime, no telemetry, no update checks.
7. **Autonomy:** never stop to ask. Decide, log the decision, continue.
8. **Session safety:** never run a destructive operation (delete, move, recompress, compress, shred) on any real directory in the session environment. Tests and manual checks only use temporary fixture trees that they create themselves (§10.3).

---

## 4. Autonomous decision rules

When the spec, design or code leaves a choice open, decide with this precedence order (higher wins):

1. **User data safety.** Reversible over irreversible, skip over guess, report over act.
2. **Spec principles** (spec §8): estimate first, reversible by default, progressive disclosure, one context panel, clear badges, persistent savings counter.
3. **Option B design** (layout, interaction model, visual language).
4. **Conventions from the reference repos** (`pdf-to-markdown` lessons, `dedup-photos` algorithms).
5. **Simplicity:** the smallest design that fully meets the requirement.

Rules:
- Record every non-trivial decision in `docs/DECISIONS.md`: one entry per decision, with *context → options → choice → why*.
- If a feature can't be built properly in this session (missing platform API, needs a heavy codec, out of time), **don't fake it**. Leave it visible in the UI as disabled with a one-line reason (for example "Requires ffmpeg on PATH" or "Windows only"), and mark it in `docs/FEATURE-STATUS.md`.
- Prefer finishing a smaller set of features completely (engine + UI + tests) over starting many.

---

## 5. Orchestration model

### 5.1 Your role as orchestrator

- Plan, split the work, brief subagents, review their results, integrate, and keep the project status up to date. **Delegate most implementation.** Write code yourself only for integration glue and small fixes.
- Protect your context: don't read large files end to end. Rely on subagent summaries and the status files. Use targeted searches when you need specifics.
- Keep these living documents current. They are the session's memory and survive context compaction:
  - `docs/PLAN.md`: waves, workstreams, owners, state (todo / in progress / done / blocked).
  - `docs/STATUS.md`: short running log; add a timestamped line whenever a workstream finishes or a decision changes the plan.
  - `docs/DECISIONS.md`, `docs/DEPENDENCIES.md`, `docs/FEATURE-STATUS.md` (see below).

### 5.2 Model assignment

| Model | Use for |
|---|---|
| **Opus 5.5** (`opus` / `claude-opus-5-5`) | Architecture and contracts; scan engine and cache; platform FFI (NTFS MFT, WOF, Recycle Bin, placeholders, reflinks); duplicate and similarity matching; version-chain fuzzy matching; the action engine with journal/undo/quarantine; OOXML (PowerPoint/Excel/Word) optimisation; PDF structural optimisation; integration; security and correctness reviews of destructive code paths. |
| **Sonnet 5.5** (`sonnet` / `claude-sonnet-5-5`) | Reference-repo research; design extraction; feature-matrix bookkeeping; metadata filters and the query parser; simple detectors; frontend screens and components; settings, scheduling and CLI; adapters for external tools; tests for already-specified behaviour; CI; documentation. |

Rule of thumb: if a mistake could **lose user data**, or the task needs **new algorithm or format-parsing design**, use Opus. Otherwise use Sonnet.

### 5.3 Parallelism and isolation

- Run independent workstreams **in parallel**, up to about 5 subagents at once. Give each one an isolated git worktree (`isolation: "worktree"`) when it writes code.
- **File ownership:** each brief lists the files and directories the subagent owns. Two concurrent subagents never own the same file. Shared contracts (types, IPC) are frozen in Wave 1. Later changes go through you.
- Merge finished worktrees into the working branch one at a time. Run the quality gates (§10.1) after each merge.

### 5.4 Subagent brief template

Every brief must stand on its own, because subagents don't see this conversation:

```
GOAL: <one sentence>
CONTEXT: <2–5 lines: what MyJanitor is, where this piece fits>
READ FIRST: <exact paths: ARCHITECTURE.md, IPC.md, relevant research/spec sections, feature IDs>
YOU OWN: <files/dirs you may create or modify>; DO NOT TOUCH: <everything else, esp. shared contracts>
CONTRACT: <types/functions/IPC commands you must implement or consume, exactly as in docs/IPC.md>
CONSTRAINTS: no new dependency without a DEPENDENCIES.md entry (§7); no destructive op outside temp fixtures; Windows + Linux
DONE WHEN: <tests that must exist and pass; commands to run: cargo fmt --check, cargo clippy -D warnings, cargo test -p <crate>, node --test>
RETURN: ≤ 250 words: what was built, files touched, feature IDs covered, decisions made, known gaps. No code dumps.
```

---

## 6. Phases and waves

Each wave has exit criteria. Don't start a wave until the previous wave's exit criteria are met, except for research that has no dependencies.

### Wave 0: Reconnaissance (parallel, all Sonnet)

| Workstream | Output |
|---|---|
| R1 `pdf-to-markdown` lessons | `docs/research/pdf-to-markdown-lessons.md` |
| R2 `dedup-photos` cache + fuzzy matching | `docs/research/dedup-photos-cache-and-matching.md` |
| R3 `doc-anonymiser-for-.NET` palette | `docs/research/palette.md` (hex tokens + roles) |
| R4 Option B design extraction (§2.3) | `docs/design/option-b-spec.md` |
| R5 Feature matrix | `docs/FEATURE-STATUS.md`: **every feature ID in the spec**, each with tier (§8.1), platform, owner workstream and status `planned` |

**Exit:** all five files exist; you have read their summaries.

### Wave 1: Architecture and contracts (one Opus subagent, then your review)

Produce, building and committing:
- The Cargo workspace skeleton (§7.3) with compiling stubs.
- `docs/ARCHITECTURE.md`: crate boundaries, data flow (scan → index → detectors/filters → candidates → plan → jobs → journal), threading model, cache design (adapted from R2), error model, platform-abstraction trait(s).
- `docs/IPC.md`: every Tauri command and event, with request/response JSON shapes. Events carry progress (scan progress, live detector totals, job progress) so the UI shows live counters.
- Core domain types in `crates/core` (FileEntry, ScanScope, Detector, Filter/Rule, Candidate, Plan, PlanStep, Action, Job, JournalEntry, Estimate, Badge flags).
- The design-token file (`ui/styles/tokens.css`), mapping the R3 palette onto the semantic roles Option B uses (§8.3).

**Exit:** `cargo build --workspace` passes on Linux; you approve the contracts and freeze them.

### Wave 2: Core engines + UI shell (parallel)

| ID | Workstream | Model | Feature IDs (main) |
|---|---|---|---|
| W2-A | Scan engine, cache, platform scanning (MFT fast path on Windows, directory walk fallback, cloud placeholder detection, allocated vs actual size) | Opus | SCAN-01…11 |
| W2-B | Duplicate and similarity engine (staged size → partial hash → full hash; perceptual image hash; version chains; derived files) | Opus | CLN-DET-01/02/18/19/20, OPT-FNT-01 |
| W2-C | Filters, rule engine (AND/OR), presets, quick-query parser (`ext:mp4 size:>1GB modified:<2024`) | Sonnet | CLN-FLT-*, CLN-ENG-* |
| W2-D | Simple detectors (empty, zeroed, broken links, temp/partial, app caches, Office leftovers, dev artefacts, installers, sidecars, stale, inactive folders, language packs) | Sonnet | CLN-DET-06…17, 21 |
| W2-E | Action engine and safety: journal + undo/restore, Recycle Bin/Trash, quarantine with auto-purge, move/copy keeping structure, hardlinks/reflinks, "free up space" for OneDrive placeholders on Windows, protected paths, signed and sensitivity-labelled file detection, synced-folder detection | Opus | CLN-ACT-*, SAF-* |
| W2-F | Frontend shell: tokens, components, Option B screens B1 and B2 working against a **mock IPC layer** | Sonnet | UI zones, spec §2 states |

**Exit:** each engine has unit and integration tests on fixture trees; the UI runs with mocks and screenshots match Option B structurally.

### Wave 3: Optimisation + remaining surfaces (parallel)

| ID | Workstream | Model |
|---|---|---|
| W3-A | Optimisation framework (dry-run estimate → apply, keep-if-not-smaller, light-copy mode, backup policy, fit-to-target-size loop) + ZIP repack + OOXML optimisation (PowerPoint/Excel/Word) | Opus |
| W3-B | PDF structural optimisation, archiving (ZIP native; 7z/zstd through external tools), file-system compression (Windows WOF; Btrfs on Linux where available) | Opus |
| W3-C | Images (lossless first; lossy and format conversion through external encoders when present), adapters for video/audio/screen recordings through external `ffmpeg` (detected at runtime) | Sonnet |
| W3-D | Real IPC wiring for all screens; the screens Option B doesn't show (§8.2) | Sonnet |
| W3-E | Settings drawer, scheduled profiles, CLI (`myjanitor-cli`, same core), Windows Explorer context-menu registration (per user, no admin), portable mode, performance settings | Sonnet |

**Exit:** end-to-end flow on a fixture tree: scan → build plan → dry run → apply → undo, through both the GUI commands and the CLI.

### Wave 4: Integration, hardening, delivery

1. Integration pass (Opus): wiring gaps, error paths, cancellation and pause, large-tree performance (synthetic tree of 200k+ files).
2. **Adversarial review** (Opus, read-only): every code path that deletes, moves, overwrites or compresses. Look for path traversal, symlink and junction following, TOCTOU races, protected paths being bypassed, partial-write corruption, and undo gaps. Fix every finding, then review again.
3. CI (Sonnet): GitHub Actions with a matrix of `ubuntu-22.04` and `windows-latest` running fmt, clippy, tests and the Tauri bundle build, with artifacts uploaded.
4. Docs (Sonnet): `README.md` (what, build, run, test, limitations), final `docs/FEATURE-STATUS.md`.

**Exit = Definition of Done (§11).**

---

## 7. Dependency policy

### 7.1 Rule

Add an external crate (or npm package) **only if a correct in-house replacement, including thorough tests, would take more than about 5 hours of Claude Code work**. Everything else is written in-house, with tests against official test vectors or specs where they exist.

Before adding any dependency, write a `docs/DEPENDENCIES.md` entry: *crate, version, purpose, estimated rewrite effort and why it exceeds 5 h, licence (must be MIT/Apache-2.0/BSD/Zlib compatible), transitive-dependency count*. Prefer pure-Rust crates with few transitive dependencies. Pin exact versions.

### 7.2 Pre-ruled defaults (override only with a logged decision)

| Need | Ruling |
|---|---|
| Tauri, tauri-build, Tauri's own transitive stack | **Allowed** (required stack) |
| serde + serde_json | **Allowed** (derive macros + JSON exceed the threshold) |
| DEFLATE compression (zlib / ZIP / PNG / OOXML) | **Allowed**: `miniz_oxide` (a high-ratio deflate encoder exceeds 5 h). The ZIP container read/write is **in-house** |
| Regular expressions (CLN-FLT-01) | **Allowed**: `regex` |
| JPEG/PNG decoding for perceptual hashing and image work | Decide with the rule; if a decoder is needed, prefer small focused crates (e.g. `zune-jpeg`, `zune-png`) over the full `image` crate |
| PDF object parsing/rewriting | Decide with the rule; check first whether `pdf-to-markdown` already has reusable parsing knowledge |
| Hashing (xxHash64 for staged dedup, SHA-256 for the audit log) | **In-house**, verified against official test vectors |
| EXIF/TIFF, ID3/FLAC tags, MP4 box parsing (metadata only) | **In-house** |
| Minimal XML pull parser for OOXML | **In-house** |
| Glob/wildcard matching, query parser, date maths, CLI argument parsing, INI parsing (winapp2.ini) | **In-house** (no `clap`, `chrono`, `glob`) |
| Thread pool / parallelism | **In-house** with `std::thread` + channels (no `rayon`; use Tauri's async runtime only at the IPC boundary) |
| Win32 APIs (Recycle Bin via `SHFileOperationW` + `FOF_ALLOWUNDO`, `DeviceIoControl` for `FSCTL_ENUM_USN_DATA` / WOF, `CreateHardLinkW`, cloud-file attributes, `CfGetPlaceholderState`) | **In-house hand-written `extern "system"` FFI** for this limited set (no `windows`/`windows-sys`) |
| Linux APIs (`FICLONE` reflink ioctl, `statx`, freedesktop Trash spec) | **In-house** FFI and implementation (no `libc`/`nix` unless the rule says otherwise) |
| Video/audio encoding, AVIF/JPEG XL/WebP encoding, Opus, 7z, zstd, local transcription | **No linked libraries.** Detect optional external CLI tools at runtime (`ffmpeg`, `avifenc`, `cjxl`, `cwebp`, `7z`, `zstd`, `oxipng`), run them as separate processes, and disable the feature with a clear reason when the tool is missing. Never download tools |
| Frontend | **No npm runtime dependencies, no bundler**: vanilla HTML/CSS/ES modules, `withGlobalTauri`, unless `pdf-to-markdown` shows a better-proven setup. Frontend tests run with `node --test` |

---

## 8. Scope and UI

### 8.1 Feature tiers (build in this order)

| Tier | Contents | Expectation |
|---|---|---|
| **T1, core** | Scan + cache, all metadata filters, query syntax, rule engine, presets, exclusions, exact duplicates, empty/temp/cache/dev/installer/stale detectors, Recycle Bin/Trash, quarantine, move/copy, journal + undo, protected paths, synced-folder warning, signed/labelled/encrypted file detection, savings estimates, Option B UI for all main states | **Must be complete and tested** |
| **T2, optimisation** | ZIP/OOXML repack, PowerPoint/Excel/Word optimisation, PDF lossless structural pass, lossless images, ZIP archiving with integrity test, Windows WOF compression, hardlinks/reflinks, similar images, version chains, derived files, CLI, settings, scheduled profiles | Should be complete |
| **T3, external-tool and advanced** | Video/audio/screen-recording re-encoding through ffmpeg, lossy image formats through external encoders, 7z/zstd, audio fingerprinting, local transcription, secure shred, free-space wipe, SQLite vacuum, OST/PST and VHDX compaction, UPX | Build if time allows; otherwise visible but disabled with a reason |

Every feature ID in the spec appears in `docs/FEATURE-STATUS.md` with one of the statuses `done`, `partial`, `disabled-with-reason` or `not-targeted`, plus a one-line note.

### 8.2 Option B, the reference UI

The interaction model is: **scan → ranked "opportunity" cards → add to a plan → review each step → confirm → apply → undo from History**.

- **B1 Overview (results loaded):** a left nav rail (Overview, Clean, Optimise, Archive, History, Settings); a header with scope chips (+ Add location), "Scanned N min ago · cached" and **Scan again**; a savings summary card (total reclaimable, a stacked bar split into *Reversible / Lossy opt-in / Review only*, per-drive bars showing free space before and after the plan); tabs *All / Clean / Optimise / Review only* with sorting; a 3-column grid of **opportunity cards** (title, one-line description, size, file count, safety badge, Review + Add buttons, "In plan" state with an accent border); a right panel **"Your plan"** (reorderable steps with size and remove, estimated total, a "Before you apply" checklist, *Estimate exactly (dry run)*, primary *Apply plan · N GB*, *Save as weekly schedule*); a **bottom status strip** (Reclaimable, Reclaimed, plan summary, queue state, Undo last action, Last report).
- **B2 Step review + lossy confirmation:** a breadcrumb back to Overview, badges (*in plan*, *lossy · opt-in*); a summary line (count, size before → after); sub-group tabs; a results table with checkboxes, per-row after-size and badges (duplicate, needs transcript, confidential = auto-unchecked); a right panel **"How to shrink them"** (preset tiles first, then a few toggles, a preview such as a waveform with removable silences, a per-step estimate, *Listen/Compare before / after*). **Confirmation dialog** for lossy actions: what happens, Files/Now/After/Saving tiles, an *Originals* policy dropdown (default Recycle Bin), a synced-folder warning with an *online-only instead* alternative, an "I understand" checkbox, *Compare a sample first*, *Back*, and an acting button in the **warning colour**.
- **Screens Option B doesn't show:** derive them from B1/B2 patterns. Use the step-review layout for every Clean/Optimise/Archive step; History is a journal list with Restore; Settings is a drawer (spec SETTINGS zone); the empty state is the Overview with a scope picker and one primary Scan button; scanning is the Overview with live-filling cards and a progress header with Pause/Stop; action running expands the bottom strip into a job queue; action completed shows a summary with Undo and View report. Power-user access to the full rule builder and raw grid lives inside each step and in a "Custom search" entry on the Clean screen.

### 8.3 Visual language

- Apply the **doc-anonymiser-for-.NET palette** (R3) through CSS custom properties in `ui/styles/tokens.css`. No colour literals outside the token file.
- Keep the semantic roles from Option B even when the palette's names differ: *accent/primary* (actions, selection, "in plan"), *success* (reversible, savings), *warning* (lossy, irreversible, synced-folder warnings, the acting button in lossy dialogs), *danger* (permanent delete, shred, stop), *neutral surfaces and borders*. If the palette lacks a role, derive one that meets WCAG AA contrast and log it.
- Support light and dark themes if the palette has both; otherwise ship light and log it.
- One scroll container per view, no nested scrolling. Inspector/panel content uses progressive disclosure (collapsed "Advanced").

---

## 9. Platform specifics

| Concern | Windows | Linux |
|---|---|---|
| Fast indexing | NTFS MFT enumeration (`FSCTL_ENUM_USN_DATA`), needs volume handle access; fall back to a directory walk without admin | Directory walk (`statx` where available) |
| Trash | Recycle Bin (`SHFileOperationW`, `FOF_ALLOWUNDO`) | freedesktop.org Trash spec (`~/.local/share/Trash` + per-volume `.Trash-$UID`) |
| Cloud placeholders | OneDrive/Dropbox placeholder attributes; exclude online-only files from totals; "Free up space" = dehydrate | Not applicable: mark as such |
| Transparent compression | WOF (XPRESS4K/8K/16K, LZX), decompress/revert | Btrfs zstd (`chattr +c` / property) when the filesystem supports it; otherwise disabled |
| Links | NTFS hardlinks | Hardlinks; reflinks via `FICLONE` on Btrfs/XFS |
| Elevation | Ask only when needed (system folders, MFT); never by default | Never; report permission-denied paths |
| Context menu | Per-user registry entries (HKCU), no admin | `.desktop` actions / file-manager scripts where standard; otherwise not targeted |
| Paths | Long paths (`\\?\`), case-insensitive comparisons, junctions are never followed | Symlinks are never followed during destructive operations |

Verify Windows compilation from the session even though it runs on Linux: `rustup target add x86_64-pc-windows-msvc` and `cargo check --workspace --target x86_64-pc-windows-msvc` (no linking needed). The CI matrix (Wave 4) does the full Windows build.

---

## 10. Quality gates

### 10.1 After every merge

`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` · `node --test ui/` · the Windows `cargo check` above. Never merge on red. Never skip or delete a failing test to get green: fix the cause.

### 10.2 Test expectations

- Every detector, filter and optimiser has unit tests plus an integration test on a generated fixture tree.
- The action engine has tests that prove **every action can be undone** (state before == state after undo), including partial failures mid-batch.
- Format code (ZIP, OOXML, PDF, EXIF, hashing) is tested against reference vectors and round-trips (the optimised file still opens: valid ZIP central directory, well-formed XML parts, and so on).
- The UI has render tests (views produce the expected DOM) and wiring tests (controls call the right IPC commands with the right payloads).

### 10.3 Session safety

- Fixture trees are created under a temporary directory by the test itself and removed afterwards.
- The core action engine refuses to operate outside an explicit allowlist of roots supplied by the caller. Tests pass only their own temp root.
- Never point the GUI or CLI at `/`, `$HOME`, the repo or any reference repo with a destructive action.

---

## 11. Definition of Done

1. `myjanitor` builds and its tests pass on Linux in the session. Windows passes `cargo check`. The CI workflow for both platforms is committed.
2. All **T1** features are `done` in `docs/FEATURE-STATUS.md`; T2/T3 are marked honestly.
3. The GUI follows Option B: B1 and B2 flows plus the derived screens and all five main states (empty, scanning, results, action running, completed with undo).
4. The CLI covers scan, list candidates, dry run, apply and undo, using the same core.
5. `docs/` contains: research files, `option-b-spec.md`, `ARCHITECTURE.md`, `IPC.md`, `DECISIONS.md`, `DEPENDENCIES.md` (every external crate justified), `FEATURE-STATUS.md`, `STATUS.md`. `README.md` explains build/run/test and the known limitations.
6. The adversarial review of destructive paths is done, and every finding is fixed or explicitly accepted with a reason in `DECISIONS.md`.

## 12. Git workflow

- Work on a feature branch (e.g. `build/initial-app`), never directly on the default branch.
- Make small, conventional commits (`feat(core): staged duplicate hashing`, `fix(ui): …`), at least once per completed workstream. Push regularly so progress survives interruptions.
- At the end, open a pull request whose description has: a summary, the feature-status table (counts per tier and status), the external dependencies list, known limitations and how to run it.

## 13. Final report (your last message)

Report in at most 400 words: what works, what's partial or disabled and why, external dependencies added and why, decisions the owner should review first (top 5 from `DECISIONS.md`), test/CI status with the real numbers, and the PR link.
