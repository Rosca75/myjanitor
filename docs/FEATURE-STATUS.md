# MyJanitor Feature Status

Final status of every feature ID in `docs/spec/file-cleaner-optimiser-features.md` (163 IDs, each exactly once). It describes the code at the commit that carries this file. Where a decision file and the code disagreed, the code was checked and wins. It was refreshed after the review-2 fixes, the T1 gap closure and the Windows CI work.

## Legend

**Tier** (build prompt 8.1): `T1` core, must be complete and tested; `T2` optimisation, should be complete; `T3` external-tool and advanced, built if time allowed, otherwise visible but disabled with a reason.

**Platform:** `Win+Linux`, `Windows only`, `Linux only`. macOS is never targeted. **Owner:** the workstream that built it, unchanged since the first build (W2-A scan, W2-B duplicates, W2-C filters and query, W2-D detectors, W2-E actions and safety, W2-F UI shell, W3-A Office and ZIP, W3-B PDF, archiving and file-system compression, W3-C images and media, W3-D engine facade and screens, W3-E settings, schedules, CLI).

**Status** (exactly one per ID):

| Status | Meaning |
|---|---|
| `done` | Engine, a GUI or CLI surface and tests exist, and it runs on the platforms it targets. A Win+Linux feature is `done` when its main path is tested on Linux in CI and works on Windows through a path that is tested there (`[WC]`) or through a tested fallback. An extra Windows-only source or call (a registry lookup, the NTFS fast path) may stay `[W]` if the note names it. A Windows-only feature is never `done` on `[W]` alone. |
| `partial` | Something works, something named in the note does not, or the feature is `Windows only` (or its real OS call is) and was never exercised against the real OS feature (`[W]`). |
| `disabled-with-reason` | The control exists and is shown disabled; the note gives the reason the UI shows (a missing tool, a missing codec, "not available yet"). |
| `not-targeted` | Not built in this release: no engine code and no UI entry. This also covers macOS parts. These are T3 features the build did not reach. |

**Verification level.** GitHub Actions (`.github/workflows/ci.yml`) runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace --no-fail-fast` and `node --test ui/` on `ubuntu-22.04` and on `windows-latest`, then builds the bundles on both. The last recorded run (run 43656957884, the state merged as PR 1) was green on all four jobs. The review-2 fixes and the gap closure came after it and had been gated locally on Linux and with the Windows-target clippy but had not had a CI run when this file was written, so `[WC]` notes on code that came with them (quarantine purge, for example) mean "the tests are not Unix-gated and will run on windows-latest", not yet "have run there". So the Windows build and the Windows test suite do run, and running them found five Windows product bugs that were fixed (`docs/decisions/W4-WIN.md`). Tests use generated fixture trees; the PDF pass was also run once over a corpus of 34 PDF files. What has not happened: nobody has operated the GUI on Windows or on Linux beyond an xvfb start-up smoke run (W4-INT D-4I11), and no Windows-only OS feature was exercised against the real thing.

Windows-specific code is described at one of three levels:

| Level | Marker | Meaning |
|---|---|---|
| (a) | `[WC]` | Tested on Windows in CI: the code path is exercised by tests that run on `windows-latest` (checked: not `cfg(unix)`-gated). Those tests build their fixtures on the runner's real NTFS volume. |
| (b) | `[W]` | Compiled and linted on Windows in CI, logic unit-tested on hand-built data, not exercised against the real OS feature. Examples: WOF compression, OneDrive placeholders and dehydrate, the real Recycle Bin, `schtasks` and registry installation, the NTFS fast path on a large real volume. These rows stay `partial` when the feature is Windows only. |
| (c) | | Not built: the `not-targeted` rows. |

Other markers in the notes:

| Marker | Level |
|---|---|
| (none) | Unit and integration tests, run on Linux in CI and, unless gated to Unix, on Windows in CI. |
| `[X]` | Also run against the real external tool (ffmpeg and ffprobe 6.1) in the build session. Those tests skip themselves when ffmpeg is absent. |
| `[A]` | External tool adapter checked by argument-vector tests only. The tool was not installed in the build session, so it has never been run for real. |

Notes starting with "Requires ffmpeg on PATH" describe the reason the card shows when the tool is missing. Findings named "review F-nn" refer to `docs/review/adversarial-review-1.md` and "G-nn" to `docs/review/adversarial-review-2.md`; all are fixed (F-12 is accepted, D-012), see `docs/review/adversarial-review-1-fixes.md` and `docs/review/adversarial-review-2-fixes.md`.

## Scan and analysis (`SCAN`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| SCAN-01 | Select scan scope: drives, folders, several roots | T1 | Win+Linux | W2-A | done | Drives and any number of folder roots; in-app folder browser (no native dialog). CLI refuses a filesystem root or the home folder as scope. |
| SCAN-02 | Include / exclude paths | T1 | Win+Linux | W2-A | done | Exclude globs and absolute paths (gitignore style), include patterns, depth limit, hidden and one-file-system switches (Settings > Scanning, used by app and CLI scans alike); CLI `--exclude`. |
| SCAN-03 | Fast indexing (NTFS MFT read, fallback directory walk) | T1 | Win+Linux | W2-A | done | Parallel directory walk (statx on Linux, FindFirstFileExW on Windows; 200,000 files in 0.5 s on Linux); the walk runs in the scan tests on windows-latest [WC]. The NTFS USN/MFT fast path is Windows only, taken only when a location is a whole drive (`C:\`; a folder is walked), and falls back to the walk on any error: its record parsers are unit-tested on hand-built records [W], and W4-WIN reports that the Windows test runs used it on an elevated runner (that is how the link-count bug D-W3 was found). No test asserts that the fast path was taken, and it was not run on a large real volume. |
| SCAN-04 | Scan cache for near-instant rescans (metadata + hashes) | T1 | Win+Linux | W2-A | done | One global cache of stamps and hashes with pruning and a trust window for files changed in the last 2 s; 200,000-file warm rescan about 0.8 s. |
| SCAN-05 | Treemap / sunburst view of disk usage | T2 | Win+Linux | W3-D | done | Disk usage view with folder tree and treemap (no sunburst), by size, age, type; real data through the disk tree commands. |
| SCAN-06 | Folder tree sorted by size | T1 | Win+Linux | W2-F | done | Folder tree sorted by size with subtree totals (200,000 entries in 49 ms). |
| SCAN-07 | Breakdown by file type, age, owner | T2 | Win+Linux | W3-D | done | Breakdown by type, extension and age; owner shows `uid:N` on Linux and `unknown` on Windows (owner is not collected there). |
| SCAN-08 | Allocated vs actual size, NTFS compression rate | T1 | Win+Linux | W2-A | done | Allocated and logical size on both OSes, compression ratio per folder. The Windows size queries (FindFirstFileExW, GetCompressedFileSizeW) run in the scan tests on windows-latest [WC]; a file actually compressed by NTFS was not tested. |
| SCAN-09 | Cloud placeholder awareness (OneDrive/Dropbox online-only files excluded from counts) | T1 | Windows only | W2-A | partial | [W] Windows only. Placeholder attributes and reparse tags are read, and online-only placeholders are never opened or counted; the logic is tested on hand-built attribute values and was never run against a real OneDrive or Dropbox placeholder. Linux: not applicable. |
| SCAN-10 | Live estimate of reclaimable space per filter and per action | T1 | Win+Linux | W2-E | done | Per-card, per-plan and exact dry-run estimates from allocated sizes; hard-link siblings are not double counted. |
| SCAN-11 | Scan progress, pause, stop | T1 | Win+Linux | W2-A | done | Progress events, pause, resume and stop; stopping publishes nothing stale. |

## Metadata filters (`CLN-FLT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-FLT-01 | Name / path: wildcard, contains, regex | T1 | Win+Linux | W2-C | done | Wildcard, contains, exact and regex on name and path (regex limits: 1024 characters, 1 MiB compiled). |
| CLN-FLT-02 | Extension and editable file-type groups (Audio, Video, Office, Archives, Installers, System...) | T1 | Win+Linux | W2-C | done | Extension and type groups; user-defined groups come from the settings file only (no editor in the UI). |
| CLN-FLT-03 | Size: min/max, top N largest, zero-byte | T1 | Win+Linux | W2-C | done | Size ranges, zero-byte, top N largest. |
| CLN-FLT-04 | Dates: created, modified, last accessed, absolute or relative ("older than 2 years") | T1 | Win+Linux | W2-C | done | Created, modified, accessed; absolute dates and ages (`modified:>2y`). |
| CLN-FLT-05 | Attributes: hidden, system, read-only, compressed, sparse, encrypted, offline | T1 | Win+Linux | W2-C | done | Hidden, read-only, compressed, sparse, encrypted, offline mapped from statx and dot-names on Linux. Windows attribute flags come from FindFirstFileExW data [W] and the mapping is unit-tested on hand-built flags. 'System' has no Linux equivalent. |
| CLN-FLT-06 | Owner / user | T1 | Win+Linux | W2-C | partial | Linux: `uid:N` or the user name (read in-house from `/etc/passwd`) matches an `owner:` filter; results still display `uid:N`. Windows owner (SID) is not collected, so owner filters never match there. |
| CLN-FLT-07 | Directory depth min/max | T1 | Win+Linux | W2-C | done | Depth min and max. |
| CLN-FLT-08 | Path length and invalid characters | T1 | Win+Linux | W2-C | done | Path length and invalid Windows characters, reserved names and trailing dot or space. |

## Content-based filters (`CLN-CNT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-CNT-01 | Full-text content search, including PDF, Word, Excel | T2 | Win+Linux | W2-C | partial | Plain text (UTF-8, UTF-16, legacy 8-bit) plus best-effort extractors for PDF, DOCX/DOCM, XLSX/XLSM and PPTX/PPTM, all capped at 8 MiB. The PDF extractor reads simple single-byte fonts only: a PDF with CID or Identity-H fonts yields no text and so reports no match. Legacy `.doc`, `.xls`, `.ppt` and OpenDocument files have no extractor and are reported as unavailable, never as 'no match'. Tested on generated files. |
| CLN-CNT-02 | True file type vs extension mismatch | T1 | Win+Linux | W2-C | done | Header sniffing for about 35 content kinds; empty files and unchecked extensions are never mismatches. |
| CLN-CNT-03 | Media metadata: EXIF (camera, date taken, GPS), resolution, duration, codec, bitrate, audio tags | T2 | Win+Linux | W2-C | partial | In-house EXIF and MP4, MP3, FLAC, Ogg and RIFF header parsers feed a `media` rule condition (rule JSON only). No quick-query key and no rule-builder field in the UI; ffprobe is not used. |

## Smart detectors (`CLN-DET`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-DET-01 | Exact duplicates (staged: size, partial hash, full hash) | T1 | Win+Linux | W2-B | done | Staged size, head/middle/tail 64 KB partial hash, full xxHash64 plus SHA-256 confirmation; hard links and online-only files never reported as duplicates, and a copy with other hard links promises no space and is never pre-selected (it is kept in preference); keep rules. Detectors running together read each file once per stage. |
| CLN-DET-02 | Similar images (perceptual hash, similarity threshold) | T2 | Win+Linux | W2-B | done | dHash and pHash on JPEG (EXIF thumbnail or bounded zune-jpeg decode) and PNG; a thumbnail match is always confirmed on the decoded main picture, and flat images never match. HEIC, RAW and WebP are not decoded; interlaced PNG is exact-match only; EXIF orientation is not applied. Only near-identical groups are pre-selected. |
| CLN-DET-03 | Similar videos | T3 | Win+Linux | W3-C | not-targeted | Not built: no detector and no UI entry. |
| CLN-DET-04 | Same music (tags or audio fingerprint) | T3 | Win+Linux | W3-C | not-targeted | Not built: no detector and no UI entry. |
| CLN-DET-05 | Duplicate meeting recordings (same meeting from several sources, audio fingerprint) | T3 | Win+Linux | W3-C | not-targeted | Not built: no detector and no UI entry. |
| CLN-DET-06 | Empty files and empty folders (including nested empty trees) | T1 | Win+Linux | W2-D | done | Empty files, and nested empty folders reported topmost and re-checked on disk; `.git` and marker files are never proposed. |
| CLN-DET-07 | Zeroed files (filled with zeros, usually corrupted) | T1 | Win+Linux | W2-D | done | Head, middle and tail sampling then streaming confirmation up to 256 MB; VM and swap images are skipped. |
| CLN-DET-08 | Broken / corrupted files (images, audio, PDF, archives) | T1 | Win+Linux | W2-D | done | Structural checks for PNG, JPEG (end-of-image marker, found by a marker walk when the tail has none, so motion photos pass), PDF, ZIP and gzip, and for audio: WAV/RIFF sizes (RF64 accepted), FLAC STREAMINFO (after an optional ID3 tag), MP3 audio after its ID3v2 tag, Ogg page magic, then the media header parser. Heuristics, review only, never pre-selected. |
| CLN-DET-09 | Invalid symlinks and broken shortcuts (.lnk) | T1 | Win+Linux | W2-D | done | Broken symlinks on both OSes (the symlink tests are Unix-only, so on Windows that part is [W]); a target on an absent drive letter, a network path or a missing or empty Unix mount point is skipped, never reported (rules unit-tested with an injected filesystem). The `.lnk` shortcut check runs only on Windows ([W], parser unit-tested on hand-built shortcuts) and only for drives that exist. |
| CLN-DET-10 | Temporary and partial files (.tmp, ~$ Office locks, .crdownload, .part, .bak) | T1 | Win+Linux | W2-D | done | `.tmp`, `~$` Office locks, `.crdownload`, `.part`, `Thumbs.db`; backups and partial downloads are listed but not pre-selected. |
| CLN-DET-11 | App caches and junk, winapp2.ini compatible | T1 | Win+Linux | W2-D | done | Rule table by path glob plus winapp2.ini from a user-supplied path (never shipped); Windows rules are inert on Linux. |
| CLN-DET-12 | Office leftovers: AutoRecover / UnsavedFiles folders, Teams and Outlook local caches | T1 | Windows only | W2-D | partial | [W] Windows only: the card is shown as unavailable on Linux. Path rules are tested on fake folder trees (those tests also run on windows-latest) but never against a real Outlook, Teams or Office profile. |
| CLN-DET-13 | Developer artefacts: node_modules, .venv, target/, bin/obj, __pycache__, .gradle, Docker build... | T1 | Win+Linux | W2-D | done | node_modules, .venv, target, bin/obj, __pycache__, .gradle and others, each needing its proof file; Docker cache is a note only. |
| CLN-DET-14 | Old installers and disk images (.msi, .exe, .iso in Downloads older than X) | T1 | Win+Linux | W2-D | done | Installers and disk images in Downloads older than N months; `.exe` and `.AppImage` are never pre-selected. |
| CLN-DET-15 | Orphan sidecar files (.xmp, .aae, .thm without parent), RAW+JPEG pairs | T1 | Win+Linux | W2-D | done | Orphan `.xmp`, `.aae`, `.thm` sidecars (review only) and RAW+JPEG pairs (separate card). |
| CLN-DET-16 | Unused language packs / localisations | T1 | Win+Linux | W2-D | done | Locale folders and files recognised by ISO 639-1 code; review only; kept-languages setting, system language always kept (Windows: the user's locale from `GetUserDefaultLocaleName`, not run here [W]; elsewhere `LC_ALL`/`LC_MESSAGES`/`LANG`); when it is unknown the card says only English is kept. |
| CLN-DET-17 | Stale files (not accessed in N months), feeds archiving | T1 | Win+Linux | W2-D | done | Last used = later of access and modification time; 12 months and 1 MiB defaults; review only, feeds archiving. |
| CLN-DET-18 | Version chains: keep latest, review others | T2 | Win+Linux | W2-B | done | Marker rules for copy numbers, v1/v2, final/draft and dates; review only with nothing pre-selected. |
| CLN-DET-19 | Derived files (PDF exported from a .pptx/.docx still present in the same folder) | T2 | Win+Linux | W2-B | done | PDF next to its source .docx/.pptx in the same folder; keeper is the source document. |
| CLN-DET-20 | Email attachment copies in Downloads that duplicate synced SharePoint/OneDrive files | T2 | Win+Linux | W2-B | done | Downloads copies of files in a synced folder; roots come from detected sync clients (Windows registry detection [W]) with name heuristics as fallback. |
| CLN-DET-21 | Closed engagements / inactive project folders (inactive for N months) | T1 | Win+Linux | W2-D | done | Direct children of a scan root with no change for N months; personal and OS folders are skipped. |

## Filter engine (`CLN-ENG`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-ENG-01 | Combine filters with AND / OR | T1 | Win+Linux | W2-C | done | AND / OR / NOT rule trees with three-valued evaluation: unreadable files are never selected. |
| CLN-ENG-02 | Saved presets / templates ("Downloads older than 90 days", "Dev junk", "Consulting cleanup") | T1 | Win+Linux | W2-C | done | 13 built-in presets plus user presets (save, apply, delete) in the Custom search view. |
| CLN-ENG-03 | Global exclusions / whitelist (protected folders, types, system paths) | T1 | Win+Linux | W2-C | done | Built-in protected folders and patterns, user protected folders and exclusions edited in Settings. Protection also holds for entries below a folder that is removed (see SAF-01). An absolute entry is the folder of that exact name even when it contains `[` (W6-B, RF-2). |
| CLN-ENG-04 | Topmost match only (report parent folder instead of every child) | T1 | Win+Linux | W2-C | done | Topmost-only option in custom search and in the folder detectors. |
| CLN-ENG-05 | Quick search syntax (e.g. `ext:mp4 size:>1GB modified:<2024`) | T1 | Win+Linux | W2-C | done | Parser with offsets in errors, autocomplete and about 20 keys (`ext name path size modified older newer type attr is owner depth pathlen content regex top`); empty query is an error. |

## Cleaning actions (`CLN-ACT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-ACT-01 | Smart auto-select for duplicates: keep newest, oldest, shortest path, highest resolution,... | T1 | Win+Linux | W2-E | done | Keep newest, oldest, shortest path or highest resolution; one keeper per group, never pre-selected. 'Select all' keeps the kept copy unchecked, and a later step of the same plan never removes a copy an earlier step kept (reviews F-02 and G-03, fixed). |
| CLN-ACT-02 | Delete to Recycle Bin (default) | T1 | Win+Linux | W2-E | partial | Linux: real freedesktop Trash with exact restore. Windows: the default route is MyJanitor's own quarantine, because the Recycle Bin gives no restore handle; exact restore from it runs on windows-latest [WC]. The setting `general.windows_recycle_bin` (Settings, off by default) switches Windows to the real Recycle Bin, and the step and dialog wording follows `app_info.trash_route`. That route is [W]: it was never exercised against the real shell, and its restore is best effort. |
| CLN-ACT-03 | Permanent delete with confirmation | T1 | Win+Linux | W2-E | done | Always asks first: confirmation dialog, `--confirm-permanent` in the CLI; skipped without confirmation; only under 'Advanced' in the GUI. |
| CLN-ACT-04 | Quarantine folder, restorable, auto-purge after N days | T1 | Win+Linux | W2-E | done | Quarantine with exact restore (runs on windows-latest [WC]). Expired items are purged automatically, each by its own retention days: at GUI start-up, at the start of acting CLI commands (`apply`, `clean --apply`, `undo`, `run-profile`, `quarantine purge`) and after each applied step. CLI `quarantine list` and `quarantine purge [--older-than DAYS]`; History keeps 'Purge old items'. A read-only CLI command never purges. |
| CLN-ACT-05 | Move / copy to another location, keeping folder structure | T1 | Win+Linux | W2-E | done | Move and Copy in the GUI (step panel, in-app folder browser) with 'Keep folder structure' (default on), and in the CLI (`plan add --action move` or `copy` with `--to DIR` and optional `--flat`). Destinations that are relative, protected or inside a scanned location are refused. A cross-volume move copies, verifies, re-checks the source, then deletes entry by entry; alternate data streams and ACLs are not carried on Windows (D-012). |
| CLN-ACT-06 | Replace duplicates with hardlinks (NTFS) or reflinks (ReFS, APFS, Btrfs) | T2 | Win+Linux | W2-E | partial | Hard links work on both OSes (byte-compared, undoable, confirmation required, never scheduled) from the GUI duplicates step and the CLI (`--action hardlink`); CreateHardLinkW runs in the tests on windows-latest [WC]. Reflinks use FICLONE on Linux and are offered only where the capability exists; the flow is tested with a simulated clone and no real Btrfs or XFS volume was used. Windows ReFS cloning is not implemented. |
| CLN-ACT-07 | OneDrive "Free up space": turn local copy into online-only instead of deleting | T2 | Windows only | W2-E | partial | [W] Windows only: dehydrate through the Cloud Files API was never exercised against a real OneDrive. Reached through the 'online-only instead' choice in the confirmation dialog; no card of its own. |
| CLN-ACT-08 | Secure shred files | T3 | Win+Linux | W2-E | not-targeted | Not built: no action and no UI entry. |
| CLN-ACT-09 | Wipe free space | T3 | Win+Linux | W2-E | not-targeted | Not built: no action and no UI entry. |
| CLN-ACT-10 | Vacuum application databases (browser SQLite) | T3 | Win+Linux | W2-E | not-targeted | Not built: no action and no UI entry. |
| CLN-ACT-11 | Export results (CSV, HTML, PDF) | T1 | Win+Linux | W2-E | done | CSV, HTML and PDF export of results and run reports (GUI buttons; CLI `report --format` and `show ID --format` with csv, html or pdf, and `--out FILE`), plus an audit CSV. The PDF is a plain text table (Helvetica, WinAnsi): characters outside that set print as `?` and the export says how many. |

## Images (`OPT-IMG`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-IMG-01 | Lossless recompression (PNG oxipng, JPEG jpegtran/MozJPEG, GIF gifsicle) | T2 | Win+Linux | W3-C | partial | PNG re-deflate and JPEG header rewrite are in-house and verified (inflate equality, scan bytes copied). oxipng and jpegtran are optional and [A] argument-tested only; their output is accepted only when it holds every kept chunk or segment and the data after IEND/EOI (re-appended, fake-tool tested). No GIF optimiser (gifsicle argument builder only). |
| OPT-IMG-02 | Lossy recompression with quality slider and side-by-side preview | T3 | Win+Linux | W3-C | disabled-with-reason | Requires cwebp, avifenc or cjxl on PATH. Quality setting exists; [A] adapters argument-tested only, never run against a real encoder. No side-by-side preview. |
| OPT-IMG-03 | Format conversion: PNG to WebP, JPEG to AVIF, lossless JPEG to JPEG XL (reversible) | T3 | Win+Linux | W3-C | disabled-with-reason | Requires cwebp, avifenc or cjxl on PATH. PNG/JPEG to WebP, AVIF or JPEG XL; [A] argument-tested only, never run against a real encoder. |
| OPT-IMG-04 | Resize to max long / short edge | T3 | Win+Linux | W3-C | disabled-with-reason | Requires cwebp on PATH, and only WebP output can resize; AVIF and JXL refuse it. [A] argument-tested only. |
| OPT-IMG-05 | Metadata control: strip all, keep all, strip GPS only, thumbnails, ICC profiles | T2 | Win+Linux | W3-C | partial | JPEG policies (keep all, strip thumbnail, strip GPS, strip all) and PNG chunk stripping. Comments are removed only by strip all; data after the picture is kept by every policy. The thumbnail and GPS strips never zero bytes shared with other EXIF structures, and refuse an unsafe layout (G-12). A GPS-only run is reachable only from a step that is already listed, not as its own card. |
| OPT-IMG-06 | Overwrite policy: always, never, only if smaller | T2 | Win+Linux | W3-A | partial | Replace the original or write a `_light` copy. 'Keep if not smaller' is always on and cannot be switched off, so 'always overwrite' is not offered. |

## Video (`OPT-VID`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-VID-01 | Re-encode to HEVC or AV1 with quality-based CRF | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. HEVC re-encode [X] run against real ffmpeg 6.1; AV1 only when the encoder is present. |
| OPT-VID-02 | Hardware encoders (NVENC, Quick Sync, AMF, VideoToolbox) with estimated encode time | T3 | Win+Linux | W3-C | partial | Requires ffmpeg. Encoders are detected and trial-encoded before use, but NVENC, QSV and AMF were never run on real hardware. No VAAPI option on Linux; VideoToolbox not targeted. |
| OPT-VID-03 | Downscale resolution and cap frame rate | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Max height and frame-rate cap [X] run against real ffmpeg. |
| OPT-VID-04 | Drop extra audio tracks and subtitles | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Keep first audio track, drop subtitles; [A] argument-tested, shares the [X] encode path. |
| OPT-VID-05 | Remux only (no re-encode), strip junk streams | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Remux preset without re-encode keeps every track (attachments, timecode) and checks the output's tracks and length with `ffmpeg -i`; a file with a track the output cannot carry (GPS/camera data, cover picture, second video) is skipped. Real-ffmpeg tested on Linux. |
| OPT-VID-06 | Compatibility warning for target codec | T3 | Win+Linux | W3-C | done | Requires ffmpeg. Compatibility text for HEVC and AV1 shown with the codec choice and in the step notes. |

## Screen recordings (`OPT-SCR`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-SCR-01 | Auto-detect screen content (low motion: slides, apps, code) and apply dedicated preset | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Heuristic on bits per pixel, common screen sizes and recorder names; no frame analysis. |
| OPT-SCR-02 | Lower frame rate (down to 5 to 15 fps), keep resolution for text legibility | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. 5 to 15 fps at full resolution. |
| OPT-SCR-03 | AV1 screen-content coding mode (intra block copy, palette) | T3 | Win+Linux | W3-C | partial | Requires ffmpeg. Screen-content flags are used only after a trial encode succeeds; no test runs them against a real encoder. |
| OPT-SCR-04 | Trim dead time (static or silent start/end segments) | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Dead-time detection and trim [X] run against real ffmpeg. |
| OPT-SCR-05 | Derive lighter outputs: audio only, or key frames to slide PDF, then archive/delete the video | T3 | Win+Linux | W3-C | partial | Requires ffmpeg. Audio-only Opus derivative [X] works; key frames to a slide PDF is not offered (no image-to-PDF writer). |

## Audio and meeting recordings (`OPT-AUD`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-AUD-01 | WAV/AIFF to FLAC (lossless) or Opus/AAC (lossy) | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Opus and FLAC [X] run against real ffmpeg; AAC [A] argument-tested. |
| OPT-AUD-02 | Recompress FLAC at maximum level | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. FLAC output always uses level 12 [X]. |
| OPT-AUD-03 | Strip or downsize embedded cover art | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Strip or shrink cover art; [A] argument-tested. |
| OPT-AUD-04 | Speech preset: Opus mono about 24 kbps | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Speech preset (Opus 24 kbps mono) [X]. |
| OPT-AUD-05 | Downmix to mono for voice-only recordings | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Mono downmix, part of the speech preset [X]. |
| OPT-AUD-06 | Remove long silences (voice activity detection, max pause setting) | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Silence removal with max-pause setting and a waveform preview of removable parts [X]. |
| OPT-AUD-07 | Transcription-safe profile (avoid over-compression when transcripts are needed) | T3 | Win+Linux | W3-C | done | Requires ffmpeg on PATH. Transcription-safe preset (Opus 48 kbps, silences kept). |
| OPT-AUD-08 | Transcribe locally, then archive/delete audio by retention rule | T3 | Win+Linux | W3-C | disabled-with-reason | Requires a local transcription tool (none is bundled); the toggle is shown disabled with that reason. |

## PDF (`OPT-PDF`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-PDF-01 | Image downsampling by target DPI | T3 | Win+Linux | W3-B | disabled-with-reason | Needs an image codec that is not bundled; the 'PDF images' card is shown unavailable with that reason. |
| OPT-PDF-02 | Font subsetting and duplicate font removal | T3 | Win+Linux | W3-B | disabled-with-reason | Needs a TrueType/CFF subsetter that is not bundled; shown unavailable. Byte-identical duplicate font streams are merged by the lossless pass. |
| OPT-PDF-03 | Lossless structural optimisation (object streams, stream recompression, unused objects) | T2 | Win+Linux | W3-B | done | In-house reader and writer: reachable objects only, deflate level 10, object and xref streams, identical streams merged; two-stage verification. 25 of 34 corpus PDFs optimised (40.6% average), the rest skipped (encrypted or malformed). |
| OPT-PDF-04 | Presets: Screen, eBook, Prepress | T3 | Win+Linux | W3-B | disabled-with-reason | Needs an image codec that is not bundled; presets are shown unavailable with that reason. |
| OPT-PDF-05 | Remove dead weight: metadata, thumbnails, edit history, attachments, JavaScript | T2 | Win+Linux | W3-B | done | Thumbnails on by default; XMP, Info, attachments, JavaScript and PieceInfo are opt-in and lossy. Old revisions disappear because only reachable objects are written. Signed and encrypted PDFs are skipped. |

## PowerPoint (`OPT-PPT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-PPT-01 | Compress / downsample embedded pictures (96 / 150 / 220 ppi) | T2 | Win+Linux | W3-A | disabled-with-reason | Needs an image or video encoder that is not bundled; the setting is shown disabled. |
| OPT-PPT-02 | Delete cropped areas of pictures (irreversible, opt-in) | T2 | Win+Linux | W3-A | disabled-with-reason | Needs an image or video encoder that is not bundled; the setting is shown disabled. |
| OPT-PPT-03 | Delete picture editing history | T2 | Win+Linux | W3-A | disabled-with-reason | Could not be validated against PowerPoint, so it is kept off; the setting is shown disabled. |
| OPT-PPT-04 | Remove unused slide masters and layouts | T2 | Win+Linux | W3-A | done | Unused layouts and unused masters, both opt-in and lossy (confirmation required, not schedulable); templates (`.potx`, `.potm`) are skipped with a note; at least one master kept; verified against the source package. |
| OPT-PPT-05 | Subset or unembed embedded fonts | T2 | Win+Linux | W3-A | partial | Unembedding fonts works (opt-in, lossy). Subsetting needs a font subsetter that is not bundled and is shown disabled. |
| OPT-PPT-06 | Embedded Excel chart data: remove, convert chart to static picture, or keep | T2 | Win+Linux | W3-A | partial | Removing embedded chart workbooks works (opt-in, lossy). Converting charts to static pictures needs a renderer that is not bundled and is shown disabled. |
| OPT-PPT-07 | Convert large EMF/WMF pictures to PNG/JPEG at chosen DPI | T2 | Win+Linux | W3-A | disabled-with-reason | Needs a vector renderer that is not bundled; the setting is shown disabled. |
| OPT-PPT-08 | Deduplicate identical media inside the package | T2 | Win+Linux | W3-A | done | Identical parts under `/media/` merged inside the package (default on); embedded objects are never merged (G-11); verified against the source package. |
| OPT-PPT-09 | Re-encode embedded video/audio, or extract and link | T3 | Win+Linux | W3-A | disabled-with-reason | Embedded media re-encoding inside packages is not available yet; the setting is shown disabled. |
| OPT-PPT-10 | Optional removals: hidden slides, speaker notes, comments, custom XML (off by default) | T2 | Win+Linux | W3-A | done | Hidden slides, speaker notes, comments and custom XML, all off by default and lossy; never on labelled files or slides other slides link to. |

## Excel (`OPT-XLS`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-XLS-01 | Bloat diagnosis: weight by used range, pivot caches, styles, images, objects | T2 | Win+Linux | W3-A | done | Weight by category (media, caches, styles, sheets) in the review table and the size-parts bar. |
| OPT-XLS-02 | Reset inflated used range on every sheet | T2 | Win+Linux | W3-A | done | Opt-in and lossy: empty formatted rows and cells after the last value are removed; bounds respect merges, validations, anchors and tables. |
| OPT-XLS-03 | Pivot caches: stop saving source data, merge identical caches | T2 | Win+Linux | W3-A | partial | Stops saving pivot cache records (opt-in, worksheet sources only). Merging identical caches is not implemented. |
| OPT-XLS-04 | Remove unused cell styles (imported from templates) | T2 | Win+Linux | W3-A | partial | Unused named styles only, skipped on templates (`.xltx`, `.xltm`). Pruning cell formats is disabled because other parts may reference them. |
| OPT-XLS-05 | Narrow whole-column conditional formatting, remove duplicate rules | T2 | Win+Linux | W3-A | partial | Exact duplicate rules and empty blocks are removed (opt-in). Narrowing whole-column ranges is disabled (needs formula-aware rewriting). |
| OPT-XLS-06 | Remove broken / unused named ranges, stale external links, unused query connections | T2 | Win+Linux | W3-A | partial | Broken `#REF!` names that nothing uses are removed (opt-in). External links are only counted in the report; unused query connections are not handled. |
| OPT-XLS-07 | Report hidden sheets (review only, no auto-delete) | T2 | Win+Linux | W3-A | done | Hidden and very hidden sheets are listed in the notes and review column; never removed. |
| OPT-XLS-08 | Compress embedded pictures | T2 | Win+Linux | W3-A | disabled-with-reason | Needs an image or video encoder that is not bundled; the setting is shown disabled. |
| OPT-XLS-09 | Optional conversion to .xlsb, with compatibility warning | T3 | Win+Linux | W3-A | disabled-with-reason | MyJanitor does not write the binary .xlsb format; the setting is shown disabled. |
| OPT-XLS-10 | Macro warning: used-range reset may break VBA relying on UsedRange | T2 | Win+Linux | W3-A | done | Macro warning in the used-range setting help, the step notes and a `macros` column. |

## Word (`OPT-DOC`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-DOC-01 | Compress pictures (cropped areas, editing history, DPI) | T2 | Win+Linux | W3-A | disabled-with-reason | Needs an image or video encoder that is not bundled; the setting is shown disabled. |
| OPT-DOC-02 | Convert embedded objects (Excel tables, Visio, EMF) to pictures or keep | T2 | Win+Linux | W3-A | partial | Removing embedded chart workbooks works (opt-in). Converting embedded objects to pictures needs a renderer that is not bundled and is shown disabled. |
| OPT-DOC-03 | Subset or remove embedded fonts | T2 | Win+Linux | W3-A | partial | Unembedding fonts works (opt-in). Subsetting needs a font subsetter that is not bundled and is shown disabled. |
| OPT-DOC-04 | Report tracked changes and comments weight (never auto-accept) | T2 | Win+Linux | W3-A | done | Counts insertions, deletions and comments with their bytes; never accepts or removes them. |
| OPT-DOC-05 | Remove large attached templates and custom XML parts | T2 | Win+Linux | W3-A | done | Removes the attached template reference and custom XML (opt-in); never on labelled files; bound, bibliography and SharePoint data kept. |

## Office packages (`OPT-OFC`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-OFC-01 | Repack ZIP container at maximum Deflate level | T2 | Win+Linux | W3-A | done | In-house ZIP writer, smallest of stored, deflate level 10 or original per entry; verified repack; signed, encrypted, legacy and labelled files skipped or protected. |
| OPT-OFC-02 | Remove embedded thumbnails and revision data | T2 | Win+Linux | W3-A | partial | Embedded thumbnails are removed (default on). Revision data (tracked changes, comments) is reported but deliberately never removed. |
| OPT-OFC-03 | Lossless recompression of other ZIP-based formats (EPUB, CBZ, APK/JAR, ODT) | T2 | Win+Linux | W3-A | done | EPUB, CBZ, JAR and ODF repack; APKs are refused on purpose (v2 signatures and zipalign cover the whole file); signed JARs are repacked; a JAR's stored entries stay stored (nested jars). |

## System fonts (`OPT-FNT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-FNT-01 | Detect duplicate installed fonts (user and system folders) | T2 | Win+Linux | W2-B | done | Duplicate installed fonts across user and system folders (hash match); keeper prefers the system folder. |
| OPT-FNT-02 | Detect unused font families and font caches (review only) | T2 | Win+Linux | W2-D | partial | Lists font cache files and folders (review only). Unused font families cannot be known offline, and the notes say so. |

## Archiving (`OPT-ARC`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-ARC-01 | Archive cold folders (not accessed in N months) to 7z / ZIP / zstd | T2 | Win+Linux | W3-B | done | Cold folders to a native ZIP, verified, with a sidecar index. 7z and tar.zst need 7z or zstd on PATH. The 7z path ran for real in CI (7-Zip 23 and 7-Zip for Windows), which found two bugs, both fixed and both failing safe (W4-WIN D-W4, D-W7); zstd is [A] argument-tested only. A folder holding links (or, for 7z, empty folders) is skipped when the originals would be removed (G-14). |
| OPT-ARC-02 | Integrity test before deleting source | T2 | Win+Linux | W3-B | done | Mandatory: every file is re-read and hashed against the archive before any source is removed; there is no option to skip it. |
| OPT-ARC-03 | Recompress existing archives with stronger algorithm or solid mode | T2 | Win+Linux | W3-B | partial | Plain `.zip` files are recompressed and checked entry by entry. ZIP to 7z or solid mode is disabled because the result would keep a `.zip` name. |
| OPT-ARC-04 | Archive closed engagements as one file with searchable index | T2 | Win+Linux | W3-B | done | `INDEX.csv` (path, size, mtime, SHA-256) is the first entry of every archive, with a sidecar beside it; undo leaves the sidecar behind. |
| OPT-ARC-05 | Optional AES-256 encryption, split volumes, compression level | T3 | Win+Linux | W3-B | disabled-with-reason | Encryption and split volumes are shown disabled: 7z takes a password only on its command line, where other processes can read it. Compression level works. |

## File-system compression (`OPT-FSC`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-FSC-01 | NTFS WOF compression per folder: XPRESS4K, XPRESS8K, XPRESS16K, LZX | T2 | Windows only | W3-B | partial | [W] Windows only: WOF through DeviceIoControl; buffer layouts unit-tested on hand-built data; never exercised against a real NTFS volume. |
| OPT-FSC-02 | Skip poorly compressible files | T2 | Windows only | W3-B | partial | [W] Windows only for WOF. The skip logic (extension table and a deflate probe on up to four 64 KiB blocks) runs on every OS in CI; WOF itself was not exercised on Windows. |
| OPT-FSC-03 | Background watcher to recompress after updates | T2 | Windows only | W3-E | disabled-with-reason | Background watcher is not built; the setting is shown disabled ('Not available yet'). |
| OPT-FSC-04 | DirectStorage warning on game folders | T2 | Windows only | W3-B | partial | [W] Windows only. The marker-based warning for steamapps, Epic, Xbox and GOG folders is unit-tested on fake folders; the card itself was never run on a Windows machine. |
| OPT-FSC-05 | Decompress / revert | T2 | Windows only | W3-B | partial | [W] Windows only for WOF: never exercised against a real volume. Btrfs revert not exercised on a real Btrfs volume. |
| OPT-FSC-06 | macOS APFS compression, Btrfs zstd equivalents | T2 | Linux only | W3-B | partial | Linux Btrfs zstd (xattr plus defrag) not exercised on a real Btrfs volume; the card is disabled elsewhere. APFS not targeted. Btrfs zlib/lzo not reported. |

## Other formats (`OPT-OTH`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-OTH-01 | Mailbox compaction (OST/PST) | T3 | Windows only | W3-B | not-targeted | Not built: no optimiser and no UI entry. |
| OPT-OTH-02 | Virtual disk compaction (VHDX/VMDK) | T3 | Windows only | W3-B | not-targeted | Not built: no optimiser and no UI entry. |
| OPT-OTH-03 | Executable packing (UPX), advanced only, antivirus false-positive warning | T3 | Win+Linux | W3-C | not-targeted | Not built: no optimiser and no UI entry (UPX is not in the tool list). |

## Optimisation engine (`OPT-ENG`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-ENG-01 | Lossless / lossy switch, global and per file type | T2 | Win+Linux | W3-A | partial | Lossy work is opt-in per step: lossy presets and toggles mark a step lossy and require confirmation. The global `lossless_only` setting and per-type switch exist in the framework but are not connected to settings or the UI. |
| OPT-ENG-02 | Content-based file type detection | T1 | Win+Linux | W3-A | done | Content sniffing with ZIP directory refinement routes files to optimisers, so a renamed file reaches the right one. |
| OPT-ENG-03 | Dry run with estimated savings, then real run | T2 | Win+Linux | W3-A | done | Exact dry run through the real optimiser into a scratch folder; estimates are always labelled 'estimated' until then. |
| OPT-ENG-04 | Keep original if output is not smaller | T2 | Win+Linux | W3-A | done | Always on: an output that is not strictly smaller is discarded, except equal-size privacy rewrites. |
| OPT-ENG-05 | Preserve timestamps, permissions, alternate data streams | T2 | Win+Linux | W3-A | partial | Timestamps and permissions preserved. Alternate data streams are not: the `preserve_ads` setting is stored but nothing reads it. |
| OPT-ENG-06 | Backup policy: originals to Recycle Bin, backup folder, or delete | T2 | Win+Linux | W3-A | done | Originals to Recycle Bin or quarantine, a backup folder, or delete (delete needs confirmation). The backup folder is guarded like a destination (F-05, fixed). |
| OPT-ENG-07 | Light-copy mode: write `_light` copy, keep original (default for client deliverables) | T2 | Win+Linux | W3-A | done | `_light` copy beside the original, collision-safe; default for Office decks and workbooks. |
| OPT-ENG-08 | Fit a target size (e.g. under 10 MB for email), iterating settings | T2 | Win+Linux | W3-A | partial | `fit_to_target` (preset ladder until under the target) exists and is unit-tested in the framework but has no IPC, UI or CLI surface. |
| OPT-ENG-09 | Per-file size breakdown (images, fonts, media, objects, caches) | T2 | Win+Linux | W3-A | done | Office packages and PDF fill part sizes (images, fonts, caches, other) shown as a size-parts bar; the other optimisers do not. |
| OPT-ENG-10 | Before / after compare view (image diff, video snippet) | T2 | Win+Linux | W3-D | partial | Compare is enabled only for audio waveforms (silences cut). No image diff and no video snippet; disabled with the reason otherwise. |
| OPT-ENG-11 | Job queue: pause, resume, reorder, CPU priority, parallel workers | T2 | Win+Linux | W3-E | partial | Pause, resume, cancel and reorder of queued jobs work. Jobs that change files run one at a time on purpose: the kept-copy rule and the journal rely on the order (W5-FIX D-F9). `parallel_jobs` and CPU priority are shown disabled with that reason. |

## Safety, trust and compliance (`SAF`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| SAF-01 | Protected system paths, not removable | T1 | Win+Linux | W2-E | done | Built-in and user protected paths, MyJanitor's own folders, symlink, junction and mount-point refusal. A folder action walks the whole folder and refuses when anything protected lies below it, checking each entry in its given and its canonical spelling (F-01 and G-01, fixed). The path rules and the junction refusals run on windows-latest [WC]. |
| SAF-02 | Preview before action (thumbnails, image diff, video snippet) | T2 | Win+Linux | W3-D | partial | Preview pane: small web images, first 4 KiB of text, audio waveform. No image diff and no video snippet. |
| SAF-03 | Undo log / restore session for every operation | T1 | Win+Linux | W2-E | done | Crash-safe JSON Lines journal (intent before action), per-entry restore and whole-run undo, recovery of dangling intents; byte-exact undo for optimisations tested. Undo refuses when the content changed since (F-06), and an unreadable journal blocks every acting command (F-08). |
| SAF-04 | Operation report: files touched, bytes saved, errors | T1 | Win+Linux | W2-E | done | Report per run: files, bytes, errors, skips with reasons; text, CSV and HTML. |
| SAF-05 | Admin elevation only when needed (system folders, WOF API) | T1 | Win+Linux | W2-E | done | Nothing elevates itself; permission-denied paths are reported as `needs_elevation`. The NTFS fast path is used only if the process already has rights ([W]). |
| SAF-06 | Fully offline, no telemetry, no uploads | T1 | Win+Linux | W3-E | done | No MyJanitor code opens a network connection; the webview policy allows no external origin. Tauri's dependency tree contains an HTTP client that the app does not call (see README). |
| SAF-07 | Detect sensitivity-labelled encrypted (RMS) and password-protected files: skip and report | T1 | Win+Linux | W2-E | done | Encrypted OOXML (CFB), encrypted ZIP entries, PDF `/Encrypt` and sensitivity labels are skipped, badged and reported. |
| SAF-08 | Preserve sensitivity labels (custom properties) when repacking; block conversion to 97-2003 formats | T1 | Win+Linux | W3-A | done | Label and custom-property parts are protected in the package model; legacy 97-2003 files are never written. |
| SAF-09 | Detect digitally signed PDF/Office files: skip by default | T1 | Win+Linux | W2-E | done | Signed OOXML, ODF and PDF files are skipped by default (setting `skip_signed`). |
| SAF-10 | Synced-folder warning; suggest Free up space | T1 | Win+Linux | W2-E | done | Sync roots detected (OneDrive, Dropbox, Nextcloud, Linux onedrive client, marker folders); warning in the plan checklist; online-only alternative offered. Windows registry detection is [W]. |
| SAF-11 | Lossy and irreversible actions require explicit confirmation | T1 | Win+Linux | W2-E | done | Lossy and permanent steps are refused inside the action engine without a confirmation, whatever the caller does. A confirmation is bound to the selection it was shown (a digest) and is asked again after a rescan or any change to the rows (G-04). |
| SAF-12 | Audit export: CSV log of every change (file, action, size before/after, hash) | T1 | Win+Linux | W2-E | done | Audit CSV per run: file, action, size before and after, SHA-256. |

## Automation and performance (`AUT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| AUT-01 | Command-line mode for scripting | T2 | Win+Linux | W3-E | done | `myjanitor-cli`: scan, list, show, search, plan, dry-run, apply, clean, history, undo, report, run-profile, schedule, context-menu, tools, settings, quarantine, cache, info, commands; exit codes 0 to 3. |
| AUT-02 | Scheduled profiles (e.g. weekly Downloads cleanup + archive cold folders) | T2 | Win+Linux | W3-E | partial | Profiles (reversible steps only: hard links, lossy and permanent steps are refused at save and at run time), next-run maths, `run-profile`, GUI list and run-now work. OS registration is generated and tested with a fake runner only: systemd and Task Scheduler (`schtasks`, [W]) were never run for real. It is done from the CLI (`schedule install`). |
| AUT-03 | Windows Explorer context menu ("Optimise with...", "Find duplicates here") | T2 | Win+Linux | W3-E | partial | Explicit CLI install. Windows `reg.exe` commands are [W]; Linux `.desktop` actions and Nautilus scripts are written and tested but not run in a file manager. The GUI accepts `--optimise` and `--duplicates`. |
| AUT-04 | Portable mode (no installation) | T2 | Win+Linux | W3-E | done | A `portable` marker file beside the executable puts everything under `<exe dir>/data`; no registry or context-menu writes in that mode. |
| AUT-05 | Multi-threaded hashing and encoding, thread count setting | T2 | Win+Linux | W3-E | done | The `threads` setting (0 = all CPUs) drives the scan walk and hash pool and the detector pools. |
| AUT-06 | Low-memory mode for very large drives | T2 | Win+Linux | W3-E | disabled-with-reason | The low-memory toggle is shown off and disabled: 'Not available in this version: scans always keep their file list in memory.' Nothing in the engine reads the setting. |
| AUT-07 | Temp folder location (fast SSD or RAM disk) | T2 | Win+Linux | W3-E | done | The `temp_dir` setting is used for every apply and dry-run scratch folder, and is refused when it lies inside a scanned or protected location; the CLI also exports it as TMPDIR/TEMP for its own process. |

## Summary

| Tier | done | partial | disabled-with-reason | not-targeted | Total |
|---|---|---|---|---|---|
| T1 | 51 | 4 | 0 | 0 | 55 |
| T2 | 32 | 31 | 8 | 0 | 71 |
| T3 | 15 | 3 | 10 | 9 | 37 |
| **Total** | **98** | **38** | **18** | **9** | **163** |

Total features: **163**. Counts are computed from the tables above by a script, and every ID of the spec was matched by regular expression against the tables (163 of 163, none missing, none duplicated).

## T1 gaps

4 of the 55 T1 features are not `done`. Each is below the "must be complete" bar for the reason given. The other 51 T1 rows are `done`; five of them were `partial` before the review-2 fixes and the gap-closure pass (CLN-DET-08, CLN-ACT-04, CLN-ACT-05, CLN-ACT-11 and SAF-01).

| ID | Status | Why |
|---|---|---|
| SCAN-09 Cloud placeholder awareness (OneDrive/Dropbox online-only files excluded from counts) | partial | Windows only, and the OneDrive/Dropbox placeholder handling was never run against a real placeholder: it is tested on hand-built attribute values only (`[W]`). |
| CLN-FLT-06 Owner / user | partial | Linux owner filters work with `uid:N` and user names. The Windows owner (SID) is not collected, because the FFI for it was judged not contained enough to add without a way to exercise it, so owner filters never match on Windows. |
| CLN-DET-12 Office leftovers: AutoRecover / UnsavedFiles folders, Teams and Outlook local caches | partial | Windows only. The path rules are tested on fake folder trees (they run on `windows-latest`) but never against a real Outlook, Teams or Office profile (`[W]`); the card is shown unavailable on Linux. |
| CLN-ACT-02 Delete to Recycle Bin (default) | partial | Linux is complete. On Windows the default route is the MyJanitor quarantine (tested in CI) and the real Recycle Bin route, now reachable through the `windows_recycle_bin` setting, was never exercised against the real shell and restores on a best-effort basis (`[W]`). |
