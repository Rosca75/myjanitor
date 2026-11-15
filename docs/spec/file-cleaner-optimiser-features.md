# File Cleaner & Optimiser: Feature Specification for Wireframe Design

---

## 1. Product context

| Item | Description |
|---|---|
| **Product** | Stand-alone desktop application, running locally on the user's computer |
| **Goal 1: Cleaning** | Find the right set of files to remove, using a wide range of filters and smart detectors |
| **Goal 2: Optimisation** | Reduce file size with compression techniques (images, video, audio, PDF, Office, fonts, archives, file-system compression) |
| **Primary users** | Knowledge workers and consultants (PowerPoint, Excel, Word, screen recordings, meeting audio), power users, photographers, developers |
| **Platform** | Windows first (NTFS, WOF API, OneDrive), with macOS/Linux equivalents where possible |
| **Constraints** | Fully offline, no telemetry, enterprise-safe (DLP, sensitivity labels, synced folders), no admin rights unless required |
| **Key principle** | Preview and estimate before acting. Reversible by default. Lossy actions are opt-in |

---

## 2. Single-screen layout (UI zones)

| Code | Zone | Content |
|---|---|---|
| `TOP` | Top bar | Scan scope picker, Scan / Stop button, mode switch (Clean / Optimise / Both), search box with query syntax, settings icon |
| `LEFT` | Left panel | Category tree: cleaning detectors, filters, optimisation targets by file type, saved presets, exclusions |
| `CENTER` | Main area | Results grid (default) switchable to treemap / sunburst / size-breakdown charts. Row checkboxes, grouping, sorting, estimated saving per row |
| `RIGHT` | Inspector panel | Context-sensitive: file preview, per-file size breakdown, action buttons, settings of the selected action (quality sliders, presets) |
| `BOTTOM` | Status strip | Total reclaimable vs reclaimed, selection summary, job queue with progress, pause/resume, undo, last-operation report link |
| `MODAL` | Dialogs | Confirmations (permanent delete, lossy actions), compare view (before/after), restore session, report export |
| `SETTINGS` | Settings drawer | Global preferences, protected paths, engines, performance, automation |

**Main screen states to design**: empty state (no scan yet), scanning (progress), results loaded, action running (job queue active), action completed (summary + undo).

---

## 3. Scan and analysis (`SCAN`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| SCAN-01 | Select scan scope: drives, folders, several roots | TOP | Multi-select path picker |
| SCAN-02 | Include / exclude paths | LEFT | Path list with add/remove |
| SCAN-03 | Fast indexing (NTFS MFT read, fallback directory walk) | SETTINGS | Toggle |
| SCAN-04 | Scan cache for near-instant rescans (metadata + hashes) | SETTINGS | Toggle + "Clear cache" button |
| SCAN-05 | Treemap / sunburst view of disk usage | CENTER | View switcher |
| SCAN-06 | Folder tree sorted by size | CENTER | Tree grid |
| SCAN-07 | Breakdown by file type, age, owner | CENTER | Chart tabs |
| SCAN-08 | Allocated vs actual size, NTFS compression rate | CENTER | Grid columns |
| SCAN-09 | Cloud placeholder awareness (OneDrive/Dropbox online-only files excluded from counts) | CENTER | Badge on rows |
| SCAN-10 | Live estimate of reclaimable space per filter and per action | BOTTOM / LEFT | Counter + per-item badge |
| SCAN-11 | Scan progress, pause, stop | TOP / BOTTOM | Progress bar + buttons |

---

## 4. Cleaning (`CLN`)

### 4.1 Metadata filters (`CLN-FLT`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| CLN-FLT-01 | Name / path: wildcard, contains, regex | LEFT | Text field + mode dropdown |
| CLN-FLT-02 | Extension and editable file-type groups (Audio, Video, Office, Archives, Installers, System...) | LEFT | Checkbox group |
| CLN-FLT-03 | Size: min/max, top N largest, zero-byte | LEFT | Range slider + number input |
| CLN-FLT-04 | Dates: created, modified, last accessed, absolute or relative ("older than 2 years") | LEFT | Date range + relative picker |
| CLN-FLT-05 | Attributes: hidden, system, read-only, compressed, sparse, encrypted, offline | LEFT | Tri-state checkboxes |
| CLN-FLT-06 | Owner / user | LEFT | Dropdown |
| CLN-FLT-07 | Directory depth min/max | LEFT | Number inputs |
| CLN-FLT-08 | Path length and invalid characters | LEFT | Toggle + number |

### 4.2 Content-based filters (`CLN-CNT`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| CLN-CNT-01 | Full-text content search, including PDF, Word, Excel | LEFT | Text field |
| CLN-CNT-02 | True file type vs extension mismatch | LEFT | Toggle |
| CLN-CNT-03 | Media metadata: EXIF (camera, date taken, GPS), resolution, duration, codec, bitrate, audio tags | LEFT | Field picker + condition |

### 4.3 Smart detectors (`CLN-DET`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| CLN-DET-01 | Exact duplicates (staged: size, partial hash, full hash) | LEFT | Detector item + settings popover |
| CLN-DET-02 | Similar images (perceptual hash, similarity threshold) | LEFT | Detector + threshold slider |
| CLN-DET-03 | Similar videos | LEFT | Detector + threshold slider |
| CLN-DET-04 | Same music (tags or audio fingerprint) | LEFT | Detector + mode dropdown |
| CLN-DET-05 | Duplicate meeting recordings (same meeting from several sources, audio fingerprint) | LEFT | Detector |
| CLN-DET-06 | Empty files and empty folders (including nested empty trees) | LEFT | Detector |
| CLN-DET-07 | Zeroed files (filled with zeros, usually corrupted) | LEFT | Detector |
| CLN-DET-08 | Broken / corrupted files (images, audio, PDF, archives) | LEFT | Detector |
| CLN-DET-09 | Invalid symlinks and broken shortcuts (.lnk) | LEFT | Detector |
| CLN-DET-10 | Temporary and partial files (.tmp, ~$ Office locks, .crdownload, .part, .bak) | LEFT | Detector |
| CLN-DET-11 | App caches and junk (browser caches, thumbnail caches, crash dumps, logs, Windows Update leftovers), winapp2.ini compatible | LEFT | Detector with sub-list of apps |
| CLN-DET-12 | Office leftovers: AutoRecover / UnsavedFiles folders, Teams and Outlook local caches | LEFT | Detector |
| CLN-DET-13 | Developer artefacts: node_modules, .venv, target/, bin/obj, __pycache__, .gradle, Docker build caches | LEFT | Detector |
| CLN-DET-14 | Old installers and disk images (.msi, .exe, .iso in Downloads older than X) | LEFT | Detector + age input |
| CLN-DET-15 | Orphan sidecar files (.xmp, .aae, .thm without parent), RAW+JPEG pairs | LEFT | Detector |
| CLN-DET-16 | Unused language packs / localisations | LEFT | Detector |
| CLN-DET-17 | Stale files (not accessed in N months), feeds archiving | LEFT | Detector + months input |
| CLN-DET-18 | Version chains (v1...vN, "final", "final_v2", date prefixes, initials suffixes): keep latest, review others | LEFT | Detector + keep-rule dropdown |
| CLN-DET-19 | Derived files (PDF exported from a .pptx/.docx still present in the same folder) | LEFT | Detector |
| CLN-DET-20 | Email attachment copies in Downloads that duplicate synced SharePoint/OneDrive files | LEFT | Detector |
| CLN-DET-21 | Closed engagements / inactive project folders (inactive for N months) | LEFT | Detector + months input |

### 4.4 Filter engine (`CLN-ENG`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| CLN-ENG-01 | Combine filters with AND / OR | LEFT | Rule builder |
| CLN-ENG-02 | Saved presets / templates ("Downloads older than 90 days", "Dev junk", "Consulting cleanup") | LEFT | Preset list + save button |
| CLN-ENG-03 | Global exclusions / whitelist (protected folders, types, system paths) | SETTINGS | Path and pattern list |
| CLN-ENG-04 | Topmost match only (report parent folder instead of every child) | LEFT | Toggle |
| CLN-ENG-05 | Quick search syntax (e.g. `ext:mp4 size:>1GB modified:<2024`) | TOP | Search box with autocomplete |

### 4.5 Cleaning actions (`CLN-ACT`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| CLN-ACT-01 | Smart auto-select for duplicates: keep newest, oldest, shortest path, highest resolution, preferred folder | RIGHT | Dropdown + "Apply selection" |
| CLN-ACT-02 | Delete to Recycle Bin (default) | RIGHT | Primary button |
| CLN-ACT-03 | Permanent delete with confirmation | RIGHT / MODAL | Secondary button + confirm dialog |
| CLN-ACT-04 | Quarantine folder, restorable, auto-purge after N days | RIGHT | Button + setting |
| CLN-ACT-05 | Move / copy to another location, keeping folder structure | RIGHT | Button + path picker |
| CLN-ACT-06 | Replace duplicates with hardlinks (NTFS) or reflinks (ReFS, APFS, Btrfs) | RIGHT | Button |
| CLN-ACT-07 | OneDrive "Free up space": turn local copy into online-only instead of deleting | RIGHT | Button |
| CLN-ACT-08 | Secure shred files | RIGHT | Button (advanced) |
| CLN-ACT-09 | Wipe free space | SETTINGS / MODAL | Button (advanced) |
| CLN-ACT-10 | Vacuum application databases (browser SQLite) | RIGHT | Button |
| CLN-ACT-11 | Export results (CSV, HTML, PDF) | CENTER / MODAL | Export menu |

---

## 5. Optimisation (`OPT`)

### 5.1 Images (`OPT-IMG`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-IMG-01 | Lossless recompression (PNG oxipng, JPEG jpegtran/MozJPEG, GIF gifsicle) | RIGHT | Toggle "Lossless" |
| OPT-IMG-02 | Lossy recompression with quality slider and side-by-side preview | RIGHT / MODAL | Slider + compare view |
| OPT-IMG-03 | Format conversion: PNG to WebP, JPEG to AVIF, lossless JPEG to JPEG XL (reversible) | RIGHT | Target format dropdown |
| OPT-IMG-04 | Resize to max long / short edge | RIGHT | Number input + edge dropdown |
| OPT-IMG-05 | Metadata control: strip all, keep all, strip GPS only, thumbnails, ICC profiles | RIGHT | Radio group |
| OPT-IMG-06 | Overwrite policy: always, never, only if smaller | RIGHT | Radio group |

### 5.2 Video (`OPT-VID`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-VID-01 | Re-encode to HEVC or AV1 with quality-based CRF | RIGHT | Codec dropdown + quality slider |
| OPT-VID-02 | Hardware encoders (NVENC, Quick Sync, AMF, VideoToolbox) with estimated encode time | RIGHT | Dropdown + time estimate |
| OPT-VID-03 | Downscale resolution and cap frame rate | RIGHT | Dropdowns |
| OPT-VID-04 | Drop extra audio tracks and subtitles | RIGHT | Track checklist |
| OPT-VID-05 | Remux only (no re-encode), strip junk streams | RIGHT | Button |
| OPT-VID-06 | Compatibility warning for target codec | RIGHT | Inline warning |

### 5.3 Screen recordings (`OPT-SCR`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-SCR-01 | Auto-detect screen content (low motion: slides, apps, code) and apply dedicated preset | CENTER / RIGHT | Badge + preset |
| OPT-SCR-02 | Lower frame rate (down to 5 to 15 fps), keep resolution for text legibility | RIGHT | Frame-rate dropdown |
| OPT-SCR-03 | AV1 screen-content coding mode (intra block copy, palette) | RIGHT | Toggle (advanced) |
| OPT-SCR-04 | Trim dead time (static or silent start/end segments) | RIGHT / MODAL | Timeline with suggested cuts |
| OPT-SCR-05 | Derive lighter outputs: audio only, or key frames to slide PDF, then archive/delete the video | RIGHT | Output-type dropdown |

### 5.4 Audio and meeting recordings (`OPT-AUD`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-AUD-01 | WAV/AIFF to FLAC (lossless) or Opus/AAC (lossy) | RIGHT | Format dropdown |
| OPT-AUD-02 | Recompress FLAC at maximum level | RIGHT | Toggle |
| OPT-AUD-03 | Strip or downsize embedded cover art | RIGHT | Toggle |
| OPT-AUD-04 | Speech preset: Opus mono about 24 kbps | RIGHT | Preset button |
| OPT-AUD-05 | Downmix to mono for voice-only recordings | RIGHT | Toggle |
| OPT-AUD-06 | Remove long silences (voice activity detection, max pause setting) | RIGHT | Toggle + seconds input |
| OPT-AUD-07 | Transcription-safe profile (avoid over-compression when transcripts are needed) | RIGHT | Preset button |
| OPT-AUD-08 | Transcribe locally, then archive/delete audio by retention rule | RIGHT | Toggle + rule picker |

### 5.5 PDF (`OPT-PDF`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-PDF-01 | Image downsampling by target DPI | RIGHT | DPI dropdown |
| OPT-PDF-02 | Font subsetting and duplicate font removal | RIGHT | Toggle |
| OPT-PDF-03 | Lossless structural optimisation (object streams, stream recompression, unused objects) | RIGHT | Toggle "Lossless" |
| OPT-PDF-04 | Presets: Screen, eBook, Prepress | RIGHT | Segmented control |
| OPT-PDF-05 | Remove dead weight: metadata, thumbnails, edit history, attachments, JavaScript | RIGHT | Checklist |

### 5.6 PowerPoint (`OPT-PPT`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-PPT-01 | Compress / downsample embedded pictures (96 / 150 / 220 ppi) | RIGHT | DPI dropdown |
| OPT-PPT-02 | Delete cropped areas of pictures (irreversible, opt-in) | RIGHT | Toggle + warning |
| OPT-PPT-03 | Delete picture editing history | RIGHT | Toggle |
| OPT-PPT-04 | Remove unused slide masters and layouts | RIGHT | Checklist of detected masters |
| OPT-PPT-05 | Subset or unembed embedded fonts | RIGHT | Radio group |
| OPT-PPT-06 | Embedded Excel chart data: remove, convert chart to static picture, or keep | RIGHT | Radio group |
| OPT-PPT-07 | Convert large EMF/WMF pictures to PNG/JPEG at chosen DPI | RIGHT | Toggle + DPI |
| OPT-PPT-08 | Deduplicate identical media inside the package | RIGHT | Toggle |
| OPT-PPT-09 | Re-encode embedded video/audio, or extract and link | RIGHT | Radio group |
| OPT-PPT-10 | Optional removals: hidden slides, speaker notes, comments, custom XML (off by default) | RIGHT | Checklist |

### 5.7 Excel (`OPT-XLS`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-XLS-01 | Bloat diagnosis: weight by used range, pivot caches, styles, images, objects | RIGHT | Breakdown bar |
| OPT-XLS-02 | Reset inflated used range on every sheet | RIGHT | Toggle |
| OPT-XLS-03 | Pivot caches: stop saving source data, merge identical caches | RIGHT | Toggles |
| OPT-XLS-04 | Remove unused cell styles (imported from templates) | RIGHT | Toggle |
| OPT-XLS-05 | Narrow whole-column conditional formatting, remove duplicate rules | RIGHT | Toggle |
| OPT-XLS-06 | Remove broken / unused named ranges, stale external links, unused query connections | RIGHT | Checklist |
| OPT-XLS-07 | Report hidden sheets (review only, no auto-delete) | RIGHT | Info list |
| OPT-XLS-08 | Compress embedded pictures | RIGHT | DPI dropdown |
| OPT-XLS-09 | Optional conversion to .xlsb, with compatibility warning | RIGHT | Toggle + warning |
| OPT-XLS-10 | Macro warning: used-range reset may break VBA relying on UsedRange | RIGHT | Inline warning |

### 5.8 Word (`OPT-DOC`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-DOC-01 | Compress pictures (cropped areas, editing history, DPI) | RIGHT | Same controls as OPT-PPT-01/02/03 |
| OPT-DOC-02 | Convert embedded objects (Excel tables, Visio, EMF) to pictures or keep | RIGHT | Radio group |
| OPT-DOC-03 | Subset or remove embedded fonts | RIGHT | Radio group |
| OPT-DOC-04 | Report tracked changes and comments weight (never auto-accept) | RIGHT | Info list |
| OPT-DOC-05 | Remove large attached templates and custom XML parts | RIGHT | Checklist |

### 5.9 Office packages, generic (`OPT-OFC`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-OFC-01 | Repack ZIP container at maximum Deflate level | RIGHT | Toggle |
| OPT-OFC-02 | Remove embedded thumbnails and revision data | RIGHT | Toggle |
| OPT-OFC-03 | Lossless recompression of other ZIP-based formats (EPUB, CBZ, APK/JAR, ODT) | RIGHT | Toggle |

### 5.10 System fonts (`OPT-FNT`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-FNT-01 | Detect duplicate installed fonts (user and system folders) | LEFT / CENTER | Detector |
| OPT-FNT-02 | Detect unused font families and font caches (review only) | LEFT / CENTER | Detector |

### 5.11 Archiving (`OPT-ARC`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-ARC-01 | Archive cold folders (not accessed in N months) to 7z / ZIP / zstd | RIGHT | Format dropdown + months input |
| OPT-ARC-02 | Integrity test before deleting source | RIGHT | Toggle (on by default, locked) |
| OPT-ARC-03 | Recompress existing archives with stronger algorithm or solid mode | RIGHT | Toggle |
| OPT-ARC-04 | Archive closed engagements as one file with searchable index | RIGHT | Button |
| OPT-ARC-05 | Optional AES-256 encryption, split volumes, compression level | RIGHT | Toggles + slider |

### 5.12 Transparent file-system compression (`OPT-FSC`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-FSC-01 | NTFS WOF compression per folder: XPRESS4K, XPRESS8K, XPRESS16K, LZX | RIGHT | Algorithm dropdown |
| OPT-FSC-02 | Skip poorly compressible files | RIGHT | Toggle |
| OPT-FSC-03 | Background watcher to recompress after updates | SETTINGS | Toggle + folder list |
| OPT-FSC-04 | DirectStorage warning on game folders | RIGHT | Inline warning |
| OPT-FSC-05 | Decompress / revert | RIGHT | Button |
| OPT-FSC-06 | macOS APFS compression, Btrfs zstd equivalents | RIGHT | Platform-specific dropdown |

### 5.13 Other formats (`OPT-OTH`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-OTH-01 | Mailbox compaction (OST/PST) | RIGHT | Button |
| OPT-OTH-02 | Virtual disk compaction (VHDX/VMDK) | RIGHT | Button |
| OPT-OTH-03 | Executable packing (UPX), advanced only, antivirus false-positive warning | RIGHT | Toggle (advanced) + warning |

### 5.14 Optimisation engine (`OPT-ENG`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| OPT-ENG-01 | Lossless / lossy switch, global and per file type | TOP / RIGHT | Segmented control |
| OPT-ENG-02 | Content-based file type detection | (engine) | None |
| OPT-ENG-03 | Dry run with estimated savings, then real run | RIGHT / BOTTOM | "Estimate" + "Apply" buttons |
| OPT-ENG-04 | Keep original if output is not smaller | RIGHT | Toggle (on by default) |
| OPT-ENG-05 | Preserve timestamps, permissions, alternate data streams | SETTINGS | Toggle |
| OPT-ENG-06 | Backup policy: originals to Recycle Bin, backup folder, or delete | SETTINGS / RIGHT | Radio group |
| OPT-ENG-07 | Light-copy mode: write `_light` copy, keep original (default for client deliverables) | RIGHT | Toggle |
| OPT-ENG-08 | Fit a target size (e.g. under 10 MB for email), iterating settings | RIGHT | Size input + "Fit" button |
| OPT-ENG-09 | Per-file size breakdown (images, fonts, media, objects, caches) | RIGHT | Stacked bar |
| OPT-ENG-10 | Before / after compare view (image diff, video snippet) | MODAL | Split view |
| OPT-ENG-11 | Job queue: pause, resume, reorder, CPU priority, parallel workers | BOTTOM | Queue list + controls |

---

## 6. Safety, trust and compliance (`SAF`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| SAF-01 | Protected system paths, not removable | SETTINGS | Locked list |
| SAF-02 | Preview before action (thumbnails, image diff, video snippet) | RIGHT | Preview pane |
| SAF-03 | Undo log / restore session for every operation | BOTTOM / MODAL | Undo button + history list |
| SAF-04 | Operation report: files touched, bytes saved, errors | BOTTOM / MODAL | Report link |
| SAF-05 | Admin elevation only when needed (system folders, WOF API) | MODAL | Elevation prompt |
| SAF-06 | Fully offline, no telemetry, no uploads | SETTINGS | Info line |
| SAF-07 | Detect sensitivity-labelled encrypted (RMS) and password-protected files: skip and report | CENTER / RIGHT | Lock badge + filter |
| SAF-08 | Preserve sensitivity labels (custom properties) when repacking; block conversion to 97-2003 formats | (engine) / RIGHT | Inline notice |
| SAF-09 | Detect digitally signed PDF/Office files: skip by default | CENTER / RIGHT | Signature badge |
| SAF-10 | Synced-folder warning (SharePoint/OneDrive deletion propagates to cloud, retention may apply); suggest Free up space | MODAL | Warning dialog |
| SAF-11 | Lossy and irreversible actions require explicit confirmation | MODAL | Confirm dialog |
| SAF-12 | Audit export: CSV log of every change (file, action, size before/after, hash) | MODAL | Export button |

---

## 7. Automation and performance (`AUT`)

| ID | Feature | Zone | Control |
|---|---|---|---|
| AUT-01 | Command-line mode for scripting | (CLI) | None |
| AUT-02 | Scheduled profiles (e.g. weekly Downloads cleanup + archive cold folders) | SETTINGS | Schedule editor |
| AUT-03 | Windows Explorer context menu ("Optimise with...", "Find duplicates here") | SETTINGS | Toggle |
| AUT-04 | Portable mode (no installation) | (packaging) | None |
| AUT-05 | Multi-threaded hashing and encoding, thread count setting | SETTINGS | Number input |
| AUT-06 | Low-memory mode for very large drives | SETTINGS | Toggle |
| AUT-07 | Temp folder location (fast SSD or RAM disk) | SETTINGS | Path picker |

---

## 8. Key design principles for the wireframe

- **Estimate first, act second**: every action shows its estimated saving before the user clicks Apply.
- **Reversible by default**: Recycle Bin, quarantine and light-copy mode are the defaults; permanent and lossy actions are visually distinct (warning colour) and need confirmation.
- **Progressive disclosure**: advanced controls sit in collapsed "Advanced" sections inside the RIGHT panel.
- **One context panel**: the RIGHT panel adapts to the selected file type or detector, so the screen never needs a second page.
- **Clear status badges in the results grid**: duplicate group, lossy candidate, encrypted (lock), signed, synced (cloud), online-only, protected.
- **Persistent savings counter** in the BOTTOM strip: reclaimable vs reclaimed, updated live.
