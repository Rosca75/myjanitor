# Adversarial review 1: destructive code paths

Scope: every path that deletes, moves, overwrites, links or compresses user files, read as
an attacker and as a pessimist. I read the main checkout on branch
`claude/eloquent-shannon-4pcqld` as it is now. Line numbers refer to that state.

Status legend:

- **repro**: verified by a throwaway reproduction (cargo tests under the review scratch
  directory, on temp trees only);
- **read**: verified by tracing the code path;
- **suspected**: plausible from the code, not confirmed (usually Windows-only paths that
  cannot be exercised from this Linux session).

Severity is ranked by impact on user data first.

## Summary

| ID | Sev | Title | Location | Status |
|---|---|---|---|---|
| F-01 | Critical | Protected paths are only checked on the item itself: removing a folder removes protected files, `.git`, `.ssh` and app folders inside it | `crates/actions/src/guard.rs:72-115`, `crates/actions/src/protected.rs:204` | repro (R7) |
| F-02 | High | "Select all" selects the keeper: every copy of a duplicate group can be removed, including by permanent delete | `crates/engine/src/steps.rs:166-191`, `crates/engine/src/apply.rs:126-170` | repro (R5) |
| F-03 | High | Optimisers write to fixed scratch names in the shared temp dir. Concurrent runs produce a corrupted output that passes every check and replaces the original | `crates/optimise/src/media/{audio.rs:344,video.rs:501,screen.rs:421}`, `images/{lossy.rs:209,jpeg.rs:166,png.rs:155}`, `crates/engine/src/apply.rs:184-194` | repro (R6) |
| F-04 | High | A folder's stamp is never compared, so a folder that changed after the scan is removed with its new content | `crates/actions/src/guard.rs:177-178`, `crates/engine/src/apply.rs:155-159` | repro (R1) |
| F-05 | Medium | `OriginalsPolicy::BackupFolder` bypasses the guard: it can point into protected or app folders, and a backup inside the quarantine is purged | `crates/actions/src/retain.rs:74-90` | repro (R3, R3b) |
| F-06 | Medium | Undo of an optimisation or archive only checks the output's size, so it silently discards later same-size edits | `crates/actions/src/undo.rs:344-386` | repro (R2) |
| F-07 | Medium | A hard link is journalled as reversible without confirmation, but a later edit of either name destroys the other copy and undo then refuses | `crates/core/src/action.rs:187`, `crates/actions/src/engine.rs:523-588` | repro (R4) |
| F-08 | Medium | If the journal fails to open, the app silently falls back to an in-memory journal and still acts: no crash recovery, no undo after restart, quarantine contents orphaned | `crates/engine/src/engine.rs:124` | read |
| F-09 | Medium | A partial failure of a cross-volume move leaves the item split between source and destination, with a `failed`, non-undoable entry and no pointer to the copy (the default Windows trash route for every non-C: drive) | `crates/actions/src/fsops.rs:340-357`, `retain.rs:118-142` | read |
| F-10 | Medium | Online-only state is never re-checked at act time, and folders holding online-only files are moved out of the sync root (Windows) | `crates/engine/src/apply.rs:127`, `crates/actions/src/engine.rs:268-292` | suspected |
| F-11 | Medium | Outputs of external tools are never fsynced before the original is moved away or replaced | `crates/actions/src/engine.rs:706-776`, `crates/optimise/src/images/common.rs:38-55` | read |
| F-12 | Low | Check-then-act by path: no dirfd/handle anchoring; parent swaps between re-stat and act are not caught | `crates/actions/src/engine.rs:357-392`, `fsops.rs` | read |
| F-13 | Low | No single-instance lock: a second process (CLI or scheduled run) "recovers" the first one's in-flight intents and deletes its temp outputs | `crates/engine/src/history.rs:56-64`, `crates/actions/src/undo.rs:426-540` | read |
| F-14 | Low | `path_key` keeps `\\?\UNC\` distinct from `\\server\`: protected UNC entries and allowlist roots can be bypassed by the other spelling | `crates/core/src/pathutil.rs:200-206` | read |
| F-15 | Low | Quarantine purge ignores the step's `purge_after_days` and uses the global setting | `crates/actions/src/quarantine.rs:143-170`, `engine.rs:416-435` | read |
| F-16 | Low | Archive and optimise clean-up removes the destination even when creating it failed because it already existed (race) | `crates/actions/src/engine.rs:839-845` | read |
| F-17 | Low | ffmpeg is run with content probing and no `-protocol_whitelist`, so a hostile "video" (HLS/concat playlist) can pull local files into the output | `crates/optimise/src/media/ffmpeg.rs:14-21` | suspected |
| F-18 | Low | Undo of Move/Trash/Quarantine does not verify content against `original_sha256` before moving the item back | `crates/actions/src/retain.rs:146-171`, `trash.rs:416-466` | read |
| F-19 | Low | Windows path spelling (trailing dot or space, 8.3 names) is not canonicalised on the final component, and `wide_path` (verbatim) and `std::fs` (normalised) can address different files | `crates/actions/src/guard.rs:121-140`, `crates/platform/src/ffi/windows.rs:73-92` | suspected |
| F-20 | Low | `dehydrate` ignores the `CfDehydratePlaceholder` HRESULT and journals `done` with `bytes_after = 0` | `crates/platform/src/action/cloud.rs:421-431` | read |

Counts: 1 Critical, 3 High, 7 Medium, 9 Low.

---

## Findings

### F-01 (Critical): protected paths are only checked on the item, never below it

**What is wrong.** `Guard::check` and `check_canonical` call `ProtectedPaths::is_protected`
on the item path only. `is_protected` returns true when the path is inside a protected
folder, equals an exact entry or matches a pattern. It never asks whether a protected path
lies *below* the item. The scan's `protected` badge (`safety.rs:80-83`) has the same blind
spot. Every folder action (Trash, Quarantine, PermanentDelete, Move, Archive with
originals removed) therefore removes protected descendants wholesale.

**Scenario.** The user protects `~/Work/old-project/contracts`. "Inactive folders"
(`detect/src/inactive.rs`, actions Archive/Move/Trash), a search for large folders, or
dev artefacts proposes `~/Work/old-project`. Applying it removes the protected contracts
and the project's `.git` (a built-in protected pattern). With PermanentDelete they are gone
for good. The same applies to the app's own folders: with the GUI scope at `~` (the GUI
does not refuse the home root; only the CLI does), a folder candidate such as
`~/.local/share` or `%LOCALAPPDATA%` would take the journal and the quarantine with it.

**Evidence.** Repro R7 (`tests/actions_repro.rs`): a guard with the user-protected folder
`root/old-project/contracts` (plus the `.git` pattern), a confirmed PermanentDelete of
`root/old-project`. `is_protected(contract)` is true, the action outcome is
`succeeded: 1`, and the contract no longer exists.

**Fix.** For `TargetKind::Folder` (and Move, Archive or Copy of a folder), refuse when any
protected folder or exact entry is below the item (`is_within(item, protected)`). Also walk
the subtree (it is already walked for `tree_size`) and refuse on any pattern match and on
the app dirs. Badge such folders `protected` at scan time too. Add a test per action kind.

### F-02 (High): every copy of a duplicate group can be selected and removed

**What is wrong.** The keeper is an ordinary candidate whose `KEEPER` badge only clears
`default_selected` (`Badges::AUTO_UNCHECK`). It is not in `Badges::BLOCKING`.
`selection_set` with `ids: None` selects every candidate of the step, and that is exactly
what the step table's header "Select all" sends (`ui/js/ctl-step.js:83-87`). An explicit
id list including the keeper works too. Nothing in `build_step`, `refresh_plan`, the
confirmation dialog or `mj-actions` enforces "keep one per group". Only Hardlink and
Reflink skip the keeper (because they need it as the link source).

**Scenario.** In "Exact duplicates" the user ticks "Select all" (a natural gesture) and
applies. Both copies of every pair go to Trash or quarantine. With the PermanentDelete
action (offered by `dupes/src/exact.rs:36-39`), the dialog says "Delete N files
permanently?" and every copy is gone. The keep-rule help text even promises "Exactly one
file of every group is always kept" (`dupes/src/keep.rs`).

**Evidence.** Repro R5 (`tests/engine_repro.rs`) drives the real `Engine`: scan of two
identical files, `selection_set { ids: None, selected: true }` → summary
`selected: 2, size_after: 0`, `plan_add_step`, `plan_apply` with no confirmation. Both
files are gone from the tree. The checklist only says "Every reversible step can be undone".

**Fix.** Enforce it in the engine, and again in `mj-actions` for defence in depth. In
`build_step`, for opportunities with groups and a removing action, refuse (or drop with a
warning) any group in which every member present on disk is selected. In
`selection_set`, never select a `KEEPER` row through `ids: None`. Add a checklist and
confirmation line for "N groups would lose every copy".

### F-03 (High): fixed scratch names in the shared temp dir corrupt outputs under concurrency

**What is wrong.** Several optimisers write their intermediate output to a constant name
in `job.temp_dir`, and that directory is the engine's single `cfg.temp_dir` (shared by
every job and by every process using the same cache dir). The names are
`mj-audio-out.<ext>`, `mj-video-out.<ext>`, `mj-screen-audio.opus`, `mj-image-out.<ext>`,
`oxipng-out.png`, `stage1.jpg` and `stage2.jpg`. Each run even starts by deleting that
name, removing a concurrent run's output. `plan_dry_run` runs optimisers on a detached
thread (`spawn_detached`) and does not refuse while an apply job is running. A scheduled
`myjanitor-cli run-profile` and the GUI share the same cache dir. Two optimisations of
different files can therefore overlap on the same scratch file.

**Scenario.** An apply is re-encoding A while the user presses "Estimate exactly" (or a
scheduled run starts) and B is encoded at the same moment. One run fails. The other gets
an interleaved file of the right size and duration, which passes the duration check, the
magic check and "smaller than before". `mj-actions` then sends A's original to the
Trash/quarantine (or deletes it under the confirmed `Delete` policy) and puts the corrupted
file in its place. JPEG's check (dimensions and markers only) would also accept another
photo from the same camera.

**Evidence.** Repro R6 (`tests/optimise_repro.rs`) runs the real `media::audio::Audio`
optimiser with the system ffmpeg on two 20 s sources, twice concurrently with one temp dir,
12 rounds. In every round one run fails ("ffmpeg finished but wrote no output") and the
other returns success with a 93,092-byte output (the expected size) whose content is
neither reference encoding. ffprobe reports the correct 20.0065 s duration, and decoding it
reports 5 errors.

**Fix.** Give every job a private scratch folder: `temp_dir/<job id>/<entry id>/`, created
with `create_dir` (not `_all`) and removed afterwards. Or derive scratch names from
`job.dst`'s unique name. Refuse `plan_dry_run` while an apply runs. Add an
inter-process lock on the temp dir (see F-13).

### F-04 (High): folders that changed after the scan are acted on with their new content

**What is wrong.** `build_step` gives folder items the stamp `0/0`, and
`Guard::compare_stamp` compares stamps only for `TargetKind::File`. A folder's `file_id` is
compared but does not change when its contents change. So an "empty folder", a cache, a
`node_modules` or an inactive folder is removed with whatever it holds now, even if the
user has worked in it since the scan.

**Scenario.** The user scans, adds "Empty folders" with "Delete permanently" (available in
`detect/src/empty.rs:45`), and saves a document into one of those folders before applying.
The confirmed step permanently deletes the document. The dialog reported only empty
folders.

**Evidence.** Repro R1: a folder item captured while empty, then
`was-empty/thesis.docx` written, then a confirmed PermanentDelete → `succeeded: 1` and the
document is gone.

**Fix.** Record a folder fingerprint at scan time (entry count plus the newest mtime in
the subtree, or the subtree byte total the index already computes). Re-check it in
`check_item` and skip with `changed` on mismatch. For "empty folder" candidates, act with
`remove_dir` (fails when not empty) instead of `remove_dir_all`.

### F-05 (Medium): the backup folder of the originals policy is never guarded

**What is wrong.** `retain::plan` accepts any `BackupFolder { folder }`. It is never passed
to `Guard::check_destination`, unlike Move and Copy. Settings only check that it is
absolute, and the step action can carry any folder over IPC. The folder can be a protected
location, inside the item, behind a symlink, or inside the quarantine. `quarantine::held`
treats every `done` entry whose `retained_at` is lexically inside the quarantine as held,
so `purge` permanently deletes such backups.

**Evidence.** Repro R3: originals backed up to `<quarantine>/backups`, then
`quarantine::purge` → the backed-up original is deleted (4,000 bytes freed). Repro R3b:
`check_destination` refuses a protected folder (`protected`), but the same folder as
`BackupFolder` is accepted and written into.

**Fix.** Call `guard.check_destination(dest.parent(), path)` in `retain::plan` for
`Backup` (or in `optimise` and `archive` before the intent). Refuse folders inside the
quarantine, journal or temp dir. In `quarantine::held`, only count entries whose
`retained_at` is the slot computed by `quarantine::slot` for that entry.

### F-06 (Medium): optimisation and archive undo destroys same-size edits

**What is wrong.** `undo_output` only checks that the output's size equals `bytes_after`.
It then sets the output aside, restores the original and deletes the output. An edit that
keeps the length is discarded. Examples: a changed byte in a re-saved document, an image
edited in place by a tool that keeps the size, a PDF form filled with fixed-width
fields. The journal has no hash of the output.

**Evidence.** Repro R2: an optimise with quarantined originals, then the output rewritten
with different content of the same length, then undo → `succeeded: 1`. The original is back
and the edited version is deleted: only `doc.bin` remains in the folder.

**Fix.** Journal the output's SHA-256 (it is in hand: the engine already re-reads files to
hash them) and compare it before undo. On mismatch, keep the output beside the restored
original (`name (MyJanitor kept).ext`) rather than deleting it, and report a conflict.

### F-07 (Medium): hard links are "reversible" without confirmation but can become irreversible

**What is wrong.** `Action::Hardlink` is `SafetyClass::Reversible`, needs no
confirmation and may run in schedules. After linking, the keeper and the former duplicate
are one inode. Any in-place write through either name changes both (applications that
save in place, logs, databases, two identical config files of different apps or
profiles). The undo then refuses ("modified after linking"), and the duplicate's
independent content is gone. The duplicate's own owner, ACLs, xattrs and ADS are also
replaced by the keeper's and only mode and mtime are restored. Reflinks do not have this
problem.

**Evidence.** Repro R4: hardlink, then edit the keeper → the "duplicate" reads the new
content, History still lists the entry as undoable, and the undo fails with `conflict`.

**Fix.** Classify Hardlink as at least `lossy_opt_in`-style "needs confirmation", with copy
that explains shared writes. Exclude it from schedules. Prefer Reflink where supported. Never
offer it under app-data or config locations. Journal and restore the duplicate's ownership
and ACLs where possible.

### F-08 (Medium): an unreadable journal silently becomes an in-memory journal

**What is wrong.** `Journal::open(...).unwrap_or_else(|_| Journal::in_memory())` runs when
the journal dir cannot be read or a month file cannot be read (permissions, I/O error, a
file locked by antivirus). The only signal is `app_info.journal_persistent = false`, shown
as a notice in Settings. Apply, undo and purge all keep working. Every "journal before act"
guarantee then holds only for the life of the process. After a crash or restart there is
no recovery, no History and no undo. Quarantined items are never purged and are invisible
(the quarantine summary comes from the journal).

**Fix.** When `persist` is on and the journal cannot be opened, refuse every mutating
command (`plan_apply`, undo, purge, schedules) with an actionable error. Do not fall back
silently.

### F-09 (Medium): partial cross-volume moves strand data

**What is wrong.** `move_item` falls back to `copy_item_verified` + `remove_item` across
volumes. If `remove_dir_all` on the source fails half way (read-only or in-use file,
permission, antivirus lock), the error propagates. `retain` then calls
`quarantine::cleanup_slot` (which deletes the manifest), and the entry is recorded `failed`
and not reversible. Part of the tree now exists only in the quarantine slot (or the Move
destination). The journal gives no pointer to that copy, `held()` ignores `failed`
entries, and History cannot restore anything. On Windows, the default "Trash" route is the
quarantine under `%LOCALAPPDATA%`, so **every** trash of a folder on a drive other than C:
takes this path. Cross-volume copies also drop ADS, ACLs, attributes and creation time,
and an undo copies back without them.

**Fix.** After a successful verified copy, a failed source delete should keep the entry
`done`/reversible with `retained_at` set, a note about what remains at the source, and the
manifest kept. Better: delete the source file by file and record progress. Preserve ADS
and ACLs on Windows or refuse cross-volume quarantine and use a quarantine folder on the
item's own volume.

### F-10 (Medium, suspected): online-only state is not re-checked at act time (Windows)

**What is wrong.** Online-only rows are filtered out in `build_step` using the scan-time
badge only. `preflight` re-checks `ENCRYPTED` but not `ONLINE_ONLY`. Storage Sense or
OneDrive often dehydrates files without changing size or mtime, so the stamp check passes.
Folder items are never inspected for online-only children. Moving placeholders out of a
sync root into the quarantine makes the provider delete them in the cloud. The quarantined
stub has no local data, so "undo" can bring back an empty placeholder. Separately,
`preflight`'s `content_badges` and the audit `hash_file` read file content, which
hydrates (downloads) a placeholder that slipped through.

**Fix.** In `preflight`, refuse removal of entries whose fresh metadata is online-only.
For folders, walk the subtree (attributes only, no data reads) and refuse or warn when it
holds online-only entries. Read content (badges, hash) only after that check.

### F-11 (Medium): external-tool outputs are not durable before the original goes

**What is wrong.** Outputs written by ffmpeg, cwebp, avifenc and cjxl are moved into
`job.dst` with `rename`, or `std::fs::copy` across volumes (`common::move_into_place`),
and are never fsynced. The action engine then moves the original away or, with
`RetainPlan::Delete`, `rename`s the temp over it, and writes `done`. None of those steps
syncs the new file or the directory (only `link_replace` syncs the dir). After a power cut,
the file in place can be empty or truncated while the journal says `done`. The undo then
refuses because the size no longer matches `bytes_after`, and for `Delete` the original is
gone.

**Fix.** In `optimise` (engine side, so it covers every optimiser), open the temp file and
`sync_all()` it before `retain`, and `sync_dir(parent)` after the final rename.

### F-12 (Low): path-based check-then-act

The guard lstat-walks ancestors and canonicalises the parent, then `run_acted` re-stats the
item (which catches most swaps, because the file id then differs). The act itself
(`rename`, `remove_dir_all`, `hard_link`) re-resolves the full path with intermediate
symlinks followed. A local process that swaps a parent directory for a symlink between the
re-stat and the act can redirect the operation outside the root. Only a co-resident
attacker can do this. Fix: open the parent with `O_PATH|O_NOFOLLOW` (or a Windows handle
with `FILE_FLAG_OPEN_REPARSE_POINT`), verify its id, and use `renameat2` / `unlinkat` /
`linkat` relative to it.

### F-13 (Low): no single-instance lock

`Engine::new` calls `recover_journal`, which settles every pending intent, including those
of another live process. `inspect_action` deletes `.mj-` temp outputs and hardlink temp
names it believes are orphans, which makes the other process's in-flight item fail. Two
processes also append to the same JSONL files with diverging in-memory views. Fix: take an
exclusive lock file in the data dir. Recover only when the lock was free, and make the CLI
refuse to apply (or wait) while the GUI holds it.

### F-14 (Low): `\\?\UNC\` and `\\server\` compare differently

`path_key` strips `//?/` except for `unc/`, so `\\?\UNC\srv\share\x` and `\\srv\share\x`
get different keys. A user-protected `\\srv\share\keep` does not protect items scanned under
a `\\?\UNC\srv\share` root, and the reverse also holds. Fix: map `//?/unc/` to `//` in
`path_key`.

### F-15 (Low): per-step quarantine retention is ignored by purge

`Action::Quarantine { purge_after_days }` only shortens the undo deadline. `purge` uses the
caller's `days` (the global `quarantine_days`) for every entry. A step that asked for 90
days is purged at 30. Fix: journal the retention per entry and purge by it.

### F-16 (Low): cleanup can delete a file the action did not create

In `archive`, any error from `archiver.create` leads to `remove_file(&dst)`. If `create`
failed because `dst` appeared between `unique_path` and the `create_new` open (a sync
client, a second instance), the engine deletes that foreign file. The optimise temp has the
same pattern, but its name is improbable. Fix: delete only when the error is not
`AlreadyExists`, or have `create` return whether it created the file.

### F-17 (Low, suspected): ffmpeg reads attacker-chosen inputs

Inputs are given as `-i <path>` with format probing. A file named `.mp4` that is really an
HLS or concat playlist makes ffmpeg open other local files (and possibly network URLs
depending on build defaults). Their content ends up in the "optimised" output, which
replaces the original. Fix: pass `-protocol_whitelist file` and force the demuxer from
MyJanitor's own sniffing (`-f mov`/`matroska`...), or refuse inputs whose sniffed type
disagrees.

### F-18 (Low): restores do not verify what they move back

`retain::restore` and `restore_freedesktop` move whatever is at `retained_at` or at the
trash location back, without checking `original_sha256`. If the user replaced the moved
file at a Move destination, or the trash name was reused after the user emptied the trash
and re-trashed a different file with the same original path, the "undo" moves an unrelated
file. Nothing is lost, but the result is wrong and silent. Fix: hash-check files before
restoring and report a conflict on mismatch.

### F-19 (Low, suspected): Windows spellings of the final component

`check_canonical` resolves the parent but not the final component, and the lexical
protected check sees the raw name. So `C:\PROGRA~1`, `C:\Windows.` or `C:\Windows ` as the
item itself pass when the root is `C:\`. Items normally come from the scanner (long names),
so this needs a crafted path. Separately, `wide_path` adds `\\?\` (no normalisation) while
`std::fs` normalises trailing dots and spaces. For a file really named `x.` (possible via
WSL, Samba or archives), `remove_item`/`fs::rename` and `MoveFileExW` can address `x` and
`x.` respectively. The stamp and id check makes a wrong-target act unlikely. Fix: resolve
the final component with `GetFinalPathNameByHandleW` on a handle opened with
`FILE_FLAG_OPEN_REPARSE_POINT`, and use one path API consistently.

### F-20 (Low): dehydrate reports success unconditionally

`dehydrate` sets the unpinned attribute, ignores `CfDehydratePlaceholder`'s HRESULT, and
the engine records `done` with `bytes_after = 0`. Reports and "reclaimed" overstate the
savings. No data loss. Fix: check the HRESULT and re-read the attributes.

---

## Checked and found sound

- Allowlist containment: `is_within` compares by component (`/a/root` vs `/a/root2` is
  correct); roots are never acted on; relative paths and `..` are refused; the root-relative
  lstat walk refuses symlinks and junctions on ancestors and mount points (different volume
  id); the canonical-parent check catches links the walk missed.
- `rename_no_replace`: `renameat2(RENAME_NOREPLACE)` with a link+unlink fallback;
  `MoveFileExW` without REPLACE. Restore, quarantine, light copies and exports never
  overwrite (`create_new` everywhere I looked, `unique_path`).
- Copies never follow links (recreated on Unix, refused on Windows); `remove_dir_all` does
  not follow links; folder copies have a depth limit; cross-volume copies are verified
  (size + xxh64 re-read) before the source is removed.
- Journal: intent written and `sync_data`'d before acting; a journal write failure stops
  the batch; torn last lines are tolerated; ids are reserved per run; panics after intent
  are settled; recovery inspects and records without moving user data (except its own
  temp names).
- SAF-11 in depth: `mj-actions` refuses PermanentDelete, originals `Delete` and archive
  source deletion without `confirmed`, and refuses lossy optimiser output when not
  confirmed. `plan_apply` requires an acknowledged `StepConfirmation`. The CLI requires
  `--yes` or a typed confirmation, plus `--confirm-lossy`/`--confirm-permanent`, and
  never offers originals `delete`. Schedules refuse lossy and permanent steps at save time
  and again at run time, and apply unconfirmed.
- Keep-if-not-smaller is enforced in the engine; the optimise rollback restores the
  original if the final rename fails; OOXML/ZIP repack writes to a new file, fsyncs and
  re-verifies every entry's CRC and size; signed, encrypted and APK archives are refused.
- Hostile inputs: the ZIP reader bounds the EOCD search, the central directory size and
  entry counts, uses checked offsets, caps the inflate output and checks the CRC; XML has
  depth, size and attribute limits; the PDF lexer has a depth limit, the xref chain has a
  visited set and a section cap, object and stream decoding are bounded (256 MiB per
  stream).
- External processes: no shell, argument vectors only, `path_arg` prefixes `-` names,
  timeouts, output caps, kill on cancel.
- Freedesktop trash: `O_EXCL` info reservation, sticky `.Trash` / `.Trash-$uid` checks
  without following links, relative `Path=` for volume trashes, restore checks the
  recorded original and refuses an occupied target; no fallback to deletion when no trash
  is usable.
- Windows Recycle Bin (opt-in route): `FOF_WANTNUKEWARNING`, fixed drives only,
  `SHFILEOPSTRUCTW` layout asserted at compile time; Windows trash defaults to the
  quarantine.
- FFI: the WOF `FSCTL` codes and buffer layout match `winioctl.h`/`wofapi.h` (unit-tested);
  the Btrfs defrag args are 48 bytes; handles are closed by `OwnedHandle`; wide strings
  are NUL-terminated; the reflink is `create_new` + `O_NOFOLLOW` and is removed on failure.
- Exports: CSV formula neutralisation, HTML escaping of every value, `create_new` +
  unique names, the protected-destination check.
- Tauri surface: a strict CSP, no `innerHTML` in the UI, paths reach actions only as scanned
  candidates (by id) or as explicit destinations; schedule ids are validated before they
  reach schtasks/systemd; systemd `ExecStart` and crontab lines are quoted.

## Not reviewed

- `crates/optimise/src/pdf.rs` and `formats/pdf/{writer,structural,verify}.rs` output
  correctness (I checked only the parser bounds); `office/{word,excel,powerpoint}.rs`
  content transformations (I checked only the shared repack and verify path).
- `images/{jpeg_rewrite,tiff_edit}.rs` in-place metadata edits; `png.rs` encoder output.
- `archive/{tar,ext}.rs` verification against the external tools' actual behaviour
  (7z `a` appends to an existing archive; I did not test it).
- Windows-only code beyond reading it: Recycle Bin restore, cloud APIs, WOF, and the MFT/USN
  scanners. None of it was executed.
- `crates/scan/**` and the index cache staleness rules (beyond what the guard compares).
- The UI flows (whether "Estimate exactly" is disabled during an apply, the dialog copy for
  folder steps) and `src-tauri` start-up and portable mode.

## Reproductions

The throwaway cargo project is `review/repro` in the session scratch directory. It depends
by path on `mj-core`, `mj-platform`, `mj-actions`, `mj-engine` and `mj-optimise`, and runs
with `TMPDIR` set to a scratch subfolder.

- `tests/actions_repro.rs`: R1 (F-04), R2 (F-06), R3/R3b (F-05), R4 (F-07), R7 (F-01).
- `tests/engine_repro.rs`: R5 (F-02).
- `tests/optimise_repro.rs`: R6 (F-03, needs `ffmpeg`).

All seven reproduced the described behaviour on this checkout.
