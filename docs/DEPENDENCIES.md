# External dependencies

Rule (build prompt 7): an external crate is added only when a correct in-house replacement, with thorough tests, would take more than about 5 hours. Every crate below has its reason. Versions are pinned with `=`, `Cargo.lock` is committed, and the manifests were parsed to check that this file lists exactly the external crates they declare (7 direct crates, none missing, none extra).

The in-house replacements that were written instead are in the second table. The optional external programs that MyJanitor can run, but never links or downloads, are in the third.

## 1. External crates

| Crate | Version | Used by | Purpose | Why not in-house (estimate over 5 h) | Licence | Transitive crates |
|---|---|---|---|---|---|---|
| `tauri` | `=2.12.3` | `myjanitor` (GUI) | Desktop shell: native window, webview, IPC commands and events, bundling | Required stack. WebView2 and WebKitGTK bindings, IPC and packaging are weeks of work | Apache-2.0 OR MIT | 377 (all targets, normal and build edges); 241 on Linux, 206 on Windows (normal edges) |
| `tauri-build` | `=2.7.2` | `myjanitor` (build script) | Embeds the `ui/` folder, config and capabilities; Windows resources | Required stack, same reason | Apache-2.0 OR MIT | 182 (build-time only) |
| `serde` (with `serde_derive`) | `=1.0.229` | every crate except `mj-scan`, `mj-dupes`, `mj-detect`, `mj-optimise` (8 crates, including `myjanitor`) | Derive (de)serialisation of the contract types that cross the IPC bridge, the journal and the settings files | Derive macros over about 150 types plus a data model: well over 5 h | MIT OR Apache-2.0 | 6 (`serde_core`, `serde_derive`, `proc-macro2`, `quote`, `syn`, `unicode-ident`) |
| `serde_json` | `=1.0.154` | `mj-engine`, `mj-actions`, `mj-cli`, `mj-filters`, `myjanitor`; `mj-core` (tests only) | JSON for IPC, settings, presets, schedules and the journal | A conformant JSON parser and writer with number edge cases and serde integration: over 5 h | MIT OR Apache-2.0 | 10 (the 6 of `serde`, plus `itoa`, `memchr`, `zmij`) |
| `miniz_oxide` | `=0.8.9` | `mj-formats` (ZIP, OOXML, PDF streams, PNG), `mj-dupes` (PNG decode) | DEFLATE inflate and a high-ratio deflate encoder | A robust inflater plus a good encoder (level 10 matters for the savings): over 5 h. Fixed by the build prompt | MIT OR Zlib OR Apache-2.0 | 2 (`adler2`, `simd-adler32`; the second is switched on only by the Tauri stack) |
| `regex` | `=1.13.1` | `mj-filters` | Regular-expression name and path filters | A linear-time engine with Unicode classes: far over 5 h. Fixed by the build prompt | MIT OR Apache-2.0 | 4 (`regex-automata`, `regex-syntax`, `aho-corasick`, `memchr`) |
| `zune-jpeg` | `=0.5.15` | `mj-dupes` (perceptual hashing of JPEG without an EXIF thumbnail) | Decode JPEG to luma for dHash and pHash | A correct baseline and progressive decoder (Huffman with restart markers, successive approximation, subsampling variants, CMYK) plus a fuzz-hardened test corpus is several days. Pure Rust; uses `unsafe` internally for SIMD, our own crates stay `forbid(unsafe_code)` and every decode runs inside `catch_unwind`. Details: [`docs/dependencies/W2-B.md`](dependencies/W2-B.md) | MIT OR Apache-2.0 OR Zlib | 1 (`zune-core` 0.5.3, same licence) |

### How the counts were measured

Per crate, from the workspace root:

```
cargo tree -p <crate>@<version> -e normal,build --prefix none --target all \
  | sed 's/ (\*)//; s/ (proc-macro)//' | sort -u | grep -v '^<crate> v' | wc -l
```

That is distinct crates reachable from the crate, excluding itself, over all targets (blank separator lines that `cargo tree` prints between edge kinds are dropped). Cargo unifies features across the workspace, which is why `miniz_oxide` shows two crates here. The per-OS numbers for `tauri` use `--target x86_64-unknown-linux-gnu` and `--target x86_64-pc-windows-msvc` with `-e normal`.

### Workspace totals

| Scope | External crates | Method |
|---|---|---|
| Whole workspace, all targets, every edge kind | **408** | `Cargo.lock` has 420 packages, minus the 12 workspace members |
| Without the Tauri stack (the 11 library and CLI crates: `mj-core` to `mj-engine`, `mj-cli`) | **19** | union of `cargo tree -p <member> -e normal,build --prefix none --target all` over those members, minus the members |
| Tauri stack (GUI only) | **389** | 408 minus 19 |
| GUI closure on Linux / on Windows | 262 / 221 | `cargo tree -p myjanitor -e normal,build --prefix none --target <triple>` |

The 19 crates without Tauri are the 5 direct ones (`serde`, `serde_json`, `miniz_oxide`, `regex`, `zune-jpeg`) and 14 transitive ones. `myjanitor-cli` therefore links only those 19.

### Licences across the whole lock

All 408 external crates are permissive (MIT, Apache-2.0, Zlib, BSD, ISC, Unicode-3.0, Unlicense and combinations) with two things to know:

- 5 crates are MPL-2.0 (`cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, `selectors`). They arrive through the Tauri stack and are used unmodified. MPL-2.0 is file-level copyleft.
- `r-efi` offers `LGPL-2.1-or-later` as one option of a three-way choice (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`); the permissive options apply.

The list comes from `cargo metadata` over the locked graph, all targets.

### Network code

No MyJanitor crate opens a network connection. The Tauri dependency tree does contain an HTTP client (`reqwest` and `hyper` appear in `Cargo.lock`, reached through `tauri`); the app never calls it, and the webview policy in `src-tauri/tauri.conf.json` allows no external origin. The Tauri capability is `core:default` only (no filesystem, dialog or shell plugin).

### Corrections to earlier notes

- `miniz_oxide`: an earlier note said it is the same version as Tauri's PNG stack. It is not: `Cargo.lock` holds 0.8.9 (ours) and 0.9.1 (through `flate2`, `png`, `tauri-codegen`). Both are small and compile quickly.
- `zune-jpeg` is pinned in `crates/dupes/Cargo.toml` and was not hoisted into the root `[workspace.dependencies]` table.
- The Tauri transitive counts above are higher than the earlier ones because build edges are now included.

### Considered and not added

| Need | Decision |
|---|---|
| Folder picker (`tauri-plugin-dialog` 2.8.2) | Not added (D-104): an in-app folder browser over the `fs_list_dir` command costs well under 5 h, keeps capabilities at `core:default` and avoids `tauri-plugin-fs`, `rfd` and their trees. |
| `mimalloc` | Not added; no allocator work was done. The large-tree benchmark (D-4I10) found its hot spot in the code. |
| `image`, `png`, `jpeg-decoder` | Not added. PNG decode is in-house on top of `miniz_oxide`; JPEG uses the smaller `zune-jpeg`. |
| `lopdf`, `pdf` | Not added (D-3B01). The PDF pass needs an object-level reader and writer under strict refuse-rather-than-repair rules; a general parser's recovery heuristics would have to be switched off. |
| `rayon`, `walkdir`, `tempfile`, `clap`, `glob`, `chrono`, `libc`, `windows-sys`, `sha2`, `twox-hash`, `zip`, `quick-xml` | Not added; see the next table. |

## 2. In-house replacements

Written instead of crates. Line counts are rough and include the tests.

| Need | Where | Size | Why in-house |
|---|---|---|---|
| xxHash64 | `crates/core/src/hash/xxh64.rs` | about 180 lines | A single pure function, checked against reference vectors. It is the cache and partial-hash key, so it must be stable and dependency-free |
| SHA-256 | `crates/core/src/hash/sha256.rs` | about 170 lines | Confirmation hash for duplicates and the audit log; streaming, reference vectors |
| ZIP reader and writer | `crates/formats/src/zip/` | about 1,800 lines | The writer must keep raw name bytes, local and central extra fields and entry order, regenerate ZIP64 records, and refuse data outside the entries. No crate exposes that control; verified by `unzip -t` and Python `zipfile` |
| XML tokenizer and tree | `crates/formats/src/xml.rs`, `ooxml/` | about 1,000 and 1,200 lines | A lossless tokenizer is needed so untouched bytes survive an Office edit; DOCTYPE rejected and size caps enforced |
| PDF reader and writer | `crates/formats/src/pdf/` | about 3,300 lines | Object model, xref tables and streams, object streams, predictors, two-stage verification; refuse rather than repair (D-3B01, D-3B02) |
| PNG decode and rewrite | `crates/formats/src/png.rs`, `crates/dupes/src/decode/png.rs` | about 800 and 320 lines | Chunk-level rewrite verified by inflated-stream equality; luma decode for hashing |
| JPEG header rewrite | `crates/formats/src/jpeg.rs`, `crates/optimise/src/images/jpeg_rewrite.rs` | n/a | Only headers before the first scan are rewritten and the scan bytes are copied verbatim |
| EXIF and TIFF reading | `crates/formats/src/exif.rs` | about 590 lines | Thumbnail, orientation, GPS and camera fields from the first bytes of a file |
| Compound File Binary (CFB) | `crates/formats/src/cfb.rs` | about 580 lines | Detects encrypted and labelled legacy containers; read-only |
| Media header parsers | `crates/formats/src/media/` | n/a | Duration and codec facts from MP4, MP3, FLAC, Ogg and RIFF headers without ffprobe |
| CRC-32, gzip validator, tar writer | `formats/src/crc32.rs`, `detect/src/inflate.rs`, `optimise/src/archive/tar.rs` | 70, 360, 300 lines | Small, and the gzip check needs a decoder that discards output |
| `.lnk` parser | `crates/detect/src/lnk.rs` | about 260 lines | Target path of a shortcut for the broken-shortcut detector |
| CLI argument parsing | `crates/cli/src/args.rs` | about 650 lines | One table drives parsing, validation, errors and `--help`, so they cannot disagree |
| Date maths | `crates/core/src/time.rs` | about 330 lines | Integer-nanosecond timestamps, calendar arithmetic and age parsing without `chrono` |
| Glob matching | `crates/core/src/glob.rs` | about 270 lines | `**`, case-insensitive paths, longest-literal pre-filter used on every candidate |
| Thread pool | `crates/core/src/pool.rs` | about 170 lines | Cancel and pause checkpoints per item, `catch_unwind` per worker, index-aligned results; replaces `rayon` |
| Platform FFI | `crates/platform/src/ffi/` and `scan/`, `action/` | hand-written | Direct `statx`, `statvfs`, `FICLONE`, and on Windows `FindFirstFileExW`, `DeviceIoControl`, `SHFileOperationW`, `Cf*`. Struct layouts are asserted at compile time and parsed by pure tested functions. No `libc` or `windows-sys` |
| Temporary directories for tests | `mj_core::testutil` (feature `testutil`) | small | Replaces `tempfile` |

## 3. Optional external tools (not linked, never downloaded)

MyJanitor finds these on `PATH` (or at a path set with `settings set tool_paths.<id> <ABSOLUTE-PATH>`) and shows the matching cards disabled, with the reason, when they are missing. On Windows only `.exe` and `.com` files count. Versions are probed with a 3 second timeout.

| Tool | Features it enables | Verified in the build session |
|---|---|---|
| `ffmpeg` and `ffprobe` | Video re-encode, screen recordings, meeting and audio recordings, waveform preview | Yes, against ffmpeg 6.1 |
| `cwebp`, `avifenc`, `cjxl` | Photos (lossy): WebP, AVIF and JPEG XL conversion | No, argument vectors only |
| `oxipng`, `jpegtran` | Optional extra engines for lossless PNG and JPEG (the in-house passes work without them) | No, argument vectors only |
| `7z` | 7z archives, archive listing and test | No, argument vectors only |
| `zstd` | `tar.zst` archives | No, argument vectors only |
| `gifsicle` | Detected and listed; there is no GIF optimiser yet | No |

UPX, SQLite tools, mailbox and virtual-disk tools and a transcription engine are not used by anything in this release.
