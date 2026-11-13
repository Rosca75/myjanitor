# MyJanitor Feature Status

Bookkeeping matrix of every feature ID in `docs/spec/file-cleaner-optimiser-features.md`. Updated through the build; final version ships in Wave 4.

## Legend

**Tier** (build-prompt 8.1): `T1` core, must be complete and tested; `T2` optimisation, should be complete; `T3` external-tool and advanced, built if time allows, otherwise visible but disabled with a reason.

**Status:** `planned` (all rows now). Later: `done`, `partial`, `disabled-with-reason`, `not-targeted`.

**Platform:** `Win+Linux`, `Windows only`, `Linux only`, `not targeted` (macOS is never targeted).

**Owner:** workstream ID from build-prompt section 6 (W2-A scan, W2-B duplicates/similarity, W2-C filters/rules/query, W2-D simple detectors, W2-E actions/safety, W2-F frontend shell, W3-A optimisation framework/ZIP/OOXML, W3-B PDF/archiving/FS compression, W3-C images and external-tool media, W3-D IPC wiring and derived screens, W3-E settings/schedules/CLI/context menu/portable/performance).


## Scan and analysis (`SCAN`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| SCAN-01 | Select scan scope: drives, folders, several roots | T1 | Win+Linux | W2-A | planned |  |
| SCAN-02 | Include / exclude paths | T1 | Win+Linux | W2-A | planned |  |
| SCAN-03 | Fast indexing (NTFS MFT read, fallback directory walk) | T1 | Win+Linux | W2-A | planned | MFT fast path Windows only; Linux uses directory walk |
| SCAN-04 | Scan cache for near-instant rescans (metadata + hashes) | T1 | Win+Linux | W2-A | planned |  |
| SCAN-05 | Treemap / sunburst view of disk usage | T2 | Win+Linux | W3-D | planned | Visual-only view |
| SCAN-06 | Folder tree sorted by size | T1 | Win+Linux | W2-F | planned |  |
| SCAN-07 | Breakdown by file type, age, owner | T2 | Win+Linux | W3-D | planned | Chart tabs |
| SCAN-08 | Allocated vs actual size, NTFS compression rate | T1 | Win+Linux | W2-A | planned | NTFS compression rate Windows only |
| SCAN-09 | Cloud placeholder awareness (OneDrive/Dropbox online-only files excluded from counts) | T1 | Windows only | W2-A | planned | Linux: not applicable, marked as such |
| SCAN-10 | Live estimate of reclaimable space per filter and per action | T1 | Win+Linux | W2-E | planned |  |
| SCAN-11 | Scan progress, pause, stop | T1 | Win+Linux | W2-A | planned |  |

## Metadata filters (`CLN-FLT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-FLT-01 | Name / path: wildcard, contains, regex | T1 | Win+Linux | W2-C | planned |  |
| CLN-FLT-02 | Extension and editable file-type groups (Audio, Video, Office, Archives, Installers, System...) | T1 | Win+Linux | W2-C | planned |  |
| CLN-FLT-03 | Size: min/max, top N largest, zero-byte | T1 | Win+Linux | W2-C | planned |  |
| CLN-FLT-04 | Dates: created, modified, last accessed, absolute or relative ("older than 2 years") | T1 | Win+Linux | W2-C | planned |  |
| CLN-FLT-05 | Attributes: hidden, system, read-only, compressed, sparse, encrypted, offline | T1 | Win+Linux | W2-C | planned | Windows attributes; mapped or disabled on Linux |
| CLN-FLT-06 | Owner / user | T1 | Win+Linux | W2-C | planned |  |
| CLN-FLT-07 | Directory depth min/max | T1 | Win+Linux | W2-C | planned |  |
| CLN-FLT-08 | Path length and invalid characters | T1 | Win+Linux | W2-C | planned | Windows invalid-character rules and long paths |

## Content-based filters (`CLN-CNT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-CNT-01 | Full-text content search, including PDF, Word, Excel | T2 | Win+Linux | W2-C | planned | PDF/Word/Excel text extraction in-house |
| CLN-CNT-02 | True file type vs extension mismatch | T1 | Win+Linux | W2-C | planned |  |
| CLN-CNT-03 | Media metadata: EXIF (camera, date taken, GPS), resolution, duration, codec, bitrate, audio tags | T2 | Win+Linux | W2-C | planned | Duration/codec/bitrate may need ffprobe |

## Smart detectors (`CLN-DET`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-DET-01 | Exact duplicates (staged: size, partial hash, full hash) | T1 | Win+Linux | W2-B | planned |  |
| CLN-DET-02 | Similar images (perceptual hash, similarity threshold) | T2 | Win+Linux | W2-B | planned |  |
| CLN-DET-03 | Similar videos | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| CLN-DET-04 | Same music (tags or audio fingerprint) | T3 | Win+Linux | W3-C | planned | Audio fingerprint part needs external tooling; tags mode in-house |
| CLN-DET-05 | Duplicate meeting recordings (same meeting from several sources, audio fingerprint) | T3 | Win+Linux | W3-C | planned | Audio fingerprinting |
| CLN-DET-06 | Empty files and empty folders (including nested empty trees) | T1 | Win+Linux | W2-D | planned |  |
| CLN-DET-07 | Zeroed files (filled with zeros, usually corrupted) | T1 | Win+Linux | W2-D | planned |  |
| CLN-DET-08 | Broken / corrupted files (images, audio, PDF, archives) | T1 | Win+Linux | W2-D | planned |  |
| CLN-DET-09 | Invalid symlinks and broken shortcuts (.lnk) | T1 | Win+Linux | W2-D | planned | .lnk shortcuts Windows only; symlinks both |
| CLN-DET-10 | Temporary and partial files (.tmp, ~$ Office locks, .crdownload, .part, .bak) | T1 | Win+Linux | W2-D | planned |  |
| CLN-DET-11 | App caches and junk, winapp2.ini compatible | T1 | Win+Linux | W2-D | planned | winapp2.ini; Windows-specific entries skipped on Linux |
| CLN-DET-12 | Office leftovers: AutoRecover / UnsavedFiles folders, Teams and Outlook local caches | T1 | Windows only | W2-D | planned | Office/Teams/Outlook caches; no Linux equivalent |
| CLN-DET-13 | Developer artefacts: node_modules, .venv, target/, bin/obj, __pycache__, .gradle, Docker build... | T1 | Win+Linux | W2-D | planned |  |
| CLN-DET-14 | Old installers and disk images (.msi, .exe, .iso in Downloads older than X) | T1 | Win+Linux | W2-D | planned |  |
| CLN-DET-15 | Orphan sidecar files (.xmp, .aae, .thm without parent), RAW+JPEG pairs | T1 | Win+Linux | W2-D | planned |  |
| CLN-DET-16 | Unused language packs / localisations | T1 | Win+Linux | W2-D | planned |  |
| CLN-DET-17 | Stale files (not accessed in N months), feeds archiving | T1 | Win+Linux | W2-D | planned |  |
| CLN-DET-18 | Version chains: keep latest, review others | T2 | Win+Linux | W2-B | planned |  |
| CLN-DET-19 | Derived files (PDF exported from a .pptx/.docx still present in the same folder) | T2 | Win+Linux | W2-B | planned |  |
| CLN-DET-20 | Email attachment copies in Downloads that duplicate synced SharePoint/OneDrive files | T2 | Win+Linux | W2-B | planned |  |
| CLN-DET-21 | Closed engagements / inactive project folders (inactive for N months) | T1 | Win+Linux | W2-D | planned |  |

## Filter engine (`CLN-ENG`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-ENG-01 | Combine filters with AND / OR | T1 | Win+Linux | W2-C | planned |  |
| CLN-ENG-02 | Saved presets / templates ("Downloads older than 90 days", "Dev junk", "Consulting cleanup") | T1 | Win+Linux | W2-C | planned |  |
| CLN-ENG-03 | Global exclusions / whitelist (protected folders, types, system paths) | T1 | Win+Linux | W2-C | planned | Settings UI via W3-E |
| CLN-ENG-04 | Topmost match only (report parent folder instead of every child) | T1 | Win+Linux | W2-C | planned |  |
| CLN-ENG-05 | Quick search syntax (e.g. `ext:mp4 size:>1GB modified:<2024`) | T1 | Win+Linux | W2-C | planned |  |

## Cleaning actions (`CLN-ACT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| CLN-ACT-01 | Smart auto-select for duplicates: keep newest, oldest, shortest path, highest resolution,... | T1 | Win+Linux | W2-E | planned |  |
| CLN-ACT-02 | Delete to Recycle Bin (default) | T1 | Win+Linux | W2-E | planned |  |
| CLN-ACT-03 | Permanent delete with confirmation | T1 | Win+Linux | W2-E | planned |  |
| CLN-ACT-04 | Quarantine folder, restorable, auto-purge after N days | T1 | Win+Linux | W2-E | planned |  |
| CLN-ACT-05 | Move / copy to another location, keeping folder structure | T1 | Win+Linux | W2-E | planned |  |
| CLN-ACT-06 | Replace duplicates with hardlinks (NTFS) or reflinks (ReFS, APFS, Btrfs) | T2 | Win+Linux | W2-E | planned | NTFS/ReFS hardlinks and reflinks; Btrfs/XFS FICLONE on Linux; APFS not targeted |
| CLN-ACT-07 | OneDrive "Free up space": turn local copy into online-only instead of deleting | T2 | Windows only | W2-E | planned | OneDrive dehydrate; not applicable on Linux |
| CLN-ACT-08 | Secure shred files | T3 | Win+Linux | W2-E | planned | Advanced; visible but may be disabled |
| CLN-ACT-09 | Wipe free space | T3 | Win+Linux | W2-E | planned | Advanced; visible but may be disabled |
| CLN-ACT-10 | Vacuum application databases (browser SQLite) | T3 | Win+Linux | W2-E | planned | SQLite vacuum |
| CLN-ACT-11 | Export results (CSV, HTML, PDF) | T1 | Win+Linux | W2-E | planned | CSV/HTML/PDF export in-house |

## Images (`OPT-IMG`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-IMG-01 | Lossless recompression (PNG oxipng, JPEG jpegtran/MozJPEG, GIF gifsicle) | T2 | Win+Linux | W3-C | planned | Lossless; external oxipng/jpegtran/gifsicle only if present |
| OPT-IMG-02 | Lossy recompression with quality slider and side-by-side preview | T3 | Win+Linux | W3-C | planned | Lossy; external encoders when present |
| OPT-IMG-03 | Format conversion: PNG to WebP, JPEG to AVIF, lossless JPEG to JPEG XL (reversible) | T3 | Win+Linux | W3-C | planned | Lossy; external encoders when present |
| OPT-IMG-04 | Resize to max long / short edge | T3 | Win+Linux | W3-C | planned | Lossy; external encoders when present |
| OPT-IMG-05 | Metadata control: strip all, keep all, strip GPS only, thumbnails, ICC profiles | T2 | Win+Linux | W3-C | planned |  |
| OPT-IMG-06 | Overwrite policy: always, never, only if smaller | T2 | Win+Linux | W3-A | planned |  |

## Video (`OPT-VID`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-VID-01 | Re-encode to HEVC or AV1 with quality-based CRF | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-VID-02 | Hardware encoders (NVENC, Quick Sync, AMF, VideoToolbox) with estimated encode time | T3 | Win+Linux | W3-C | planned | Needs ffmpeg; NVENC/QSV/AMF Windows, VAAPI/NVENC Linux; VideoToolbox not targeted |
| OPT-VID-03 | Downscale resolution and cap frame rate | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-VID-04 | Drop extra audio tracks and subtitles | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-VID-05 | Remux only (no re-encode), strip junk streams | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-VID-06 | Compatibility warning for target codec | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |

## Screen recordings (`OPT-SCR`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-SCR-01 | Auto-detect screen content (low motion: slides, apps, code) and apply dedicated preset | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-SCR-02 | Lower frame rate (down to 5 to 15 fps), keep resolution for text legibility | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-SCR-03 | AV1 screen-content coding mode (intra block copy, palette) | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-SCR-04 | Trim dead time (static or silent start/end segments) | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-SCR-05 | Derive lighter outputs: audio only, or key frames to slide PDF, then archive/delete the video | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |

## Audio and meeting recordings (`OPT-AUD`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-AUD-01 | WAV/AIFF to FLAC (lossless) or Opus/AAC (lossy) | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-AUD-02 | Recompress FLAC at maximum level | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-AUD-03 | Strip or downsize embedded cover art | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-AUD-04 | Speech preset: Opus mono about 24 kbps | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-AUD-05 | Downmix to mono for voice-only recordings | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-AUD-06 | Remove long silences (voice activity detection, max pause setting) | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-AUD-07 | Transcription-safe profile (avoid over-compression when transcripts are needed) | T3 | Win+Linux | W3-C | planned | Needs ffmpeg on PATH |
| OPT-AUD-08 | Transcribe locally, then archive/delete audio by retention rule | T3 | Win+Linux | W3-C | planned | Needs local transcription engine |

## PDF (`OPT-PDF`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-PDF-01 | Image downsampling by target DPI | T3 | Win+Linux | W3-B | planned | Lossy image recompression |
| OPT-PDF-02 | Font subsetting and duplicate font removal | T3 | Win+Linux | W3-B | planned | Font subsetting is complex |
| OPT-PDF-03 | Lossless structural optimisation (object streams, stream recompression, unused objects) | T2 | Win+Linux | W3-B | planned |  |
| OPT-PDF-04 | Presets: Screen, eBook, Prepress | T3 | Win+Linux | W3-B | planned | Lossy image recompression |
| OPT-PDF-05 | Remove dead weight: metadata, thumbnails, edit history, attachments, JavaScript | T2 | Win+Linux | W3-B | planned |  |

## PowerPoint (`OPT-PPT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-PPT-01 | Compress / downsample embedded pictures (96 / 150 / 220 ppi) | T2 | Win+Linux | W3-A | planned |  |
| OPT-PPT-02 | Delete cropped areas of pictures (irreversible, opt-in) | T2 | Win+Linux | W3-A | planned |  |
| OPT-PPT-03 | Delete picture editing history | T2 | Win+Linux | W3-A | planned |  |
| OPT-PPT-04 | Remove unused slide masters and layouts | T2 | Win+Linux | W3-A | planned |  |
| OPT-PPT-05 | Subset or unembed embedded fonts | T2 | Win+Linux | W3-A | planned |  |
| OPT-PPT-06 | Embedded Excel chart data: remove, convert chart to static picture, or keep | T2 | Win+Linux | W3-A | planned |  |
| OPT-PPT-07 | Convert large EMF/WMF pictures to PNG/JPEG at chosen DPI | T2 | Win+Linux | W3-A | planned |  |
| OPT-PPT-08 | Deduplicate identical media inside the package | T2 | Win+Linux | W3-A | planned |  |
| OPT-PPT-09 | Re-encode embedded video/audio, or extract and link | T3 | Win+Linux | W3-A | planned | Needs ffmpeg on PATH |
| OPT-PPT-10 | Optional removals: hidden slides, speaker notes, comments, custom XML (off by default) | T2 | Win+Linux | W3-A | planned |  |

## Excel (`OPT-XLS`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-XLS-01 | Bloat diagnosis: weight by used range, pivot caches, styles, images, objects | T2 | Win+Linux | W3-A | planned |  |
| OPT-XLS-02 | Reset inflated used range on every sheet | T2 | Win+Linux | W3-A | planned |  |
| OPT-XLS-03 | Pivot caches: stop saving source data, merge identical caches | T2 | Win+Linux | W3-A | planned |  |
| OPT-XLS-04 | Remove unused cell styles (imported from templates) | T2 | Win+Linux | W3-A | planned |  |
| OPT-XLS-05 | Narrow whole-column conditional formatting, remove duplicate rules | T2 | Win+Linux | W3-A | planned |  |
| OPT-XLS-06 | Remove broken / unused named ranges, stale external links, unused query connections | T2 | Win+Linux | W3-A | planned |  |
| OPT-XLS-07 | Report hidden sheets (review only, no auto-delete) | T2 | Win+Linux | W3-A | planned |  |
| OPT-XLS-08 | Compress embedded pictures | T2 | Win+Linux | W3-A | planned |  |
| OPT-XLS-09 | Optional conversion to .xlsb, with compatibility warning | T3 | Win+Linux | W3-A | planned | Binary .xlsb writer; compatibility warning |
| OPT-XLS-10 | Macro warning: used-range reset may break VBA relying on UsedRange | T2 | Win+Linux | W3-A | planned |  |

## Word (`OPT-DOC`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-DOC-01 | Compress pictures (cropped areas, editing history, DPI) | T2 | Win+Linux | W3-A | planned |  |
| OPT-DOC-02 | Convert embedded objects (Excel tables, Visio, EMF) to pictures or keep | T2 | Win+Linux | W3-A | planned |  |
| OPT-DOC-03 | Subset or remove embedded fonts | T2 | Win+Linux | W3-A | planned |  |
| OPT-DOC-04 | Report tracked changes and comments weight (never auto-accept) | T2 | Win+Linux | W3-A | planned |  |
| OPT-DOC-05 | Remove large attached templates and custom XML parts | T2 | Win+Linux | W3-A | planned |  |

## Office packages (`OPT-OFC`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-OFC-01 | Repack ZIP container at maximum Deflate level | T2 | Win+Linux | W3-A | planned |  |
| OPT-OFC-02 | Remove embedded thumbnails and revision data | T2 | Win+Linux | W3-A | planned |  |
| OPT-OFC-03 | Lossless recompression of other ZIP-based formats (EPUB, CBZ, APK/JAR, ODT) | T2 | Win+Linux | W3-A | planned |  |

## System fonts (`OPT-FNT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-FNT-01 | Detect duplicate installed fonts (user and system folders) | T2 | Win+Linux | W2-B | planned |  |
| OPT-FNT-02 | Detect unused font families and font caches (review only) | T2 | Win+Linux | W2-D | planned |  |

## Archiving (`OPT-ARC`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-ARC-01 | Archive cold folders (not accessed in N months) to 7z / ZIP / zstd | T2 | Win+Linux | W3-B | planned | ZIP in-house; 7z/zstd (T3) need external tools |
| OPT-ARC-02 | Integrity test before deleting source | T2 | Win+Linux | W3-B | planned |  |
| OPT-ARC-03 | Recompress existing archives with stronger algorithm or solid mode | T2 | Win+Linux | W3-B | planned | ZIP in-house; 7z/zstd (T3) need external tools |
| OPT-ARC-04 | Archive closed engagements as one file with searchable index | T2 | Win+Linux | W3-B | planned |  |
| OPT-ARC-05 | Optional AES-256 encryption, split volumes, compression level | T3 | Win+Linux | W3-B | planned | AES-256/split volumes via external 7z |

## File-system compression (`OPT-FSC`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-FSC-01 | NTFS WOF compression per folder: XPRESS4K, XPRESS8K, XPRESS16K, LZX | T2 | Windows only | W3-B | planned | NTFS WOF; Linux Btrfs covered by OPT-FSC-06 |
| OPT-FSC-02 | Skip poorly compressible files | T2 | Windows only | W3-B | planned | NTFS WOF; Linux Btrfs covered by OPT-FSC-06 |
| OPT-FSC-03 | Background watcher to recompress after updates | T2 | Windows only | W3-E | planned | Background watcher |
| OPT-FSC-04 | DirectStorage warning on game folders | T2 | Windows only | W3-B | planned | NTFS WOF; Linux Btrfs covered by OPT-FSC-06 |
| OPT-FSC-05 | Decompress / revert | T2 | Windows only | W3-B | planned | NTFS WOF; Linux Btrfs covered by OPT-FSC-06 |
| OPT-FSC-06 | macOS APFS compression, Btrfs zstd equivalents | T2 | Linux only | W3-B | planned | Btrfs zstd where supported; macOS APFS part not targeted |

## Other formats (`OPT-OTH`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-OTH-01 | Mailbox compaction (OST/PST) | T3 | Windows only | W3-B | planned | OST/PST compaction |
| OPT-OTH-02 | Virtual disk compaction (VHDX/VMDK) | T3 | Windows only | W3-B | planned | VHDX/VMDK; needs external tooling |
| OPT-OTH-03 | Executable packing (UPX), advanced only, antivirus false-positive warning | T3 | Win+Linux | W3-C | planned | Needs upx on PATH; antivirus warning |

## Optimisation engine (`OPT-ENG`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| OPT-ENG-01 | Lossless / lossy switch, global and per file type | T2 | Win+Linux | W3-A | planned |  |
| OPT-ENG-02 | Content-based file type detection | T1 | Win+Linux | W3-A | planned | Shared with CLN-CNT-02 |
| OPT-ENG-03 | Dry run with estimated savings, then real run | T2 | Win+Linux | W3-A | planned |  |
| OPT-ENG-04 | Keep original if output is not smaller | T2 | Win+Linux | W3-A | planned |  |
| OPT-ENG-05 | Preserve timestamps, permissions, alternate data streams | T2 | Win+Linux | W3-A | planned | Alternate data streams Windows only |
| OPT-ENG-06 | Backup policy: originals to Recycle Bin, backup folder, or delete | T2 | Win+Linux | W3-A | planned |  |
| OPT-ENG-07 | Light-copy mode: write `_light` copy, keep original (default for client deliverables) | T2 | Win+Linux | W3-A | planned |  |
| OPT-ENG-08 | Fit a target size (e.g. under 10 MB for email), iterating settings | T2 | Win+Linux | W3-A | planned |  |
| OPT-ENG-09 | Per-file size breakdown (images, fonts, media, objects, caches) | T2 | Win+Linux | W3-A | planned |  |
| OPT-ENG-10 | Before / after compare view (image diff, video snippet) | T2 | Win+Linux | W3-D | planned | Visual-only compare view |
| OPT-ENG-11 | Job queue: pause, resume, reorder, CPU priority, parallel workers | T2 | Win+Linux | W3-E | planned | Queue UI and CPU priority |

## Safety, trust and compliance (`SAF`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| SAF-01 | Protected system paths, not removable | T1 | Win+Linux | W2-E | planned |  |
| SAF-02 | Preview before action (thumbnails, image diff, video snippet) | T2 | Win+Linux | W3-D | planned | Preview pane; video snippet needs ffmpeg |
| SAF-03 | Undo log / restore session for every operation | T1 | Win+Linux | W2-E | planned |  |
| SAF-04 | Operation report: files touched, bytes saved, errors | T1 | Win+Linux | W2-E | planned |  |
| SAF-05 | Admin elevation only when needed (system folders, WOF API) | T1 | Win+Linux | W2-E | planned | Linux never elevates; reports permission-denied paths |
| SAF-06 | Fully offline, no telemetry, no uploads | T1 | Win+Linux | W3-E | planned |  |
| SAF-07 | Detect sensitivity-labelled encrypted (RMS) and password-protected files: skip and report | T1 | Win+Linux | W2-E | planned |  |
| SAF-08 | Preserve sensitivity labels (custom properties) when repacking; block conversion to 97-2003 formats | T1 | Win+Linux | W3-A | planned | Applied during OOXML repack |
| SAF-09 | Detect digitally signed PDF/Office files: skip by default | T1 | Win+Linux | W2-E | planned |  |
| SAF-10 | Synced-folder warning; suggest Free up space | T1 | Win+Linux | W2-E | planned |  |
| SAF-11 | Lossy and irreversible actions require explicit confirmation | T1 | Win+Linux | W2-E | planned |  |
| SAF-12 | Audit export: CSV log of every change (file, action, size before/after, hash) | T1 | Win+Linux | W2-E | planned |  |

## Automation and performance (`AUT`)

| ID | Feature | Tier | Platform | Owner | Status | Note |
|---|---|---|---|---|---|---|
| AUT-01 | Command-line mode for scripting | T2 | Win+Linux | W3-E | planned | myjanitor-cli |
| AUT-02 | Scheduled profiles (e.g. weekly Downloads cleanup + archive cold folders) | T2 | Win+Linux | W3-E | planned |  |
| AUT-03 | Windows Explorer context menu ("Optimise with...", "Find duplicates here") | T2 | Win+Linux | W3-E | planned | Windows HKCU registry; Linux .desktop actions |
| AUT-04 | Portable mode (no installation) | T2 | Win+Linux | W3-E | planned |  |
| AUT-05 | Multi-threaded hashing and encoding, thread count setting | T2 | Win+Linux | W3-E | planned |  |
| AUT-06 | Low-memory mode for very large drives | T2 | Win+Linux | W3-E | planned |  |
| AUT-07 | Temp folder location (fast SSD or RAM disk) | T2 | Win+Linux | W3-E | planned |  |

## Summary

| Tier | planned | done | partial | disabled-with-reason | not-targeted | Total |
|---|---|---|---|---|---|---|
| T1 | 55 | 0 | 0 | 0 | 0 | 55 |
| T2 | 71 | 0 | 0 | 0 | 0 | 71 |
| T3 | 37 | 0 | 0 | 0 | 0 | 37 |
| **Total** | 163 | 0 | 0 | 0 | 0 | 163 |

Total features: **163**.
