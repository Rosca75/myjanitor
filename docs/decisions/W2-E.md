# W2-E decisions: action engine and safety layer

Rule applied throughout: user data safety first (reversible over irreversible,
skip over guess, report over act). Ids are `D-2Exx` (ARCHITECTURE.md §11).

## D-2E01 Windows "Recycle Bin" goes to the quarantine by default
- **Context:** `SHFileOperationW(FO_DELETE, FOF_ALLOWUNDO)` returns no handle to the
  recycled item; the only way back is to find its `$I`/`$R` pair in
  `<drive>\$Recycle.Bin\<SID>\`, an undocumented format.
- **Choice:** `mj_actions::TrashRoute`. On Windows the default route is `Quarantine`:
  `Action::Trash` and `OriginalsPolicy::RecycleBin` move the item into the app
  quarantine, journalled as `quarantine`, exactly restorable from History and
  auto-purged after `quarantine_days`. `TrashRoute::Platform` (the real Recycle Bin)
  stays available through `ExecOptions` and is implemented in
  `mj_platform::action::trash`; its restore is **best effort**: match the original
  path (case-insensitive) and a deletion time within 10 minutes in the `$I` records
  (v1 and v2 layouts), refuse when the original is occupied, rename `$R` back,
  return `not_found` otherwise. Linux keeps the real freedesktop Trash (exact restore).
- **Also:** `FOF_WANTNUKEWARNING` is added to the brief's flags (without it the shell
  silently deletes items too large for the bin), and only `DRIVE_FIXED` volumes are
  accepted (network / removable drives have no bin, the shell would delete).
- **UI impact (W3-D/W2-F):** on Windows label the default "Keep in MyJanitor's
  quarantine (restorable)" or explain that "Recycle Bin" items are held in quarantine.

## D-2E02 SAF-11 is enforced inside `mj-actions`
- `execute(ctx, items)` treats every item as unconfirmed: permanent delete,
  `originals: delete` and lossy optimiser output are skipped with
  `confirmation_required`. `execute_with(ctx, items, &ExecOptions { confirmed: true, .. })`
  is what the facade calls after `StepConfirmation`. `dry_run` assumes confirmation
  (it estimates what a confirmed run would do).

## D-2E03 Confidential files are skipped for every action except copy
- Signed / labelled / encrypted files (content inspection, plus the encrypted file
  attribute) are skipped per `Settings::safety::skip_*` for every action that removes
  or changes the item; `copy` is allowed. The skip uses error code `protected`
  (no dedicated code exists, see contract requests). Folder targets are not
  content-inspected (only files are).

## D-2E04 Journal durability
- `sync_data` after **every** record, not only intents; the in-memory view is updated
  only after the line is durable; a new journal file is followed by a directory sync.
- A torn last line is ignored on replay and terminated with `\n` on open; it is never
  deleted (audit). Records never go to a month file older than the newest one, so
  "later line wins" holds when the clock goes back.
- Entry ids are reserved per run (`Journal::reserve_entry_ids`) so several jobs of one
  run never collide (`EntryId = run * 1_000_000 + index`).

## D-2E05 Recovery inspects and records; it never moves or deletes user data
- Dangling intents are settled `done` / `failed` by looking at both locations, with a
  note naming any leftover (e.g. an incomplete cross-volume copy is left in place and
  named). The only removals are MyJanitor's own temporary names beside an original
  that is verifiably intact (a hard link to the keeper, an optimiser temp output).
- A trash intent whose original is gone settles as `done`, *not reversible*, with
  "restore it from the Trash" (the ticket is only known after the move).

## D-2E06 Undo runs carry their own entries
- Each undone entry gets an entry in the undo run (`intent` before acting, then
  `done`/`failed`); the source entry gets `undone` / `undo_failed`. `undo_failed`
  entries can be retried. `undo` creates the undo `RunRecord` if missing and refuses a
  run id of another kind (recovery recognises undo entries by their run's kind and
  finds the source by original path + action + retained locations).
- The restore target must pass the guard's lexical checks (inside the allowed roots,
  not protected) and no existing ancestor may be a link: the facade must pass allowed
  roots that cover the originals (e.g. the original run's scope roots).

## D-2E07 Link replacement
- Content is verified by a full byte comparison right before acting (SHA-256 of the
  duplicate computed in the same pass, for the audit), the link is created under
  `.<name>.mj-<entry>.tmp` beside the duplicate, the duplicate is re-stat'ed, then the
  temp name is renamed over it. Undo copies the current content to a temp name,
  checks its SHA-256 equals the journalled one, restores the duplicate's mtime and
  renames it back. A hard-linked duplicate's own permissions cannot be restored (the
  journal has no field); a reflink gets the duplicate's permissions and mtime at once.

## D-2E08 Mount points below a root are refused
- An ancestor between the root and the item on another device (`st_dev`) is treated
  like a junction (`link_not_followed`), per ARCHITECTURE §7.2 rule 3. This also
  refuses Btrfs subvolumes below a scan root (conservative; revisit if users ask).

## D-2E09 Other choices
- `rename_no_replace` (renameat2 `RENAME_NOREPLACE` / `MoveFileExW` without
  `REPLACE_EXISTING`) is the only rename used for user paths, except the deliberate
  replacement of a verified duplicate by its link and of an original by a
  confirmed `originals: delete` output. Without kernel support files use link +
  unlink; folders check-then-rename (a narrow race, documented).
- Cross-volume moves: copy + verify (size and xxh64 re-read from disk) + fsync +
  delete; the source is deleted only after verification. Links inside folders are
  recreated as links (Unix) or the copy is refused (Windows).
- Quarantine manifest at `<q>/<run>/<entry>.manifest.json` (original, size, mtime,
  xxh64, SHA-256). Purge removes only paths lexically inside the quarantine with no
  link on the way, using the caller's `now`; failures are retried by the next purge.
- Audit SHA-256 is computed for files up to 1 GiB (`ExecOptions::audit_hash_limit`).
- `Unsupported` from the platform (dehydrate on Linux, reflink on ext4, no trash on a
  volume) is recorded as *skipped*, never as done.
- `fs_decompress` is not reversible (the previous algorithm has no journal field).
- freedesktop `directorysizes` is not maintained (optional in the spec).
- Dehydrate (Windows): verify a Cloud Files reparse tag, set `FILE_ATTRIBUTE_UNPINNED`
  (clearing `PINNED`), then `CfDehydratePlaceholder` best effort. Hydrate clears
  `UNPINNED` and calls `CfHydratePlaceholder`.
- Sync roots: OneDrive environment variables and registry (`Accounts\*\UserFolder`,
  `SyncEngines\Providers\OneDrive\*\MountPoint` for SharePoint), Dropbox `info.json`,
  Nextcloud `nextcloud.cfg`, the Linux `onedrive` client config, well-known folder
  names and marker files under home. Unknown home ⇒ error ⇒ "unknown". Providers
  outside the list and renamed folders without markers are not found (residual risk);
  callers can add roots (`SafetyInspector::add_sync_roots`).
- PDF signature/encryption scan: full file up to 64 MiB, head + tail 4 MiB beyond;
  signature dictionaries inside compressed object streams are not seen.
- SAF-05: nothing elevates; `safety::needs_elevation` flags permission-denied protected
  paths on Windows, `report::permission_denied_paths` lists them.
