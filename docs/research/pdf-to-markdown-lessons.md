# Lessons from `pdf-to-markdown` (PDF2MD) for MyJanitor

Source: `/home/user/pdf-to-markdown` (read-only), 7 commits, version 0.1.1, MIT.
Research notes only. Values below were read from the repo files named in each section.

---

## 1. Versions and workspace layout

**Resolved in `Cargo.lock`** (manifests only say `"2"`, so the lock is what fixes them):

| Crate | Locked version | Declared in |
|---|---|---|
| `tauri` | 2.12.3 | `src-tauri/Cargo.toml`: `tauri = { version = "2", features = [] }` |
| `tauri-build` | 2.7.2 | build-dependency `"2"` |
| `tauri-plugin-dialog` | 2.8.2 | `"2"` |
| `tauri-plugin-fs` | 2.7.0 | transitive (dialog dependency), not used directly |
| `tauri-runtime` / `tauri-runtime-wry` | 2.12.1 / 2.12.1 | transitive |
| `wry` / `tao` | 0.57.0 / 0.37.1 | transitive |
| `mimalloc` | 0.1.52 | workspace dep, `default-features = false` |
| `rayon` | 1.12.0 | |
| `pdf_oxide` | 0.3.78 (pinned `=0.3.78`) | |

JS side (`package.json`): `@tauri-apps/api ^2.12.3`, `@tauri-apps/plugin-dialog ^2.8.2`,
`@tauri-apps/cli ^2.12.2`. Keep the JS `@tauri-apps/*` major/minor in step with the Rust crates.
Dev tooling: vite ^8.3.4, vitest ^5.0.4, typescript ^7.0.2, jsdom ^30.1.2.

**Rust:** `edition = "2021"`, `rust-version = "1.88"` (workspace-level, inherited with
`version.workspace = true` etc.). CI uses `dtolnay/rust-toolchain@stable`.

**Workspace** (root `Cargo.toml`, `resolver = "2"`):
- `crates/pdf2md-engine`: pure library + CLI bin, no Tauri dependency.
- `crates/pdf-fixtures`: deterministic raw PDF writer and corpus generator (dev tool, also a dev-dependency).
- `src-tauri`: thin shell. `[lib] name = "pdf2md_lib"`, `crate-type = ["staticlib","cdylib","rlib"]`;
  `main.rs` just calls `pdf2md_lib::run()`.
- Frontend at repo root (`index.html`, `src/`, `vite.config.ts`), not in a subfolder.

**Profiles worth copying:**
- `[profile.release]`: `lto = "thin"`, `codegen-units = 1`, `strip = true`, `panic = "unwind"`
  (unwind is required so `catch_unwind` can isolate parser panics).
- `[profile.ci]` inherits release with `lto = false`, `codegen-units = 16`, `strip = false`:
  optimised tests without paying LTO link time on every test binary.
- `[profile.dev.package.pdf_oxide] opt-level = 2` so tests on a heavy dependency run at realistic speed.

**Dependency pin lesson:** `pdf_oxide 0.3.78` failed to compile against `office_oxide >= 0.1.13`
(missing struct field). Fix: add `office_oxide = "=0.1.9"` to the workspace and commit `Cargo.lock`.
Rule: commit `Cargo.lock`, pin `=` on any crate that breaks on semver-compatible updates, and
comment the reason next to the pin.

`.gitattributes`: `* text=auto eol=lf`; `*.pdf *.png *.ico *.icns binary` (byte-exact test inputs).
`.gitignore` includes `/src-tauri/gen/schemas/` (generated capability schema).

---

## 2. `src-tauri/tauri.conf.json`

| Key | Value |
|---|---|
| `$schema` | `https://schema.tauri.app/config/2` |
| `productName` / `version` | `PDF2MD` / `0.1.1` (version duplicated in Cargo workspace, package.json, tauri.conf.json: three places to bump) |
| `identifier` | `io.github.rosca75.pdf2md` |
| `build.beforeDevCommand` | `npm run dev` |
| `build.devUrl` | `http://localhost:1420` (Vite `strictPort: true`, host `127.0.0.1`) |
| `build.beforeBuildCommand` | `npm run build` (`tsc --noEmit && vite build`) |
| `build.frontendDist` | `../dist` (Vite output, relative to `src-tauri/`) |
| `app.withGlobalTauri` | **not set** (default false); no `window.__TAURI__` usage anywhere |
| window | one, label `main`, 1100x720, min 800x560, `dragDropEnabled: true` |
| `security.csp` | `default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost` |
| `bundle.active` | true |
| `bundle.targets` | `["appimage", "app"]` (no `nsis`/`msi`/`deb`) |
| `bundle.category` | `Productivity` |
| `bundle.icon` | `icons/32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`, `icon.ico` |
| `bundle.macOS.minimumSystemVersion` | `12.0` |

Notes for MyJanitor:
- The CSP's `connect-src ... ipc: http://ipc.localhost` is the Tauri 2 IPC allowance. Without
  `ipc:` and `http://ipc.localhost` in `connect-src`, `invoke` is blocked by the CSP on
  Windows/Linux. Keep it verbatim.
- `style-src 'unsafe-inline'` was kept for inline `style` attributes (the clipboard fallback sets
  `style.position`). A stricter app can drop it if it uses only classes.
- Windows ships as a **portable exe** (`tauri build --no-bundle`), Linux as AppImage, macOS as a
  zipped `.app`. `bundle.targets` therefore does not list Windows installers at all; the Windows
  artefact is `target/release/pdf2md.exe` copied out. If MyJanitor wants an installer it must add
  `nsis` (or `msi`) itself; nothing here is proven for that.
- Window controls: `dragDropEnabled: true` is what makes Tauri deliver OS file drops with real
  paths (see section 5); HTML5 drop events do not give paths.

---

## 3. Capabilities / permissions

Single file `src-tauri/capabilities/default.json`:

```
identifier "default", windows ["main"], permissions ["core:default", "dialog:default"]
$schema "../gen/schemas/desktop-schema.json"  (generated; gitignored)
```

- `core:default`: required for `invoke`, `listen`, and window/webview APIs. The drag-drop
  listener (`getCurrentWebview().onDragDropEvent`) works under `core:default`; no extra window
  permission was needed.
- `dialog:default`: the native open-file / open-folder dialogs (`open()` from
  `@tauri-apps/plugin-dialog`). Registered with `.plugin(tauri_plugin_dialog::init())`.
- **No `fs` plugin permissions.** All file reading and writing happens in Rust commands
  (`std::fs`), so the webview never gets filesystem scope. This is the pattern to keep: the UI
  passes paths it got from a dialog or a drop, Rust does the I/O.
- Custom `#[tauri::command]`s need no capability entry in this setup (app-defined commands are
  allowed by default unless a `build.rs` `AppManifest` restricts them; the repo's `build.rs` is just
  `tauri_build::build()`). If MyJanitor later adds an `AppManifest`, commands must be listed.

---

## 4. How the frontend loads

**Bundler: Vite + TypeScript**, not vanilla. `index.html` has `<script type="module" src="/src/main.ts">`;
`vite build` emits `dist/`; `frontendDist` points at `../dist`.

- `src/api.ts` is the **only** module that imports `@tauri-apps/*` (`invoke`, `listen`,
  `getCurrentWebview`, `open`). Everything else gets an `Api` interface, so Vitest can inject a mock.
  This is the same isolation idea as the DedupPhotos `api.js` rule.
- No `withGlobalTauri`; the npm `@tauri-apps/api` package is imported and bundled instead.
- `vite.config.ts` specifics: `clearScreen: false`; `envPrefix: ["VITE_", "TAURI_ENV_*"]`;
  `build.target` = `chrome105` when `TAURI_ENV_PLATFORM === "windows"` else `safari13`;
  `minify`/`sourcemap` keyed off `TAURI_ENV_DEBUG`; `server.watch.ignored` excludes `src-tauri` and `crates`.
- **The Rust crate cannot compile without `dist/`** (`tauri::generate_context!` embeds
  `frontendDist` at compile time). Every CI job and every local `cargo test/clippy` must run
  `npm run build` first. Documented in README and in the CI comment.

**Implications for MyJanitor (no bundler, vanilla ES modules, `withGlobalTauri`):**
pdf-to-markdown does not prove that setup, so the following is inference from Tauri 2 behaviour,
not from the reference repo:
1. `build.frontendDist` must point directly at the folder holding `index.html` (for example
   `../ui`), and it must exist at `cargo build` time because `generate_context!` embeds it. With no
   bundler, `beforeBuildCommand` can be omitted and `devUrl` can be omitted (Tauri then serves
   `frontendDist` directly; `tauri dev` has no hot reload but a manual reload works).
2. Set `"app": { "withGlobalTauri": true }` and use `window.__TAURI__.core.invoke`,
   `window.__TAURI__.event.listen`, `window.__TAURI__.webview.getCurrentWebview()`; dialogs via
   `window.__TAURI__.dialog.open` (plugin-dialog exposes itself on the global when the plugin
   is registered and `withGlobalTauri` is on).
3. Keep the reference's isolation rule: one `api.js` module wrapping every `window.__TAURI__`
   call, with a mock-able interface for tests.
4. Asset paths in `index.html` are root-relative to `frontendDist` (`/js/app.js`, `/css/base.css`).
5. CSP applies: no inline `<script>`; `type="module"` files from `'self'` are fine. Keep the
   `connect-src` IPC entries. Vendor any font or icon locally (the reference uses system fonts only).
6. Trade-off: the reference gets typechecking (`tsc` strict), test mocking and a fixed dev
   port from Vite. A no-bundler frontend loses `tsc`; consider `// @ts-check` plus JSDoc
   types, and Vitest/jsdom can still test plain ES modules without a bundler step.

Recommendation: the no-bundler plan is viable and removes the Node build prerequisite for
`cargo build`; the only pre-build requirement becomes "the `ui/` folder exists". Verify early
with a spike that `window.__TAURI__.dialog` is present (the reference never used the global).

---

## 5. IPC patterns (`src-tauri/src/lib.rs`, `jobs.rs`, `save.rs`, `src/api.ts`)

**Commands** (4): `extract`, `get_markdown`, `remove`, `save_all`; registered with
`tauri::generate_handler![...]`. All are `async fn` taking `State<'_, AppState>`.
- `extract(app: AppHandle, state, id: String, path: String, password: Option<String>) -> Result<(), String>`:
  returns immediately after spawning work; results arrive as events.
- `get_markdown(id) -> Result<String, String>`, `remove(id) -> Result<(), String>`.
- `save_all(dir: String, items: Vec<SaveRequest>) -> Result<Vec<SaveOutcome>, String>`.
- Argument names are camelCase on the JS side matching Rust snake_case params by Tauri's default
  conversion (the JS calls use `{ id, path, password }`, `{ dir, items }`).

**Error type serialisation:** command-level errors are plain `String` (`Result<_, String>`);
JS receives a rejected promise with that string. `api.ts::errorText` flattens string / Error /
JSON. Domain errors that should be shown per-file are **not** command errors: they travel inside
`DoneEvent.error` as a serialisable struct `{ code, message, hint }` (the engine's
`ConvertError` has `code()` and `hint()`: what failed plus an actionable next step). Batch
operations return per-item outcomes (`SaveOutcome { id, path?, error? }`) so one failure does not
fail the batch. Optional fields use `#[serde(skip_serializing_if = "Option::is_none")]`.

**Events:** names `extract://progress` (`{id, done, total}`) and `extract://done`
(`{id, ok, pages?, warnings?, error?}`), emitted with `app.emit`. Frontend uses
`listen<T>(name, e => cb(e.payload))` and returns the unlisten function. Emit failures are
ignored (only fail when the window is closed). Progress is throttled to one event per 50 ms
(`PROGRESS_INTERVAL`, about 20/s per file) so the webview is not flooded.

**Long-running work off the main thread:** `tauri::async_runtime::spawn_blocking` from the command;
the command itself returns `Ok(())` at once. Inside, the engine runs on a dedicated thread with a
16 MiB stack (`PARSE_STACK`) because hostile PDFs recurse deeply and a stack overflow aborts the
process rather than panicking.

**Concurrency limit:** `MAX_CONCURRENT = 2` files at a time, via a hand-written counting
semaphore (`Mutex<usize>` + `Condvar`, RAII `Permit`) whose waiters give up if their file is
cancelled. Lock helper recovers from poisoned mutexes (`lock()` ignores poison).

**Cancellation:** `CancelToken` (shared atomic flag) passed into the engine, checked per page.
`AppState.begin(id)` assigns a monotonically increasing **generation**, cancels and replaces any
previous run for the same `id`; `finish()` stores a result only when the generation is still
current and not cancelled, so a stale/cancelled run can never overwrite a newer one or leak
a "done" event. `remove(id)` cancels and frees memory. Cancelled runs emit nothing.

**Panic isolation:** `panic::catch_unwind(AssertUnwindSafe(...))` around the engine call; a panic
becomes an `ErrorInfo` (`code`, `message`, `hint`) rather than killing the app. Requires
`panic = "unwind"` in release.

**State:** `AppState` managed via `.manage(AppState::default())`; results (Markdown) held in
`Arc<String>` so `get_markdown` and `save_all` do not copy under the lock.

**File input:** the webview gets real paths from (a) the dialog plugin (`open({multiple, filters})`)
and (b) `getCurrentWebview().onDragDropEvent` (payload types `enter|over|drop|leave`, with `paths`
on enter/drop). Needs `dragDropEnabled: true`. The id is chosen by the frontend and passed to Rust.

**Save safety (`save.rs`):** never overwrite (`name (1).md`, `(2)`), collisions inside a batch
tracked in a set, filename sanitising, Windows reserved device names (CON, NUL, COM1...) matched
before the first dot, write via temp file then persist with `0644` (temp files are `0600`),
`describe_io_error` turns `io::Error` into actionable sentences.

**Frontend convenience storage:** `localStorage` wrapped in try/catch (`platform.ts`), last
output folder only.

---

## 6. Build, bundle, release, CI

**Commands** (via npm so the pinned CLI is used): `npm run tauri -- dev`;
`npm run tauri -- build --no-bundle` (Windows portable exe);
`npm run tauri -- build --bundles appimage` (Linux);
`npm run tauri -- build --target universal-apple-darwin --bundles app` (macOS).

**`ci.yml`** (push to main + PRs; `concurrency` cancel-in-progress; `permissions: contents: read`):
- Matrix `os: [ubuntu-24.04, windows-latest, macos-latest]`, `fail-fast: false`.
- Actions: `actions/checkout@v7`, `dtolnay/rust-toolchain@stable` (components rustfmt, clippy),
  `Swatinem/rust-cache@v2`, `actions/setup-node@v7` (`node-version: lts/*`, `cache: npm`).
- Linux apt packages: `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libxdo-dev
  libssl-dev build-essential curl wget file`.
- Order: apt deps, `npm ci && npm run build` (dist must exist for the Tauri crate),
  `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
  (Ubuntu only), `cargo test --workspace --profile ci` (all OSes).
- Separate `frontend` job: `npm ci`, `npm test`, `npm run build`.

**`release.yml`** (tags `v*` + manual dispatch; build jobs `contents: read`, publish job
`contents: write`; `bash` default shell so Windows steps work):
- Version derived from the tag (`GITHUB_REF_NAME#v`), else `0.0.0-dev`.
- `fail-fast: false` so one platform failing does not hide others; publish job `needs: build`.
- Full frontend and Rust tests run before bundling.
- Linux env: `NO_STRIP=true` (linuxdeploy's bundled strip is too old for new toolchains) and
  `APPIMAGE_EXTRACT_AND_RUN=1` (no FUSE on runners).
- macOS: targets `aarch64-apple-darwin,x86_64-apple-darwin`, `ditto -c -k --sequesterRsrc --keepParent`.
- Artefact names: `PDF2MD_<ver>_windows_x64_portable.exe`, `_linux_x86_64.AppImage`,
  `_macos_universal.zip`; `SHA256SUMS.txt` generated; `softprops/action-gh-release@v3`;
  `actions/upload-artifact@v7`, `download-artifact@v8` with `merge-multiple`.
- Release notes embedded in the workflow: unsigned-binary warnings (SmartScreen "More info, Run
  anyway", macOS right-click Open or `xattr -cr`, Linux `chmod +x` and `libfuse2t64` on Ubuntu 24.04).
- `find ... | head -n 1` to locate the AppImage/`.app` and fail loudly with diagnostics if absent.

**Platform notes:** Windows needs WebView2 (Windows 11 has it) and MSVC build tools; the console
window is hidden in release via `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`
in `main.rs`. Binaries are unsigned. Not proven here: Windows installers, code signing, `.deb`
packaging, Linux CI beyond `ubuntu-24.04`.

---

## 7. Tests

- **Rust unit tests** inline (`#[cfg(test)]`) in `save.rs`, `jobs.rs` (collision naming, reserved
  names, no-overwrite, missing folder, unicode round-trip, missing file reports not-found...).
- **Engine integration tests** `crates/pdf2md-engine/tests/`: `corpus.rs` (data-driven),
  `parallel.rs` (parallel output identical to single-threaded; progress; cancellation),
  `performance.rs` (throughput floor and peak-heap ceiling; only meaningful in optimised builds).
  Criterion bench `benches/convert.rs`; `examples/bench` with a counting allocator.
- **Corpus:** `tests/corpus/*.pdf` (34 committed files) plus `manifest.toml` with per-case
  `file`, `about`, `password`, `expect_error`, `must_contain`, `must_not_contain`, `in_order`;
  only assertions any correct implementation must satisfy. Files are generated deterministically
  by `crates/pdf-fixtures` (`cargo run -p pdf-fixtures --bin gen-corpus`); `pdf-fixtures/tests`
  verify the corpus opens and large docs generate.
- **Frontend tests** Vitest + jsdom (`pool: threads`), `src/**/*.test.ts`: `app.test.ts` (with a
  mock `Api`), `state.test.ts`, `view/markdown.test.ts`.
- Run: `npm test`; `cargo test --workspace --profile ci`. Needs `npm run build` first.
- Design doc: `docs/superpowers/specs/2026-10-13-pdf2md-design.md` (not analysed in depth).

---

## 8. Bugs fixed after the fact (git log) as pitfall, fix, rule

| # | Pitfall (commit) | Fix | Rule for MyJanitor |
|---|---|---|---|
| 1 | **Stack overflow aborting the app**: touching boxes gave negative gaps, XY-cut recursed forever on the same region (f4a1fc9) | Clamp gaps so each level progresses; add a depth guard (overflow aborts, it cannot be caught) | Any recursion over untrusted input needs a progress guarantee and an explicit depth limit |
| 2 | Synthetic corpus missed real-world failures; found only on a real 756-page spec (f4a1fc9) | Root-cause fixes plus regression tests | Test on real, large, ugly files, not only generated ones; add each find as a regression test |
| 3 | Panics and stack depth on hostile input; only extra workers had big stacks (804edc4) | Whole job on a 16 MiB-stack thread; panics mapped to errors; per-page guards | Run every file operation on a big-stack worker inside `catch_unwind`; keep `panic = "unwind"` |
| 4 | Non-total ordering comparator can make `sort` panic (804edc4) | Total-order fallback | Never sort with a comparator on floats or partial keys without a total order (use `total_cmp`) |
| 5 | Quadratic work on pathological pages (>2000 rules) (804edc4) | Skip the pairing step past a threshold | Put caps on any O(n^2) step over file-derived counts |
| 6 | Output injection: link targets with control chars, schemes, `\|` breaking tables (804edc4) | Allowlist `http(s)/mailto/ftp`, reject control chars, percent-encode structure chars | Treat file contents as hostile when rendering to any markup or UI; allowlist, do not denylist |
| 7 | Windows reserved device names (CON, NUL...) only matched without extension (804edc4) | Match before the first dot | Sanitise output filenames for Windows rules even on Linux builds |
| 8 | Saved files created 0600 because they came from a temp file (804edc4) | chmod 0644 on Unix | Temp-then-rename writes inherit temp permissions; set them explicitly |
| 9 | macOS packaging step `cd`'d into the bundle dir, so the final `ls out` failed after the zip existed (777e5a4) | `find` the bundle from the repo root, diagnostics when missing, `fail-fast: false` | Never `cd` in release steps; locate artefacts with `find` and fail with diagnostics; let all platforms finish |
| 10 | v0.1.0 published nothing because one job failed (777e5a4) | Released 0.1.1 instead of moving the public tag | Never move or reuse a tag; bump the version |
| 11 | pdf_oxide did not build after a semver-compatible transitive update (office_oxide) (docs/pdf_oxide-evaluation.md) | Exact pin `=0.1.9` plus committed lockfile | Pin fragile deps with `=` and a comment |
| 12 | Heading with trailing `#` lost it (CommonMark closing sequence); `\|` in a code span split a table cell (b055280) | Escape final `#`; fallback to escaped plain text | Escape for the target format at the render edge, with tests per special character |
| 13 | Heading levels collapsed to 6 when all headings shared a size (f4a1fc9) | Derive from numbering depth | Heuristics need a fallback when the dominant signal is flat |
| 14 | System allocator serialised multithreaded parsing on Windows (15f54cc) | mimalloc as `#[global_allocator]` in app and CLI | If a parser is allocation-heavy and parallel, measure with mimalloc on Windows |
| 15 | Parser handle is `!Send`; each worker needs its own document; 3.3x file size per worker (15f54cc, docs/performance.md) | One handle per worker, worker count capped by memory budget (192 MiB) and 8 pages per worker | Budget memory per worker; cap by cores and by size |
| 16 | Frontend flooding: per-page events (15f54cc) | 50 ms progress throttle | Throttle progress events |
| 17 | Stale results racing after re-adding a file (design in jobs.rs) | Generation counter on every run | Tag every background job with a generation; drop results from stale runs |
| 18 | Optimised tests too slow with LTO and too slow unoptimised (15f54cc) | `profile.ci` and `profile.dev.package.<heavy>` | Add a `ci` profile |

No Windows-specific or Linux-specific runtime bugs beyond items 7, 14 and the AppImage env vars
(`NO_STRIP`, `APPIMAGE_EXTRACT_AND_RUN`) appear in the history. The history is short (7 commits),
so this is the complete list, not a sample.

---

## 9. PDF knowledge: relevance to a lossless PDF structural optimiser

**Parser:** `pdf_oxide` 0.3.78 (crates.io; about 450k lines of Rust; 15 minute clean release
build on a 2-core laptop; deps include `aes`, `brotli`, `base64`...). Used **read-only for text**:
`extract_spans`, `get_annotations`, `extract_paths`, `get_page_visible_box`, `is_authenticated`,
`authenticate`. It is not shown to expose any rewrite/save API, and the repo never writes PDFs
through it. The repo does **not** use `lopdf`.

**What can inform an optimiser (limited):**
- `pdf-fixtures` (`crates/pdf-fixtures/src/writer.rs`, `obj.rs`, `encryption.rs`) is a hand-written
  raw PDF writer: ordered object ids, xref tables, flate streams (`deflate`), AES-128 and owner-only
  encryption, incremental update, object streams, malformed variants (bad xref, garbage prefix,
  no EOF, truncated, corrupt stream, zero-byte, non-PDF). The corpus of 34 PDFs (notably
  `object_streams.pdf`, `incremental_update.pdf`, `malformed_*`, `corrupt_stream.pdf`,
  `encrypted_*`, `scanned_image_only.pdf`, `mixed_graphics.pdf`) is a ready-made **test input
  set** for an optimiser's robustness tests.
- Engine error taxonomy (`error.rs`: password-required, `no_text_layer`, cancelled, corrupt...) is
  a useful model for user-facing per-file errors.
- Documented parser behaviours: `from_bytes` keeps two copies of the file (about 2x RAM before
  parsing); parser recovers some malformed files; empty user password is tried automatically;
  CropBox clamped to MediaBox; span bbox y-up despite docs saying top-left.
- Performance baselines: 756 pages/22.5 MB in 2.9 to 3.8 s on a 2-core laptop.

**What does not transfer:** nothing in the repo does stream recompression, object de-duplication,
xref/object-stream rewriting, font subsetting, image recompression or linearisation. The Markdown
layout pipeline (XY-cut, tables, headings) is irrelevant to structural optimisation. For a
lossless optimiser MyJanitor would need a library with write support (for example `lopdf` or
`qpdf` bindings); this repo gives no evidence either way, so that choice needs separate research.

---

## 10. Rules for MyJanitor (checklist)

1. Pin Tauri to the 2.12.x line (`tauri 2.12.3`, `tauri-build 2.7.2`, `tauri-plugin-dialog 2.8.2`); commit `Cargo.lock`; keep JS `@tauri-apps/*` on the matching 2.12 minor.
2. Workspace layout: engine library crate (no Tauri), thin `src-tauri` shell with `[lib]` + `main.rs` calling `run()`, optional fixtures crate; `edition 2021`, `rust-version 1.88` (or newer, set once in `[workspace.package]`).
3. Frontend: either Vite+TS (proven) or no-bundler with `withGlobalTauri`; the latter is unproven in the reference, so spike it first. With no bundler, `frontendDist` is the folder holding `index.html` and must exist at compile time.
4. Wrap every Tauri call in a single `api.js`; make it mockable for tests.
5. CSP must include `connect-src 'self' ipc: http://ipc.localhost`; no inline scripts.
6. Capabilities: `core:default` + `dialog:default` only; do all file I/O in Rust, not through the `fs` plugin.
7. Heavy work: command returns at once, work in `spawn_blocking`, progress and done via events; throttle progress to about 20/s.
8. Cancellation with a shared token plus a per-job generation counter so stale runs never publish results; cap concurrent jobs with a cancel-aware semaphore.
9. Run each file operation on a 16 MiB-stack thread inside `catch_unwind`; release profile `panic = "unwind"`.
10. Return per-item outcomes for batches; errors as `{code, message, hint}` with an actionable hint; plain `String` for command-level failures.
11. Treat every input file as hostile: recursion depth limits, quadratic-work caps, total-order comparators, allowlisted output.
12. Never overwrite user files; unique-name suffixes; sanitise names for Windows reserved names; set explicit permissions after temp-file persist. For a cleaner that deletes or shrinks, also keep originals unless explicitly asked.
13. Use `mimalloc` as global allocator if the work is allocation-heavy and parallel (measure on Windows).
14. Add a `ci` cargo profile and per-package `opt-level` for heavy dependencies.
15. CI: `ubuntu-24.04` + `windows-latest`; apt: `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libxdo-dev libssl-dev build-essential curl wget file`; `Swatinem/rust-cache@v2`; fmt and clippy `-D warnings` on Ubuntu; tests on all; frontend build before cargo if a `dist` is required.
16. Release: tag-triggered, `fail-fast: false`, tests before bundling, locate artefacts with `find` (no `cd`), SHA256SUMS, notes explaining unsigned warnings; never move a tag. Linux AppImage env: `NO_STRIP=true`, `APPIMAGE_EXTRACT_AND_RUN=1`; users on Ubuntu 24.04 may need `libfuse2t64`.
17. Windows: `windows_subsystem = "windows"` in release; needs WebView2; reference ships a portable exe via `--no-bundle` (an installer is unproven and would need `nsis` in `bundle.targets`).
18. Keep versions in sync across `Cargo.toml` workspace, `tauri.conf.json` and `package.json` (three places).
19. Tests: inline unit tests for file-safety logic, data-driven corpus with a `manifest.toml`, generated deterministic fixtures, Vitest+jsdom for UI with a mock API; add every real-world failure as a regression test.
20. For lossless PDF work, this repo only offers test fixtures and error-handling patterns; parser/writer library choice (write-capable) needs separate research.
