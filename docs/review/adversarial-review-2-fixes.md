# Adversarial review 2: fixes

Fixes for [`adversarial-review-2.md`](adversarial-review-2.md), on branch `ws/w5fix`. Rule
applied throughout: user data safety first (reversible over irreversible, skip over guess,
report over act). Each regression test fails on the reviewed code and passes after the fix
(checked by reverting the fix locally for G-01, G-03, G-09; the others assert the exact
scenario of the review). Decisions are logged in [`../decisions/W5-FIX.md`](../decisions/W5-FIX.md).

## Findings

| ID | Status | Commit | Regression test(s) | What changed |
|---|---|---|---|---|
| G-01 (High) | fixed | `cf08990` | `mj-actions` `tests/review2_fixes.rs`: `g01_protected_entry_below_a_folder_is_seen_through_a_symlinked_root`; `mj-engine` `scan_ops::tests::f01_folder_badge_sees_protected_entries_below` (extended) | `Guard::check_tree` checks the folder and every walked entry in the item's spelling **and** in its canonical spelling (`protected::canonical_spelling`); `ProtectedPaths` keys every existing folder/exact entry (built-in, app, user) in its given and its canonical spelling (links, junctions, mapped/`subst` drives and short names resolve through `canonicalize`, whose `\\?\` and `\\?\UNC\` forms `path_key` already folds, W4-WIN D-W1/F-14). The scan badge translates candidates through the canonical spelling of each scan root (computed once per pass, no per-row I/O). |
| G-02 (High) | fixed | `40596b9` | `review2_fixes.rs`: `g02_cross_volume_move_never_deletes_what_changed_after_the_copy`, `g02_source_changed_before_deletion_is_refused_whole_and_untouched` | The cross-volume copy records each entry's signature (kind, size, mtime, Unix dev/inode/ctime, link target; Windows creation time and attributes); a file that changes while it is read fails the copy. Before the first deletion the whole source is compared with the manifest: any change, new or vanished entry removes our own (verified, just created) copy and refuses the move (`changed`, source complete and untouched). During deletion each entry is removed only while its signature still matches; anything else is left and counted (`CopiedSourceLeft`, journalled `done` with `retained_at` at the complete copy and the "left behind" note, as F-09); a folder is removed only when empty and copied. |
| G-03 (Medium) | fixed | `bcabeed` | `mj-engine` `tests/review2_fixes.rs`: `g03_a_later_step_never_removes_the_copy_an_earlier_step_kept` (both step orders) | `plan_apply` and the dry run collect every kept copy (duplicate survivors and link sources) of every step, with their ancestors, and drop from any step an item that is, or holds, such a copy when its action takes it away (trash, quarantine, delete, move, archive without kept sources, Delete originals); the step carries a warning. |
| G-04 (Medium) | fixed | `557aa42` | `review2_fixes.rs` (engine): `g04_rescan_keeps_the_users_exclusions_and_stale_confirmations_are_refused`; `ui/js/wiring.test.js`, `mock.test.js` | `ConfirmationInfo.selection_digest` (SHA-256 of the step's action and its items with path, kind, scan stamp, subtree stamp, file id); `StepConfirmation.selection_digest` must echo it and `plan_apply` refuses (`confirmation_required`) a missing or stale one. After a rescan, a row the previous results had keeps the selection the user saw (by path; checked only while its stamp is unchanged); only new rows take defaults. CLI, UI dialog, mock and `docs/IPC.md` updated. |
| G-05 (Medium) | fixed | `1182603` | `mj-optimise` `office::powerpoint::tests::g05_layout_removal_is_opt_in_changes_the_deck_and_never_touches_templates`, `defaults_are_lossless_dedup_thumbnail_and_keep_layouts` | "Remove unused slide layouts" off by default; layouts and masters toggles are lossy settings (confirmation, refused in schedules, output reported lossy); both skip templates (main-part content type `*.template*`: `.potx`, `.potm`) with a note. Sibling: Excel "Remove unused cell styles" skips `.xltx`/`.xltm`. Card copy no longer promises layout removal. |
| G-06 (Low) / F-13 | fixed | `980ac62`, `d0dfb3b` | `mj-engine` `instance::tests::g06_a_stale_lock_file_never_blocks_and_the_lock_is_shared_in_process`; `mj-platform` `lockfile::tests::second_open_is_refused_until_the_first_is_closed`; `f13_second_process_neither_recovers_nor_acts` (now holds a real OS lock) | `mj_platform::lockfile`: `flock(LOCK_EX\|LOCK_NB)` via a hand-written `extern "C"` on Unix, share mode 0 on Windows, on a file that is never deleted. The pid in the file is informational only. Engines of one process share the lock through a refcounted registry. |
| G-07 (Low) / F-18 | fixed | `acb9402` | `review2_fixes.rs`: `g07_restore_refuses_a_link_or_folder_found_where_a_moved_file_was`, `g07_a_trashed_link_is_restored_as_a_link` | `verify_retained` refuses unless a file entry finds a regular file (with the recorded hash/size), a folder entry a real folder, and a link entry (new optional journal field `original_link`) a link. |
| G-08 (Low) | fixed | `f65a6b8` | `mj-actions` `review_fixes.rs`: `g08_an_optimiser_conflict_never_leaves_its_output_beside_the_users_file`; `mj-core` `error::g08_tests` | `Error::is_destination_taken(dst)`: only a `create_new`/no-replace refusal naming exactly the temp path leaves it alone; any other failure removes the job's output. Sibling: `framework::optimise_one`. |
| G-09 (Low) | fixed | `f5c8f1c` | `review2_fixes.rs`: `g09_an_archived_file_is_rechecked_before_its_source_goes` | `recheck_tree` re-stats file items against stamp and file id (also used before move/retain of files). |
| G-10 (Low) | fixed | `6578d2e` | `mj-formats` `pdf::tests::g10_hybrid_file_whose_table_and_stream_disagree_is_refused` | Hybrid files: in-use table entries take precedence, the `/XRefStm` stream fills only objects the table lists as free; disagreement on an in-use object refuses the file. |
| G-11 (Low) | fixed | `46e8964` | `office::package` tests (`is_media_name` assertions) | Only `/media/` parts are de-duplicated; `/embeddings/` never. |
| G-12 (Low) | fixed (default kept, now correct) | `6c76619` | `images::tiff_edit::tests::g12_strip_ifd1_never_zeroes_a_value_shared_with_ifd0_or_the_exif_ifd` | `check_clear` protects every other structure: header, IFD0 table+values, Exif IFD (MakerNote included) and Interop IFD, GPS IFD unless removed, IFD1 + thumbnail unless removed; an unreadable sub-IFD refuses. A refused thumbnail strip keeps the segment; a refused GPS strip keeps the existing privacy fallback (orientation-only EXIF). |
| G-13 (Low) | fixed | `24c8892` | `review2_fixes.rs`: `g13_undo_failed_quarantine_items_are_counted_and_purged` | `quarantine::held` includes `undo_failed` entries (still in their own slot). |
| G-14 (Low) | fixed (by refusal) | `ac67744` | `review2_fixes.rs`: `g14_archive_never_removes_sources_it_could_not_store` | A folder whose sources would be retained/deleted is skipped when it holds links (no format stores them) or, for 7z, empty folders; the reason names "keep the originals". Storing links (`-snl`, tar symlink entries) is left for later. |

## Review 1 items reported as partly closed

| ID | Now | Where |
|---|---|---|
| F-01 | closed | G-01 (canonical spelling in `check_tree`, protected set and badge) |
| F-02 | closed | G-03 (kept copies across the whole run) |
| F-09 | closed | G-02 (entry-by-entry verification at deletion time) |
| F-13 | closed | G-06 (OS advisory lock) |
| F-18 | closed | G-07 (kind check) |
| F-19 residual | narrowed | `768d401`: DOS device names refused below the root; Windows destinations with a device name or trailing dot/space refused (pure tests). `\\.\`, `\\?\Volume{GUID}\` spellings (F-14 residual) are resolved by `canonicalize` in the G-01 comparisons. |

## Sibling instances fixed

- `framework::optimise_one` (G-08 pattern): same destination-taken rule.
- Excel unused cell styles on templates (G-05 pattern).
- `recheck_tree` re-stat now also covers file moves and retains (G-09 pattern).
- `scan_ops::badger` file candidates also checked in the canonical root spelling (G-01 pattern).
- `copy_file_verified` (used by every verified copy, reflink fallback included) refuses a file that changed while being read (G-02 pattern).

## Behaviour changes users will notice

- A folder holding something protected under another spelling (symlinked root, mapped drive) is locked.
- A cross-volume move/trash of something being written meanwhile is skipped as `changed` (nothing moved) or reports what it left behind.
- A step that would remove a copy another step keeps skips it with a warning.
- After a rescan, unchecked rows stay unchecked; a lossy/permanent step confirmed before a rescan or a selection change must be confirmed again.
- PowerPoint layout removal is off by default, needs the confirmation, cannot be scheduled and never touches templates.
- A stale `myjanitor.lock` after a crash no longer blocks start-up; the file stays on disk.
- Archiving a folder with links (or, in 7z, empty folders) while removing the originals is skipped.
