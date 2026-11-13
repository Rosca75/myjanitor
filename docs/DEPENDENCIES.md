# External dependencies

Rule (brief §7): an external crate is added only when a correct in-house
replacement with thorough tests would take more than about 5 hours. Every
entry: crate, exact version, purpose, rewrite estimate, licence, transitive
count. Versions are pinned with `=` in the root `Cargo.toml`
(`[workspace.dependencies]`) and `Cargo.lock` is committed.

Transitive counts were measured with `cargo tree -p <crate> -e normal --prefix none
--target all | sort -u` (distinct crates excluding the crate itself). The whole
non-GUI workspace (`mj-engine` and below) pulls **16** external crates in total;
the GUI adds the Tauri stack (about 390 crates across both targets, 242 on
Linux and 207 on Windows for `tauri` alone).

## Pre-ruled (brief §7.2)

| Crate | Version | Used by | Purpose | Rewrite estimate (why > 5 h) | Licence | Transitive |
|---|---|---|---|---|---|---|
| `tauri` | =2.12.3 | `src-tauri` | Desktop shell: window, webview, IPC, events, bundling | Required stack (weeks: WebView2/WebKitGTK bindings, IPC, packaging) | Apache-2.0 OR MIT | 356 (all targets) |
| `tauri-build` | =2.7.2 | `src-tauri` (build) | Generates the context (config, capabilities, embedded `ui/`), Windows resources | Required stack | Apache-2.0 OR MIT | 183 (build-time) |
| `serde` (+ `serde_derive`) | =1.0.229 | all crates | Derive (de)serialisation of every contract type | Derive macros over ~150 types plus a data model: well over 5 h | MIT OR Apache-2.0 | 7 |
| `serde_json` | =1.0.154 | engine, actions, cli, src-tauri; core (dev) | JSON for IPC, settings, presets, schedules, the journal | Conformant JSON parser/serialiser with number edge cases and serde integration: > 5 h | MIT OR Apache-2.0 | 11 |
| `miniz_oxide` | =0.8.9 | formats (W2-E reader, W3-A/B/C writers) | DEFLATE inflate/deflate for ZIP, OOXML, PNG, PDF streams | High-ratio deflate encoder plus a robust inflater: > 5 h (brief ruling). Same version as Tauri's PNG stack, so no extra build | MIT OR Zlib OR Apache-2.0 | 2 |
| `regex` | =1.13.1 | filters (CLN-FLT-01) | Regular-expression name/path filters | Linear-time engine with Unicode classes: far > 5 h (brief ruling) | MIT OR Apache-2.0 | 4 |

## Considered and not added in Wave 1

| Need | Decision |
|---|---|
| Folder picker (`tauri-plugin-dialog` 2.8.2) | **Not added** (D-104): an in-app folder browser over the `fs_list_dir` command is well under 5 h, keeps capabilities at `core:default`, and avoids the plugin's `tauri-plugin-fs` + `rfd` transitive tree. |
| `mimalloc` | Not added; measure in Wave 4 (lessons rule 13). |
| JPEG/PNG decoding for perceptual hashing | W2-B decides under the rule (prefer a small focused decoder such as `zune-jpeg` over `image`), with an entry below before adding it. |
| PDF object parsing | W3-B decides under the rule; in-house first (`mj-formats::pdf`). |

## Additions by workstream

Each workstream appends rows under its own heading only (no shared lines, so
merges do not conflict). Same columns as above.

### W2-A

### W2-B

### W2-C

### W2-D

### W2-E

### W2-F

### W3-A

### W3-B

### W3-C

### W3-D

### W3-E
