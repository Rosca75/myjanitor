# W2-D decisions (simple smart detectors, `crates/detect`)

Format as `docs/DECISIONS.md`. Ids D-2Dxx.

## D-2D01 Rules match index paths, not environment folders
- **Context:** the cache, Office and font-cache detectors must "resolve against the scanned roots"; `DetectContext` carries no home/AppData map.
- **Choice:** rules are case-insensitive path globs (`**/AppData/Local/Google/Chrome/User Data/*/Cache`) matched against the paths in the index, through a pre-filter on the literal last component (`rules.rs`). No `std::env` use except the optional winapp2 `%Var%` map and the system language.
- **Why:** works for any scanned drive (a backup of a profile on D:), is testable on any OS, and stays O(entries).

## D-2D02 Settings carry what the context lacks
- Kept languages: setting `kept_languages` (empty = `LC_ALL`/`LC_MESSAGES`/`LANG`, English always kept). winapp2: setting `winapp2_path` (user's own file, never shipped); `%Var%` expansion uses the process environment (`winapp2::resolve` takes any map).
- Download folders: built-in names plus setting `folders`.

## D-2D03 Selection policy
Pre-selected only when nothing the user wrote can be lost: empty files/folders, zeroed files (24 h minimum age), `.tmp`/`~$`/Thumbs.db, known caches, dev artefacts, installers other than `.exe`/`.AppImage`, dangling links, orphan `.aae/.thm/.lrv`. Never pre-selected: orphan `.xmp` (photo may only have moved), backups/partial downloads/swap files, AutoRecover/UnsavedFiles and Outlook OST, Windows Update/system caches, unknown `~/.cache/<app>`, every review-only detector, stale files, inactive folders (task rule "review-only"; the design card counts selected items, so the engine/UI may choose to preselect).

## D-2D04 Safety class per detector
Kept the stub classes except `LanguagePacks` -> `ReviewOnly` (default action None; Quarantine/Trash available), per CLN-DET-16. `InactiveFolders` stays `Reversible` + Archive (matches the design card) with unselected candidates.

## D-2D05 Saving for archive-type candidates
Stale files and inactive folders set `size_after = size` and `Confidence::Unknown` (archive ratio unknown until the archiver estimates); removal-type candidates set `size_after = 0`, `Exact`. `Candidate::size` is the reclaimable (allocated) size, so folder sizes are block-rounded on disk.

## D-2D06 Empty folders confirmed on disk
The index can miss content of an unreadable folder, so each topmost empty candidate is re-listed with `read_dir` (links count as content, depth-capped); mismatch -> skipped (`not_empty_on_disk`), unreadable -> diagnostics. Folders inside `.git/.svn/.hg/node_modules/...` and zero-byte marker files (`.gitkeep`, `__init__.py`, ...) are never proposed.

## D-2D07 Zeroed files
Sample head/middle/tail (4 KiB each) then streaming confirmation up to `max_size_mb` (256); larger -> diagnostics `too_large_to_confirm`. Skips sparse/online-only, files changed within 24 h, files under 1 KiB, and VM/swap/image extensions that are legitimately preallocated. Parallel pass, serial retry of failed reads (SMB/GVFS concurrency rule).

## D-2D08 Broken files: in-house gzip validator
Truncated gzip cannot be seen from head/tail bytes, and no inflate crate may be added, so `inflate.rs` is a ~250-line puff-style decoder that discards output and checks that every block decodes and the ISIZE trailer matches (CRC not recomputed). Capped by `max_gzip_mb` (32). Other formats: PNG signature+IEND, JPEG SOI+EOI(4 KiB tail), PDF header+`%%EOF`, ZIP EOCD (64 KiB tail; CFB-encrypted OOXML skipped). Never pre-selected.

## D-2D09 Shortcuts and links
`.lnk` is parsed in-house (`lnk.rs`, header + LinkInfo local base path, UTF-16 preferred, ANSI only if ASCII) and evaluated only on Windows, only for `X:\` targets, and only when the drive root exists (an unplugged drive never makes shortcuts "broken"). Off Windows they are counted under `lnk_not_evaluable_on_this_os`. Symlinks: broken only on `NotFound`; targets under `/mnt`, `/media`, `/run/media`, `/Volumes` are skipped unless the target's parent folder exists.

## D-2D10 Dev artefacts
Generic names need their proof file beside them (see module doc); `.gradle` is reported only beside Gradle scripts (so `~/.gradle` is not proposed); `dist` needs `package.json`/`pyproject.toml`/`setup.py`; `venv` needs `pyvenv.cfg`. Docker build cache lives in Docker's store and is reported only as a diagnostics note (`docker builder prune`).

## D-2D11 Language packs
Folder or file names count only when they parse as an ISO 639-1 code (+ script/region), and only in `locale/`, `locales/`, `*.lproj`, `locales/*.pak`, `lang-xx.pak`. Sub-groups are the language code.

## D-2D12 Stale files / inactive folders
Last used = later of access and modification time; access time ignored when missing or equal to creation; a note is added when ~all access times are at or before the modification time. Defaults: 12 months, minimum size 1 MiB (tiny files are noise for archiving). Inactive folders: direct children of a scan root (`depth == 1`), skipping hidden/system folders and personal/OS folders (Documents, Downloads, AppData, ...).

## D-2D13 Unused fonts
"Unused family" is not knowable offline; OPT-FNT-02 lists font cache files/folders (Windows FontCache, `FNTCACHE.DAT`, fontconfig, Adobe `AdobeFnt*.lst`) review-only and says so in `diagnostics.notes`. Installed fonts are never listed.

## D-2D14 Orphan sidecars
Parent matching is by same-folder stem (also `IMG_1.JPG.xmp`, iPhone `IMG_E/O1234.AAE` -> `IMG_1234`, GoPro `GL..` -> `GH..`) against a fresh `read_dir`, so a parent hidden by scan exclusions is still found. `.xmp` in Presets/Settings/CameraRaw/Lightroom/Adobe folders is skipped (presets, not sidecars).

## Contract change requests
- `DetectContext` has no protected-path, home or environment information; detectors do not return protected paths because the engine's `SafetyInspector` badges them (ARCH §3.4). If the engine wants detectors to skip them earlier, add a `protected: &dyn Fn(&Path) -> bool` to the context.
- `Candidate` has no per-candidate safety class or default action; `DetectorInfo` is per card.
