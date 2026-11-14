# Adversarial review 1: fixes

Fixes for every finding of [`adversarial-review-1.md`](adversarial-review-1.md), on branch
`ws/w4fix`. Rule applied throughout: user data safety first (reversible over irreversible,
skip over guess, report over act). Each regression test fails on the reviewed code and
passes after the fix (checked by reverting the fix locally before committing).

## Findings

| ID | Status | Commit | Regression test(s) | What changed |
|---|---|---|---|---|
| F-01 | fixed | see `git log --grep F-01` | `mj-actions` `tests/review_fixes.rs`: `f01_*` (6 tests, every folder action); `mj-engine` `scan_ops::tests::f01_folder_badge_sees_protected_entries_below` | `Guard::check_tree` walks every folder item (no links followed) before any action except Copy and refuses (`protected`) when a protected folder/exact entry/pattern or one of MyJanitor's folders lies below it (also lexically, for folders that do not exist yet), and (`link_not_followed`) when another volume is mounted inside it; an unreadable subtree refuses too. VCS (`.git`, `.hg`, `.svn`, `.bzr`) and credential stores (`.ssh`, `.gnupg`, `.aws`, `.azure`, `.kube`, `.password-store`, `.docker`) are built-in patterns, the folder itself included. Only a regenerable artefact folder (`ActionItem::owned_tree`, set by the engine for `Rebuildable` folder rows: dev artefacts, caches) may take VCS metadata with it; credential stores never. Folder rows holding protected entries are badged `protected` at scan time (index-based, no I/O), so they are locked in the table. |
