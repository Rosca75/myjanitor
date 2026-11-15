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
