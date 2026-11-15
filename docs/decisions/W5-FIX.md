# W5-FIX: review-2 fixes and T1 gap closure

Decisions taken without the owner (nobody could be asked), by "user data safety first".
The W4-WIN path rules are kept: every new comparison goes through `path_key`, canonical
forms come from `std::fs::canonicalize` (whose `\\?\` and `\\?\UNC\` prefixes `path_key`
folds), and nothing above an allowed root is judged by its spelling.

## D-F1 Canonical spellings are added, never substituted (G-01)

The protected set keeps every entry in its given spelling and adds the canonical one when
the path exists; `check_tree` checks both the item spelling and the canonical spelling of
the item. Adding is chosen over replacing because a path that does not exist yet (a
protected folder created later) still matches lexically, and a link removed later does not
silently drop protection. Cost: one `canonicalize` per protected entry when the set is
built, one per folder item, one per scan root per badging pass.

## D-F2 A source that changed before the first deletion: our copy goes, the source stays (G-02)

The brief asked to "stop and report, leaving both sides". When the mismatch is found before
anything was deleted, the source is complete and newer than the copy; keeping the copy
would leave a stale duplicate in the quarantine (and an undo that refuses because the
original place is occupied). The copy is ours (created by this call, verified), so it is
removed and the item is skipped as `changed`. When a change happens after the first
deletion, both sides are kept and journalled exactly as the brief says (`CopiedSourceLeft`,
`done`, `retained_at` at the copy, a note naming what was left behind).

## D-F3 Signatures instead of re-hashing at deletion time (G-02)

The per-entry check at deletion time compares a fresh lstat signature (size, mtime and on
Unix dev, inode and ctime, which any write, truncate or rename-over moves) with the one
taken when the entry was copied. Re-hashing every file again would double the I/O of a
long move for no additional case on Unix; on Windows (no ctime in std) a same-size write
that also restores the mtime would go unseen (accepted, same residual as F-04).

## D-F4 Kept copies are protected at plan level (G-03)

All items of a run are built by `plan_apply` (GUI, CLI, schedules), so the global set of
kept copies is computed there, once, and both step orders are covered. Over-conservative
by design: two steps that each keep what the other would remove both skip.

## D-F5 Selection digest instead of a plan generation (G-04)

A digest of what the step would act on survives a rescan that found nothing new (the user
is not asked again for nothing), and changes whenever a row, its stamp or the action
changes. The originals and synced-folder choices are the dialog's own answers and are not
part of it. Selections are carried over by path; a checked row stays checked only with an
unchanged stamp (an edited file must be reviewed again); unchecked rows always stay
unchecked.

## D-F6 Templates are recognised by content type (G-05)

The optimiser's operations see the package, not the file name; the main part's content
type is what Office itself uses. A missing content type counts as a template (skip).

## D-F7 The instance lock file is never deleted (G-06)

Deleting the file on exit reopens the race the review described (a second process could
lock a new inode while the first still holds the old one). The empty-ish file stays in the
data folder; its pid text is for people only.

## D-F8 Archives refuse rather than store links (G-14)

Storing links would change the archive format contract and the verification of three
formats; a refusal with a way out ("keep the originals") is contained and loses nothing.

## Part B: T1 gap closure

| Item | Feature | Result | Commit | Tests |
|---|---|---|---|---|
| 1 | CLN-ACT-04 | done: expired quarantine items purged (guarded `purge_expired`, each item's own retention) at GUI engine start, at the start of acting CLI commands (`apply`, `clean --apply`, `undo`, `run-profile`, `quarantine purge`) and after each applied step; CLI `quarantine list [--json]` and `quarantine purge [--older-than DAYS --yes]`; UI copy says so | `b2590b5` | `t1_gaps::cln_act_04_*`, cli `quarantine_list_and_purge` |
| 2 | CLN-ACT-05 | done: Copy offered wherever Move is; step panel Copy… with the in-app folder browser and "Keep folder structure" (default on) for Move and Copy; CLI `--action move\|copy --to DIR [--flat]`; destinations refused at plan time when relative, protected or inside a scanned location, and checked by the guard per item | `edaeb5d` | `t1_gaps::cln_act_05_*`, `wiring.test.js` |
| 3 | CLN-ACT-02 | done: setting `general.windows_recycle_bin` (default off, help says restore is best effort); `app_info.trash_route` (`quarantine`/`recycle_bin`/`trash`) drives the step tile and the dialog label | `a51b369` | `t1_gaps::cln_act_02_*`, `step.test.js` |
| 4 | CLN-ACT-11 | done: `mj_formats::pdf::table` text-only PDF (Helvetica/WinAnsi, escaped, `?` replacement counted and noted), `ExportFormat::Pdf` for report and results exports, UI buttons, CLI `report --format pdf`, `show ID --format csv\|html\|pdf --out FILE` | `bb39fa0` | `pdf::table::tests` (re-parsed), `t1_gaps::cln_act_11_*`, `w3.test.js` |
| 5 | CLN-ACT-06 | done: duplicates step offers hard links (explanation, F-07 confirmation) and reflinks, each disabled with its reason without the capability | `5d70430` | `wiring.test.js` |
| 6 | AUT-05/06/07 | done: `temp_dir` used for every apply/dry-run scratch folder, refused inside a scanned or protected location; `threads` drive the scan walk/hash pool (detector pools already followed it); `low_memory`, `parallel_jobs`, `cpu_priority` shown disabled with the reason | `ba3d777` | `t1_gaps::aut_07_*`, `settings.test.js` |
| 7 | CLN-DET-08 | done: WAV/RIFF size, FLAC STREAMINFO, MP3 frame sync after ID3v2, Ogg page magic, then the media prober | `8130cbd` | `broken::tests::cln_det_08_*` |
| 8 | CLN-FLT-06 | partial: Linux names from `/etc/passwd` (cached) match `owner:` filters; Windows owner SID not collected (the FFI was judged not contained enough to add unexercised) | `30871a7` | `users::tests`, `eval_tests` |
| 9 | CLN-CNT-01 | done (best effort): PDF, DOCX, XLSX, PPTX extractors | `5d7185d` | `extract::tests`, `t1_gaps::cln_cnt_01_*` |

### D-F9 Settings that do nothing are disabled, not removed

`low_memory`, `parallel_jobs` and `cpu_priority` stay in the settings file (older files keep
loading) but the Settings view shows them off and disabled with "Not available in this
version: …". Implementing `parallel_jobs` would run plan steps concurrently, which breaks
the ordering the G-03 kept-copy rule and the journal reasoning rely on. `lossless_only`
(OPT-ENG-01, T2) and `preserve_ads` have no control in the UI and remain inactive: lossy
work is already opt-in per step with a confirmation, and new files never carry ADS.

### D-F10 The Windows Recycle Bin setting never overrides a fixed route

The setting applies only when the engine runs with the OS default route on Windows. A
caller (tests, a portable build) that fixed the route keeps it, so no test can reach a real
Recycle Bin.

### D-F11 The CLI purges only before acting

A read-only CLI command (`scan`, `list`, `history`) never deletes anything, even expired
quarantine items; the GUI purges at start-up because opening it is how a user resumes work.
