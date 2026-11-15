# Adversarial review 2: destructive code paths after the review-1 fixes

Scope: the frozen snapshot `/home/user/myjanitor-wt/w4rev2` (commit `8aa90a0`), read as an
attacker and as a pessimist. Part A tries to break each fix of review 1; Part B covers what
review 1 did not review. Line numbers refer to this snapshot.

Status legend: **repro** (throwaway reproduction on temp trees, scratch project
`review2/repro` outside the repository), **read** (traced in the code), **suspected**
(plausible, not confirmed; usually Windows-only).

## 1. Verification of review 1

| ID | Verdict | Evidence |
|---|---|---|
| F-01 | partly closed | `check_tree` walks iteratively (no recursion, depth cap 4096), checks every entry against folders/exact/patterns incl. user globs, refuses mounts; linear cost per item. **But** every comparison is lexical on the spelling the scan used: a protected folder spelled through another path (symlinked root, canonical vs link spelling) is not seen below the item. G-01 (repro). |
| F-02 | partly closed | Within one step: survivor per group, full byte compare of the keeper before acting, re-stat right before acting, hard-link-of-itself refused. Across steps of one plan the keeper is not protected: a later step (a folder action containing the keeper, or any step selecting it) removes it after the duplicate went. G-03 (read). |
| F-03 | closed | `ScratchDir` (`core/src/scratch.rs`): `create_dir` with pid/counter/clock name, removed on drop (panic=unwind runs drops); per item in apply and dry run; archiver builds beside `dst` in a private folder and moves in with `rename_no_replace`. Remaining fixed names (`files.txt`, `partial.tar`, `<out>.7z.mjtmp`, `archive.tar`) are all inside private folders. Residuals (Low): default permissions (0755 & umask), a killed process leaves `mj-archive-*` beside the destination (not cleaned by recovery). |
| F-04 | closed | `TreeStamp` = entries, file bytes, newest mtime of the folder **and every subfolder**; compared in preflight, right before acting and (archives) after verification. Adding, removing or renaming anything changes a directory mtime; only an in-place same-size edit that also restores the file's mtime goes unseen (acceptable). Empty folders use `remove_dir`. |
| F-05 | closed | `check_retain_plan` (`actions/src/engine.rs:559`) guards the backup folder like a destination, lexically and canonically, and refuses quarantine/scratch; `quarantine::held` counts only exact slots. |
| F-06 | closed | `output_sha256` compared before undo for every optimise/archive form. Residual: if hashing the output fails at apply time the field is `None` and undo falls back to the size check (`engine.rs:1054`, `1141`). |
| F-07 | closed | `Action::Hardlink` is `LossyOptIn`; `requires_confirmation` includes it (engine, `plan_apply` via `needs_confirmation`), schedules refuse it at save (`validate`) and run time (`apply_steps` + unconfirmed `plan_apply`). Keeper-is-selected, keeper-became-a-link (`guard.check(keeper, .., false)`), cross-volume and already-linked cases are refused. CLI: see below. |
| F-08 | closed | `journal_error` + `require_writable` on every acting command (9 call sites: apply, undo x3, purge, schedules). |
| F-09 | partly closed | Failed source deletion now keeps a done, reversible entry. **New in the same path:** the source is deleted entry by entry after the copy without checking that each entry is still what was copied: anything written or changed in the source during the (possibly long) copy is deleted and exists nowhere. G-02 (repro). |
| F-10 | closed (simulated) | `preflight` re-reads online-only from fresh metadata before any content read; folders count online-only files in the walk. |
| F-11 | closed | `sync_output` on every optimiser output and archive before the original goes; directory sync after the final rename; `move_into_place` syncs. |
| F-12 | accepted (reasonable) | Window narrowed: full guard re-check after the intent, immediately before the act. Exploitation needs a co-resident process racing in the user's own folders. |
| F-13 | partly closed | Lock taken by every persistent engine, recovery skipped without it, acting refused. On Linux the pid-file scheme is fragile: a stale lock whose pid was reused (normal after a crash and reboot) makes the GUI refuse to open; the stale-takeover path can let two processes both "hold" it; `Drop` removes the file by path even when another process now owns it. G-06 (read). |
| F-14 | closed | `pathutil::windows_key` maps `\\?\UNC\` to `\\` (tested). Residual of the same class as G-01: other spellings of one location (`\\.\C:\`, `\\?\Volume{GUID}\`, a mapped or `subst` drive vs its UNC/target) still give different keys. |
| F-15 | closed | `retention_days` journalled per quarantine entry, `purge_expired` honours it; explicit purge applies the given age. Side finding G-13 (entries in `undo_failed` are no longer counted or purged). |
| F-16 | closed | Archive removed only after its own `create` succeeded; optimiser temp left alone on `already_exists`/`conflict`. Minor residual: a `Conflict` the optimiser returns for another reason leaves its temp file beside the original (G-08). |
| F-17 | closed | Every ffmpeg input goes through `media::ffmpeg::input_args` (`-protocol_whitelist file`, demuxer forced from own sniffing; unknown content refused before ffmpeg runs) in audio, video and screen; the only other `-i` uses are `lavfi` test sources. ffprobe is never executed (only listed as a tool). The forced `mov` demuxer keeps `enable_drefs` at its default (off). |
| F-18 | partly closed | Hash check added, but `verify_retained` (`actions/src/retain.rs:205`) returns `Ok` whenever what sits at the retained location is not a regular file: a link or folder put where the moved file was is moved back without a check. G-07 (read). |
| F-19 | closed (fail safe), with gaps | Ambiguous names below the root are refused on Windows. Not covered: DOS device names (`CON`, `NUL`, `COM1`, ...), which `std::fs` also maps differently from the verbatim API; destinations (Move target, backup folder) are not checked at all; legitimate `name~1.ext` files are skipped (documented). Windows-only, suspected. |
| F-20 | closed (read) | HRESULT kept, attributes re-read, `bytes_after` from fresh metadata (`engine.rs:809-823`). |

## 2. New findings

| ID | Sev | Title | Location | Status |
|---|---|---|---|---|
| G-01 | High | Protected paths below a folder are only seen in the scan's own spelling: a protected folder reached through a symlinked root (or protected under its canonical/other spelling) is deleted with its parent | `crates/actions/src/protected.rs:298-305`, `crates/actions/src/guard.rs:236-282`, `guard.rs:162-179` | repro |
| G-02 | High | Cross-volume move deletes source content that changed or appeared during the copy (default Windows trash route for every non-C: drive) | `crates/actions/src/fsops.rs:355-432` | repro |
| G-03 | Medium | "One copy is always kept" holds per step only: a later step of the same plan removes the kept copy (or the folder holding it) | `crates/engine/src/apply.rs:105-220`, `crates/actions/src/engine.rs:337-397` | read |
| G-04 | Medium | A confirmation is not bound to what it confirmed: a rescan keeps the plan's steps but resets the selection to the defaults, and `plan_apply` rebuilds items from the current selection | `crates/engine/src/plan.rs:304-320`, `crates/engine/src/apply.rs:424-446`, `crates/core/src/plan.rs:108-118` | read |
| G-05 | Medium | PowerPoint "Remove unused slide layouts" is on by default, classed lossless/reversible (so it runs unattended), and applies to templates `.potx`/`.potm` whose layouts are their whole content | `crates/optimise/src/office/powerpoint.rs:30,45-51,118-163,365` | read |
| G-06 | Low | Linux instance lock: pid reuse after a crash blocks the GUI; stale takeover race; `Drop` removes someone else's lock | `crates/engine/src/instance.rs:29-84` | read |
| G-07 | Low | Restore moves back a link or folder found where a moved file was, without any check | `crates/actions/src/retain.rs:205-221` | read |
| G-08 | Low | An optimiser `Conflict` error leaves its `.mj-N.tmp` output beside the user's file | `crates/actions/src/engine.rs:923-932` | read |
| G-09 | Low | Archiving a single file: the source is not re-checked between verification and its removal (`recheck_tree` is a no-op for files) | `crates/actions/src/engine.rs:1117-1146`, `594-603` | read |
| G-10 | Low | PDF hybrid files: the `/XRefStm` entries take precedence over the classic table; verification parses the original with the same reader, so a disagreement with other readers is not detected | `crates/formats/src/pdf/document.rs:215-227,263-267` | suspected |
| G-11 | Low | Office media de-duplication also merges identical OLE embeddings (`/embeddings/*.bin`): two embedded objects then share one part, which Office edits as one | `crates/optimise/src/office/package.rs:307-356` | suspected |
| G-12 | Low | EXIF thumbnail/GPS stripping (default JPEG policy `strip_thumbnail`) zeroes out-of-line values shared with IFD0 or lying in the Exif IFD: only the header and IFD0's table are protected | `crates/optimise/src/images/tiff_edit.rs:112-129,163-194` | repro |
| G-13 | Low | A quarantined item whose undo failed (`undo_failed`) is no longer in the quarantine summary and is never purged | `crates/actions/src/quarantine.rs:127-139` | read |
| G-14 | Low | Archiving drops links (listed in `INDEX.csv` only) and, in 7z, empty folders, then removes the sources | `crates/optimise/src/archive/ext.rs:1-9`, `zipfmt.rs:163-167` | read |

Counts: 0 Critical, 2 High, 3 Medium, 9 Low. Review 1: 14 closed (F-10, F-19, F-20 by reading or simulation only), 5 partly closed (F-01, F-02, F-09, F-13, F-18), 0 not closed, F-12 accepted (reasonable).

### G-01 (High): protected entries below a folder, in another spelling

**Scenario.** The scan root is `~/Dropbox` (a symlink to `/data/Dropbox`), or the user
protected `/data/Dropbox/Clients/contracts` while scanning `~/Dropbox`; on Windows a mapped
drive `Z:\` for `\\srv\share`, a `subst` drive, or a junction root. An "inactive folders"
or "large folders" candidate `~/Dropbox/Clients` is trashed or permanently deleted, and the
protected contracts go with it.

**Evidence.** Repro `g_protected_below_through_symlinked_root` (`tests/actions2.rs`):
`root` is a symlink to `real/`; the guard protects `real/proj/contracts`;
`is_protected(real/proj/contracts/c.pdf)` is true; a confirmed PermanentDelete of
`root/proj` gives `succeeded: 1` and the contract is gone. `has_protected_below` compares
`path_key` prefixes of the item's lexical spelling, the walk calls `is_protected` on
`item.path` joined names (same spelling), and `check_canonical` only canonicalises the item
itself (`/real/proj` is not protected, nothing below it is asked).

**Fix.** In `check_tree`, also run `has_protected_below` on the canonical item path, and
match each walked entry both as `item.path/rel` and as `canonical(item)/rel`. Store user
protected folders in both their given and canonical forms (`protected_paths_set`). The scan
badge (`scan_ops.rs:549`) should do the same.

### G-02 (High): cross-volume move deletes what changed during the copy

**Scenario.** On Windows every Trash of a folder (or file) on a drive other than C: goes to
the quarantine under `%LOCALAPPDATA%` by copy + verify + delete; on Linux, Move to another
disk does the same. The copy of a large folder takes minutes. Any file an application writes
into the folder during that time, or any file it rewrites, is then deleted by
`remove_source`, which removes every entry it finds without comparing it with what was
copied. The new file exists nowhere; the rewritten file survives only in its old version.
The journal says `done`, nothing is reported. The tree re-check (`recheck_tree`) runs before
the copy, not after it.

**Evidence.** Repro `g_cross_volume_move_deletes_what_changed_after_copy`:
`move_item_with(src, dst, force_copy = true, ..)`; the injected remover writes
`src/sub/late.txt` and rewrites `src/sub/b.txt` on its first call (the moment right after
the copy finished). Result `MoveHow::Copied` (nothing left behind), `late.txt` exists in
neither tree, `dst/sub/b.txt` holds the old content and the new content is gone. A single
file appended to during `copy_file_verified` loses the appended part the same way.

**Fix.** Record `(relative path, size, mtime, file id)` for every entry as it is copied; in
`remove_source`, delete an entry only when its fresh metadata still matches, else leave it
and count it in `left` (the existing `CopiedSourceLeft` note then tells the user); never
remove a folder that still holds entries. Re-run the folder stamp check after the copy and
before the first deletion.

### G-03 (Medium): the kept copy is not protected across steps

**Scenario.** The plan has "Exact duplicates → Trash" (keeper `~/old-project/photo.jpg`,
duplicate `~/Pictures/photo.jpg`) and then "Inactive folders → Trash" or "Delete
permanently" with `~/old-project`. `plan_apply` builds every step up front; each step's
`group_survivors` looks only at its own selection. Step 1 removes the duplicate (its keeper
still exists, `check_keep` passes); step 2 removes the folder holding the keeper (its tree
stamp is unchanged). Every copy is gone. The reverse order is caught (the keeper is missing
when step 1 runs), so safety depends on step order. Same with any later file step that
selects the keeper (large files, old downloads, similar images with another keep rule).

**Fix.** In `plan_apply`, collect every `KeepCopy.path` of every step and refuse (skip with a
warning, checklist line) any item of any step that is, or contains, a kept copy; or, in
`mj-actions`, keep a per-run set of kept paths and refuse to remove them or their ancestors.

### G-04 (Medium): confirmations are not bound to what was confirmed

**Scenario.** The user reviews "Old downloads → Delete permanently", unchecks the files they
want to keep, opens the dialog ("Delete 12 files permanently?"). A scan finishes meanwhile
(started earlier, or by the user in another view). `plan_after_rescan` keeps the step, the
new record has new candidate ids so the user's exclusions no longer apply, and the
selection is back to the detector's defaults. The user ticks the acknowledgement;
`plan_apply` accepts it by `step_id` alone and builds items from the current selection: the
files the user explicitly unchecked are permanently deleted. Even without a race, after any
rescan the plan silently applies default selections to steps the user had curated.

**Fix.** Put in `StepConfirmation` what the dialog showed (plan generation or a digest of
the item paths, file count and bytes) and refuse `plan_apply` when it differs. After a
rescan, carry selections over by path, or mark steps "needs review" and refuse to apply
them until reopened.

### G-05 (Medium): templates lose their layouts by default

**Scenario.** A corporate `.potx` template holds one sample slide and twenty layouts. The
PowerPoint optimiser runs with defaults (also from a schedule, as the step is reversible and
not lossy): `remove_layouts` keeps only the layouts slides use (one per master when none is
used) and garbage-collects the rest with their media. The output verifies (well-formed,
relationships resolve) and replaces the template; the original sits in the Trash or the
quarantine only until it is purged. The loss is noticed when someone next builds a deck.

**Fix.** Never remove layouts or masters from `.potx`/`.potm` (and arguably from any file
by default); make the option off by default and mark it as changing the document.

### G-06 (Low): Linux instance lock robustness

- pid reuse: after a crash or power cut the file keeps a pid; after a reboot that pid
  usually belongs to some other process, `/proc/<pid>` exists, and every MyJanitor (GUI
  included, which then exits) refuses until the user deletes the file by hand.
- Two processes that both find a stale pid both `remove_file` it; the second removal can
  delete the first one's fresh lock, and both then succeed `create_new`.
- `Drop` removes `myjanitor.lock` by path; in the case above (or with two in-process
  engines) it removes a lock another holder relies on.

**Fix.** Use `flock(LOCK_EX | LOCK_NB)` on a file that is never deleted (the kernel releases
it on exit or crash), as the Windows branch effectively does with share mode 0.

### G-07 (Low): restore without check for non-files

`verify_retained` returns `Ok` when the retained location is not a regular file or the entry
is not a file. A symlink or folder placed where a moved file was is moved back to the
original place. Nothing is lost, the result is wrong. **Fix:** for a file entry, refuse
unless the retained location is a regular file with the recorded hash.

### G-08 (Low): temp output left beside the original

`engine.rs:927` returns any `Error::Conflict` from the optimiser without cleanup, to avoid
deleting a name someone else took. A `Conflict` raised for another reason after the
optimiser wrote `temp` leaves `.name.mj-N.tmp.ext` in the user's folder, recorded only in a
skipped entry. **Fix:** let the optimiser report "destination existed" distinctly (or compare
the temp's file id with one recorded when the job created it).

### G-09 (Low): archived file not re-checked before removal

For a file item, `recheck_tree` returns immediately, so between `archiver.verify` and
`retain::retain` (Delete under confirmation, or Trash) nothing re-stats the source. A write
in that window is lost with the Delete policy. **Fix:** `ctx.guard.restat(item)` after
verification for file items.

### G-10 (Low, suspected): PDF hybrid-file precedence

In a hybrid file the `/XRefStm` entries are merged before the classic table of the same
section (`merge` keeps the first), so an object listed in both is taken from the stream.
ISO 32000 has readers consult the table first and the stream for what the table does not
hold. Verification loads both files with the same reader, so a crafted or broken file whose
two tables disagree would be rewritten to what other viewers do not show, and still verify.
**Fix:** refuse a hybrid file when the table and the stream disagree on an in-use object.

### G-11 (Low, suspected): shared OLE embeddings

`is_media_name` includes `/embeddings/*.bin`. Two identical embedded objects (an OLE
workbook pasted twice) become two relationships to one part; editing one object in Office
can then change, or confuse, the other. **Fix:** de-duplicate `/media/` only.

### G-12 (Low): EXIF edits zero values other structures still use

`strip_ifd1` (run by the default JPEG policy `strip_thumbnail`) and `strip_gps` zero every
region they consider the removed IFD's: its table, the out-of-line values of its entries and
the thumbnail bytes. `check_clear` only refuses an overlap with the TIFF header and IFD0's
*table*; IFD0's out-of-line values, the Exif IFD, the Interop IFD and the MakerNote are not
protected. Writers that pool identical values (both IFDs pointing at one `XResolution`
rational), or maker notes that hold the preview the IFD1 points into, get that data zeroed.
**Evidence:** repro `g_strip_ifd1_zeroes_a_value_shared_with_ifd0` (`tests/tiff.rs`): a
TIFF whose IFD0 and IFD1 share the `XResolution` value; `strip_ifd1` returns `Ok(true)` and
IFD0's rational becomes `0/0`. The scan data is untouched (verified), so the picture is
intact; the metadata is not. **Fix:** collect the regions of every other IFD (tables and
values, Exif/GPS/Interop sub-IFDs, MakerNote) and refuse (keep the segment) on any overlap.

### G-13 (Low): undo-failed quarantine items become invisible

`quarantine::held` keeps only `Done` entries. F-06 and F-18 now leave refused undos in
`UndoFailed` (retryable), but such an item, still in its slot, drops out of the quarantine
summary and is never purged (an invisible disk leak; History can still retry it). **Fix:**
treat `UndoFailed` like `Done` in `held`.

### G-14 (Low): archive loses links and (7z) empty folders

Links inside an archived folder are never stored (listed as "not archived" in `INDEX.csv`),
and `7z` archives store no empty folders; verification accepts both and the sources are
then retained or, with the confirmed Delete policy, deleted. The links and empty folders
exist nowhere afterwards. **Fix:** refuse to remove the sources of a folder that holds
links (or empty folders, for 7z), or store them (`-snl` with the link in the list file; tar
supports both).

## 3. Checked and found sound

- **7z include list** (`-i@file`, cwd = parent of the folder): verified with the real 7z
  (repro `tests/archive7z.rs`). `*`, `?`, `[x]` and a leading `-` in names archive and
  verify correctly; a folder named `old*` makes 7z also add `old-sibling/x.txt` (outside
  the item), and a newline in a name makes 7z fail. In both cases verification refuses (the
  count of listed files must equal the expected set, every expected name must match by size
  and CRC) and the sources stay. Strength by format: ZIP decompresses every entry and
  compares SHA-256; tar.zst decompresses to a private folder and compares SHA-256; 7z relies
  on `7z t` (data vs stored CRC) plus the listed CRC-32 vs a fresh CRC-32 of the source:
  weaker (CRC-32, tool-reported) but adequate against accidents.
- **PDF rewrite**: verification parses original and output independently, walks both graphs
  in lock step (every reachable object, dictionaries key by key, streams by decoded content)
  and checks page count/order, decoded page contents, font and image sets, catalog keys and
  `/ID`; signed (`/ByteRange` anywhere), certified, `/Perms` (incl. Reader usage rights) and
  encrypted files are refused; PDF/A and PDF/X keep the classic layout (no object streams,
  which PDF/A-1 forbids); `/Metadata` streams are never compressed; incremental updates are
  merged newest first and the output must have one revision; inconsistent offsets refuse.
  Its blind spot is shared parsing (G-10).
- **OOXML**: garbage collection removes only parts that became unreachable through the
  operations (reachability is computed before them); every untouched part is
  CRC-identical, relationships must resolve, content types must cover every part; signed,
  encrypted and RMS files are refused; a failing optional operation falls back to the safe
  subset. Excel's style, pivot and name operations are opt-in (not exercised).
- **JPEG rewrite**: segment lengths bounds-checked, MPF refused, entropy data and trailer
  compared byte for byte, frame header compared, only trailing zero padding after an EOI is
  trimmed; Replace bodies are always shorter than the original (no `u16` overflow).
- **Scan cache staleness**: fingerprints are reused only on exact `(size, mtime_ns)` (2 s
  untrusted window); a file replaced with the same size and mtime can carry stale hashes
  into a duplicate group, but `check_keep` compares the two files byte for byte before
  removing (exact groups) and every act re-stats, so a stale entry cannot make an action
  touch changed data.
- **Plan selection**: `build_step` only takes selected, unblocked candidates of the current
  results, within the index roots, with their scan stamp and file id; stale opportunity ids
  are refused (`unknown opportunity`). See G-04 for what a rescan does to selections.
- **CLI**: roots are canonicalised (symlinks, `..`, `.`, trailing separators) before the
  filesystem-root, home and app-folder refusals; `--yes` or a typed `APPLY`, plus
  `--confirm-lossy` (hard links included) / `--confirm-permanent`; originals `delete` is
  never offered. CLI `apply` re-scans and applies default selections of that scan (by
  design, documented).
- **Schedules**: lossy, permanent and hard-link steps refused at save (`validate`) and again
  at run time (`apply_steps` checks the rebuilt plan, `plan_apply` is called unconfirmed and
  `mj-actions` refuses unconfirmed); ids validated before reaching `schtasks`/systemd;
  `ExecStart` words quoted with `%`/`$` doubling, crontab single-quoted, the schedule name is
  folded to one line in `Description=`.
- **IPC**: every acting command goes through the engine (no command takes a raw path to
  act on: items come from scanned candidates by id; destinations go through
  `check_destination`); exports use `create_new` + unique names after the destination check.
  A compromised webview is, by construction, the user: it can set the scope, the user
  protected list and acknowledge confirmations; only the built-in protected set and the
  guard rules remain. That is inherent to the design, not a bug.
- **Undo paths**: Copy undo compares content (file hash or full tree digest) before
  removing; hard-link undo verifies content and replaces only the verified link; restore
  never overwrites; restore targets are re-guarded lexically and for link ancestors.
- **Journal failure, lock refusal and recovery order** (F-08, F-13) as wired in
  `Engine::new`: lock first, journal second, recovery only with the lock.

## 4. Not reviewed

- Windows-only code was read, never executed: WOF, Cloud Files (dehydrate/hydrate), the
  Recycle Bin route, MFT/USN scanners, `share_mode(0)` lock, F-19 refusals, junction
  handling in `check_tree`.
- Excel opt-in transforms (used-range reset, unused named styles remap, conditional
  formatting de-duplication, broken names) and Word opt-in transforms (custom XML,
  template, fonts): only their registration and defaults were checked, not their XML index
  arithmetic.
- `png.rs` encoder output, lossy image paths (cwebp/avifenc/cjxl), video/audio quality
  checks beyond the input whitelist.
- `tar.rs` reader against hostile archives (the tar is our own output, but it is re-read
  after an external tool touched it).
- UI flows (whether a scan can finish while the confirmation dialog is open, see G-04; the
  "Select all" label), `src-tauri` start-up and portable mode.
- Performance of `check_tree` on 200k-entry folders was reasoned (linear; three to four
  walks per folder item: preflight, act, `tree_size`, manifest), not measured.

## Reproductions

Scratch cargo project `review2/repro` in the session scratch directory (outside the
repository), depending by path on the snapshot's `mj-core`, `mj-platform`, `mj-actions`,
`mj-formats`, `mj-optimise`, `mj-engine`, `mj-scan`; `TMPDIR` set to a scratch folder.

- `tests/actions2.rs`: `g_protected_below_through_symlinked_root` (G-01),
  `g_cross_volume_move_deletes_what_changed_after_copy` (G-02). Both fail on this snapshot
  (they assert the safe behaviour).
- `tests/tiff.rs`: `g_strip_ifd1_zeroes_a_value_shared_with_ifd0` (G-12), fails.
- `tests/archive7z.rs`: real 7z with wildcard, newline, dash and bracket names (sound).
