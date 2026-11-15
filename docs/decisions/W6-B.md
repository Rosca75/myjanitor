# W6-B: the "You can free up" total

Owner report: "Check if the total under 'You can free up' is correct. I got a strange
number (173 MB) while the opportunities were much more than 173 MB." Real scan on Windows.

Decisions taken without the owner (nobody could be asked), by "a file cleaner people trust
with their real files": a number on screen says what applying frees, and every number that
is not that says so in words.

## Root causes (line numbers at 66375fb)

1. **The cards showed another quantity than the hero** (`ui/js/components/card.js:30`). The
   card's big number was `formatBytes(opp.size_before)`, the size of the files the card acts
   on; "Saves about" was only a tooltip. The hero sums `saving` (what applying frees,
   `crates/engine/src/opportunities.rs:286-293`). For a removal the two are equal; for an
   optimisation, transparent compression or an archive the saving is a fraction of the
   files' size (images and PDFs typically 10 to 30 %, NTFS compression 30 to 50 %), and for
   a light copy it is zero. **This is the cause of "173 MB next to much larger cards"**: on a
   real Windows scan the large cards were the optimise/compress/archive ones, whose big
   number was gigabytes of files while their contribution to the hero was a fraction of it.
   The same quantity was summed by the "Show N smaller opportunities · X" expander
   (`ui/js/views/overview/results.js:35`).
2. **The hero ignored the action** (`opportunities.rs:216, 239`, `Candidate::saving()`): a row
   counted `size - size_after` as estimated by its detector whatever the card's action, so
   a card whose plan step copies, makes a light copy or only lists counted as freeing space,
   and a removal of a file the detector estimated for another action counted the estimate
   instead of the whole file. The plan (`plan.rs:68-90, 151`) special-cased `Copy` only;
   the dry run likewise.
3. **A file listed by several opportunities was counted once per card** (RF-8; detectors
   `crates/detect/src/caches.rs:37` with `office.rs:93-100`, AppCaches' temp rule with
   `temp.rs:63`, dev artefacts inside cache folders, a duplicate that is also a PNG to
   recompress or an old installer). The hero, the plan total and the drive bars summed
   per card with no cross-card de-duplication, and applying a plan holding both cards
   failed the second step on a file the first had removed (reproduced:
   `a_file_two_steps_list_is_counted_and_removed_once`, job `CompletedWithErrors`,
   `not_found`). A kept duplicate removed by another card was also counted as freed.
   This error goes the other way (the hero too large), so it does not explain 173 MB.
4. **The per-drive "after plan" bars were stale and double counted**: `disk.rs:89` used
   each step's rows on their own (same over-count as 3), and the UI fetched them only with
   the list, never on `plan://changed` (`ui/js/events.js:26`).
5. **The mock hid all of this**: its hero was a fixed constant (`ui/js/mock/model.js:182`,
   `HERO_SUMMARY`), unrelated to the cards, and its plan steps used wireframe numbers
   (duplicates step 8.6 GB next to a 12.4 GB card). No UI test could see the mismatch.

Hypotheses checked and **not** causes: `split_by_safety` severity keys are distinct per
class (`results.rs:117`), so buckets always added up to the total; `formatBytes` is the
same function for hero and cards; the summary does not depend on the tab or the sort (now
pinned by `summary_and_counts_do_not_depend_on_the_tab_or_the_sort`); the hero and cards
come from one `opportunities_list` response, so they could not be stale relative to each
other (only the drive bars could, cause 4); `Candidate::saving()` saturates, never wraps.

## D-B1 What the hero means

`summary.total` is **what applying every available card with its current selection and
action frees, every file counted once**. A card in the plan counts with its step's action,
any other card with its default action. It is exactly the sum of the cards' `saving`, and
of its three buckets (each counted row in the bucket of its own safety class). The UI says
it under the number ("What applying the cards below frees with their current selection,
each file counted once.").

Alternative rejected: keep the hero as "potential" and show file sizes on the cards with a
label. The owner reads the hero as "what I gain if I apply what is listed"; a card number
that is not part of that sum invites exactly the comparison that produced the report.

## D-B2 One per-row decision for every surface (`crates/engine/src/savings.rs`)

`savings::tally` decides once, per row, what it frees under a given action; the hero, the
cards, the step review (one card on its own), the plan steps and total, the drive bars and
the dry-run measurement are sums over it. Rules: a removal (trash, quarantine, permanent
delete, move, hard link, reflink, make online-only) frees the whole row; an optimisation,
archive or transparent compression frees its estimate; review only, copy, light copy,
archive keeping its sources and decompress free nothing. Not counted: unselected or locked
rows, online-only rows (not on this disk), groups whose every copy is selected under a
removing action (apply keeps one), a copy another card keeps.

## D-B3 A file in several opportunities: the card that frees most counts it

When several cards select the same file, the one that frees most for it counts it (the
earlier card on a tie, so the result is stable); a row inside a folder another row acts on
is counted once, by whichever frees more (the folder, or everything counted inside it). The
other card keeps the row ticked and reports the bytes as `overlap`, shown on the card in
words ("X more is counted in another card") and summed in the hero sentence. Chosen over
"first card wins" because the larger figure is what applying both actually frees (removing
a file frees its whole size even if another card would only have recompressed it), and over
splitting the bytes because a file is freed once, by one action.

Execution follows the same rule (RF-8): a run drops an item from its step when another step
removes the same path (the first removing step keeps it) or a folder holding it, with a step
warning, so no step fails on a file that is already gone and no file is optimised just
before being removed. The removal of a path frees at least as much as anything else done to
it, so this agrees with the account. The detector-level duplicate that had no reason to
exist is removed too: AppCaches no longer generates the classic Teams `AppData/Roaming`
rules that the Office leftovers card owns.

## D-B4 What the cards show

The card's big number is `saving`, its share of the hero. Under it, in words when it
applies: "from X of files" (an optimisation: the size of what it rewrites), "keeps the
originals: frees no space" (light copy, copy), "X more is counted in another card", "+ X to
review". A card that counts nothing but lists rows for review (review-only cards pre-select
nothing) shows "X to review" instead, muted, and is not part of the hero; the hero adds
"Plus X to review in N cards, not counted." (`summary.to_review`,
`summary.review_opportunities`). "Biggest saving first" orders by the same number the card
shows, and the smaller-opportunities expander sums it.

## D-B5 The plan and the drive bars

The plan applies the same account over its own steps, so with every card in the plan its
total equals the hero, and each step equals its card. With a subset in the plan a step can
be larger than its card (the card loses an overlap to a card that is not in the plan); this
is correct for the plan and is not hidden. Each drive's `plan_saving` is the plan's counted
rows on that volume, so the bars add up to the plan total. `plan://changed` now refetches
the list and the drive bars, since a plan change can change both.

## D-B6 Disk view totals (SCAN-7)

`FileIndex::totals` (the disk usage tree) counted online-only placeholders in `bytes` while
the scan summary and `FolderTree` exclude them, so a OneDrive folder showed its cloud size.
`bytes` now excludes them, `online_only_bytes` reports them; `files` still counts them. The
rest of SCAN-7 (the disk view rebuilds totals per request and `breakdown` builds a
`FolderTree` it does not use) is a performance issue, not arithmetic, and is left open.

## D-B7 The mock follows the engine

The mock computes card numbers from its rows with the same rules (removal, estimate,
nothing), the hero as the sum of the cards, plan steps as their cards, and spreads the
plan total over the drives so the bars add up. It lists no file in two cards, so its
`overlap` is always 0 (the overlap wording is covered by the engine tests and the card
rendering, not the mock). The wireframe totals (48.6 GB hero, 29.3 GB plan) are gone; the
UI tests that pinned them were updated to the derived numbers, deliberately.

# W6-B, second task: review round 1, action core

Fixes of the round-1 findings on the code that removes users' files. Every fix has a test
that failed before it (or, for the pure performance items, a test or measurement named
below). Rule kept throughout: no fix widens what an action may remove; where a choice was
open, the one that keeps more of the user's data won.

## D-B8 A literal path with `[` is a folder, everywhere (RF-2, SCAN-2, SCAN-3)

One classifier, `mj_core::glob::is_pattern_entry`, now serves the protected list
(`ProtectedPaths::new`), the scan exclusions (`ScopeFilter::new`), the global exclusions
(`Exclusions::from_entries`) and `DetectContext::is_protected`. `[` and `]` are legal in
NTFS and ext4 names (`D:\Clients [2024]`), so in an **absolute** entry only `*` or `?` makes
a pattern; a relative entry (`**/Clients/**`, `*.[ch]`) is a pattern when it holds any of
`*`, `?`, `[`, as before. An absolute entry is always keyed as the folder of that exact name
and, when it holds `*` or `?`, is in addition compiled as a glob whose brackets are literal
(`entry_glob` rewrites `[` to the class `[[]`). Treating an entry as both can only protect
(or exclude) more, never less, and a literal key for `/data/*` matches only a folder really
named `*`. Rejected: keeping bracket classes in absolute entries for power users; nobody
writes a class into an absolute protected path, while folder names with brackets are
common (`Photos [raw]`, `Music [FLAC]`). Also from SCAN-3: a trailing separator on an
exclusion (`node_modules/`, the gitignore spelling) is dropped, so it no longer matches
nothing; a bare root (`/`, `C:\`) keeps its separator.

## D-B9 A partly removed archive source keeps its archive (ACT-1)

When removing the sources of a verified archive fails, the archive is deleted only while
`recheck_tree` proves the source untouched (same subtree stamp, or same file stamp and
id). Otherwise the archive is the only complete copy of what is already gone: it is kept,
`created` still names it, and the item fails with a `conflict` that names it. The removal
itself was not changed to an entry-by-entry delete: the user asked for a permanent delete,
and the failure is now harmless.

## D-B10 Undo of a file copy needs its original (ACT-4)

Undoing a copy deletes the copy permanently, so a file copy is removed only while the
original is still a regular file with the same content (SHA-256 of both read at undo
time). Otherwise the undo fails with `conflict` and keeps the copy, which may be the last
one. This is what the folder branch already did and what the module's table promised. The
check is a separate function so crash recovery's "is the copy complete?" test is unchanged.

## D-B11 Recovery judges an interrupted move by both sides (ACT-5)

For a dangling Trash/Quarantine/Move intent whose original and copy both exist, recovery
now compares them (`tree_digest` for folders, SHA-256 for files): a copy that holds every
entry the source still has, and more, means the deletion of the source had started after
the copy verified: the entry is settled `done`, reversible, with an F-09 style note naming
the copy and how many entries remain at the source, so the quarantine lists, purges and
restores it. Equal sides: still `failed`, now worded "a complete copy remains at ...;
nothing was removed". Anything else keeps the "incomplete copy" wording. Recovery still
deletes nothing (D-2E05). The existing recovery test asserted "incomplete copy" for a case
whose copy is in fact complete; it now asserts the new wording (deliberate change).

## D-B12 Read-only outputs (ACT-3)

`fsops::set_mtime` and `fsops::sync_output` work only on MyJanitor's own new files (an
optimiser output, a reflink, an undo's separated copy, an archive). When the read-only
permissions copied from the original refuse a writable handle (Windows read-only attribute;
Unix mode without the owner's write bit for a non-root user), the file is made
owner-writable for the call and its permissions are put back. Chosen over reordering the
three call sites so that any future caller is covered. Verified on Linux by running the
test binary as an unprivileged user (`setpriv --reuid=65534`); as root (this sandbox, most
CI containers) the test cannot fail, the Windows CI job checks it for real.

## D-B13 Link replacement re-checks the kept copy (ACT-11)

Right before the rename over the duplicate, the kept file's size, mtime and file id are
compared with the values read before the byte comparison, and for a hard link the new name
must be that same file. On a mismatch the temporary name is removed and the item is skipped
("the kept copy changed; nothing was replaced").

## D-B14 Zero-day quarantine (ACT-9)

Both layers: the action engine holds a quarantined item at least one day
(`days.max(1)`, the lower bound of the `quarantine_days` setting), and `plan_add_step` /
`plan_update_step` refuse a retention outside 1..=3650 with `invalid_input`. Clamping in
the engine was preferred over classing 0 days as a permanent delete because no screen
offers 0 and a permanent delete has its own action. The CLN-ACT-04 engine test used 0 days
to see the automatic purge; it now back-dates the journal record by two days and checks
the start-up purge (deliberate test change).

## D-B15 Torn journal records (ACT-12)

The journal remembers, per file, that a write failed part-way; the next record written to
that file first writes a newline, so it is not glued to the fragment and lost on replay.
The fragment alone is ignored on replay, as a torn last line already was.

## D-B16 Performance under the results lock (ENG-5, IPC-3, ACT-6, ACT-7)

`after_apply_step` keys the removed paths once (before taking the write lock) into a
`DoneSet` and tests each row's key and its ancestors' keys: O(depth) per row, same rule as
`is_within`. Measured in a debug build: 200 000 rows against 50 000 removed paths in 0.3 s;
the old loop did not finish in 90 s. Siblings fixed in the same files: `build_step` looks
groups up by id, the dry run and `hardlink_not_freed` look skipped items up in a set. The
keep rule (`select::apply_keep_rule`) buckets candidates by group in one pass (50 000
groups of 3: 56 s before, 0.1 s after), and `selection_apply_keep_rule` indexes rows by id
and finds keepers in one pass. That last hunk is in `crates/engine/src/steps.rs`, a file
the other lane also edits; it is confined to `selection_apply_keep_rule`. Not done: the
Name sort of `candidates_page` (same file, not part of this brief).

## D-B17 One read per verified duplicate (ACT-8)

`fsops::compare_files_hashed` returns the SHA-256 and xxh64 of the equal content in the
same pass as the byte comparison; the audit hash and the quarantine manifest reuse it (the
manifest only while the size still agrees; the act-time re-stat has just confirmed the
file). A removed duplicate is read once instead of three times on the quarantine route
(test: `fsops::content_reads_on_this_thread`, a per-thread counter). Not done: the kept copy
is still read once per member of its group; caching its verified hash across a batch would
need an identity check of its own and was left out of a safety-critical path.

## D-B18 File conditions never select a folder (RF-1)

Extension, type group, size, zero-byte, the three dates, header mismatch, content and media
describe a regular file's own bytes and times. On a folder, link or device they are now
*not applicable* (`Unknown`): never a match, and their negation is no match either, since
`not(Unknown)` stays `Unknown`. A folder's own size is 0 and its own mtime only moves when
a direct child is added or removed, so `size:<1MB`, `older:90d` and `-ext:mp4` used to
select every folder, and `search_run` turned each into a whole-subtree row pre-selected for
Trash (the "Downloads older than 90 days" preset took a year-old folder holding last
week's tax return). Name, path, kind, depth, path length, attributes, owner and invalid
name still apply to folders, so `kind:dir name:=node_modules` (Dev junk) is unchanged.
Defence in depth: `search_run` pre-selects a folder row only when the rule holds a
non-negated `kind:dir` (`mj_filters::asks_for_folders`); a folder matched on its name,
path length or attributes alone is listed unselected. The Downloads and hidden-system
presets gained `kind:file`. Rejected: ageing a folder by the newest file in its subtree
(`totals().newest_modified`); it would need index access inside the evaluator and is a
feature, not a fix. `testutil::set_mtime` now also sets a folder's mtime (used by the
regression test).

## D-B19 The fast index keeps an entry only below an entered folder (SCAN-1)

`walker::fast::try_fast` no longer assumes a subtree directly follows its folder in key
order (`skip.zip` and `skip - Copy` sort between `skip` and `skip/x`). It keeps a set of
the folders it admitted and entered (seeded with the root) and considers an entry only when
its parent is in that set; sorting still guarantees a parent comes before its children.
The directory walk and the fast path now share one rule, "only an entered folder's
children are seen", so excluded, hidden, other-volume and link-target content is dropped
the same way on both. Chosen over a component-wise sort because it does not depend on any
ordering property beyond "parent first". `Admitted::Dropped` lost its `subtree` flag,
which nothing reads any more.

## D-B20 A typed path wildcard with separators matches part of the path (RF-3)

Indexed paths are absolute and `*` does not cross a separator, so `path:*/Photos/*` (the
documented example) matched only a `Photos` folder at a filesystem root, and
`-path:*/Photos/*` excluded nothing while the rows were pre-selected. The quick-query
parser (`query::values::anchor_path_wildcard`) now rewrites a typed path wildcard that
holds a separator: `\` becomes `/`, a lone `*` as first or last segment becomes `**`, and
a pattern not anchored at a root (`/`, `**`, `X:`) gets a leading `**/`. So
`*/Photos/*` = `**/Photos/**`, `Photos/*.png` = `**/Photos/*.png`, `/home/*/Photos/*` stays
anchored. Patterns written in the rule builder (`Condition::Path` directly) are taken
literally; they were never documented as relative. The two parser tests that pinned the old
shape now pin the new one.

## D-B21 Broken files: a suspicion is checked before it is reported (RF-4)

JPEG: a missing `FF D9` in the last 4 KiB no longer means "cut short" by itself. A motion
photo (`PXL_…MP.jpg`, Samsung) is a complete JPEG followed by a video, so the check then
walks the marker segments from SOI by their lengths (an EXIF thumbnail's own EOI is
skipped) and streams the entropy-coded data in 64 KiB chunks until it meets `FF D9`; the
file is broken only when it ends first, inside a segment or inside the image data. Only
suspicious files pay for the read, and it stops at the image's end. The cheap tail check
still accepts a file whose last 4 KiB holds `FF D9` (a JPEG cut inside its thumbnail can
pass, as before: missing a damaged file is the safe direction for a review-only card).
Audio now matches the in-house prober: MP3 no longer needs a frame sync at the exact byte
after the ID3 tag (the prober searches for the first frame, as players do; "ends inside
its ID3 tag" stays), FLAC accepts an ID3 tag in front of `fLaC`, and WAV accepts `RF64`
without judging its 32-bit size field.

## D-B22 A non-Unicode name keeps its folder's key as prefix (SCAN-6, RF-5)

`path_key` used the wire codec, which escapes a non-Unicode path as a whole behind a NUL,
so `/r/a/bad\xff.txt` keyed as `\0/r/a/bad%FF.txt`: outside its own folder for
`is_within`, `FileIndex::subtree`, `totals`, `tree_stamp`, the protected-path check and
the action guard, and first in the index order that `FolderTree::build` relies on
(ancestors above the direct parent lost its bytes). The key is now built from the path's
encoded bytes: valid UTF-8 runs as they are, each undecodable byte as NUL plus two hex
digits. NUL cannot occur in a real name, so keys stay distinct; a valid path's key is
unchanged; a child's key always extends its folder's key with `/`. The wire codec
(`encode_path`) is unchanged. Scan-cache records whose key starts with NUL (the old form)
are dropped at load rather than bumping the format version, which would have discarded
every user's fingerprints; those few files are hashed again once. Sibling fixed:
`detector::env::decode` dropped the NUL that starts an escaped path in a NUL-joined list;
an empty piece now marks the next entry as escaped (no entry is ever empty), so the list
format and the engine side stay as they are. Not added: an engine-level test that acts on a
non-Unicode temp file; the guard's `root_of` is `is_within`, which the core test covers.

## D-B23 A drive that goes away mid-scan is not a finished scan (SCAN-4)

A listing or stat that fails with "not found" was skipped silently (right for one temp
file deleted during the scan), so an unplugged USB disk or a dropped share produced a
"done" index with a fraction of the files, `unreadable = 0`, and a cache prune that threw
away the fingerprints of everything not reached. Now a folder that vanished between being
listed and being read becomes a hole (the cache keeps the records below it), and whenever
anything vanished, `walker::run` stats every walked root again after the walk: a root that
is gone now is reported unreadable (its state becomes `Unreadable`, so the cache does not
prune under it), and the scan fails with that error when no walked root remains. A folder
really deleted is pruned at the next scan, when it is no longer listed at all. Chosen over
counting every vanished entry as unreadable, which would make ordinary temp-file churn
show up as errors.

## D-B24 Settings > Scanning are the session scope's options (SCAN-5)

The app scans the session scope, whose `options` were `ScanOptions::default()` forever:
the four Scanning switches (fast index, scan cache, hidden files, one drive per location)
changed `settings.scan` and nothing read it except the CLI. The engine now owns the link:
the scope's options are copied from `settings.scan` at start-up (and for a launch-path
scope) and again whenever `settings_set` changes that section. An explicit `scope_set`
(CLI `--no-cache`, a schedule's scope) still replaces them for its scan, until the next
change of the setting. With the cache off, detectors get no fingerprint store either, so
"off" means hashes are neither reused nor stored. The `threads` fallback in `run_scan` is
unchanged.

## D-B25 A nested location on another drive is a location (SCAN-8)

`effective_roots` dropped any root lexically inside another, and the outer walk then
stopped at the other drive's mount point (`one_file_system`, default on): a second disk
mounted at `/home/me/Data` and added explicitly was never scanned. `mj_scan::effective_roots_on`
keeps such a root when `one_file_system` is on and its volume differs from its nearest
containing root's; the engine's scope normalisation and `walker::run` use it. The outer
walk still drops the mount point, so nothing is indexed twice; with `one_file_system` off
the root collapses as before and the outer walk enters it. A root whose volume cannot be
read collapses (the old behaviour). Also: a directory skipped as another filesystem is now
a hole for the cache, so scanning only the outer location no longer prunes the
fingerprints that a scan of the mount recorded. On Windows a folder-mounted volume is a
reparse point that `scope_add_root` refuses, so this is a Linux/macOS case.

## D-B26 A temp folder is as old as the newest thing in it (RF-7)

The minimum-age gate of `aged` rules (`**/AppData/Local/Temp/*`, `**/Windows/Temp/*`,
`/tmp/*`, `/var/tmp/*`) read the matched entry's own mtime. For a folder that only moves
when a direct child is added or removed, so a three-day-old build folder with a file
written ten minutes ago passed the 24 h gate and was pre-selected whole. A matched folder
now passes only when it and every entry below it (files and subfolders) are older than the
cutoff; an unknown time counts as recent, as before. The subtree is read only for aged
rules that matched a folder.

## D-B27 A link to an offline drive is not broken (RF-6)

Broken links are pre-selected, so the "maybe offline" guard decides between a real
dangling link and a junction to an unplugged drive. It recognised only Unix mount parents
and only checked that the target's parent existed. `links::target_may_be_offline` (pure,
with the existence checks injected so the Windows rules run in the Linux tests) now skips:
any network target (`\\server\share`, `\\?\UNC\…`, `//server/share`); a drive-letter
target (also behind `\\?\` or the NT `\??\` prefix of junctions) whose drive root is
absent; and a target below `/mnt`, `/media/<user>`, `/run/media/<user>`, `/Volumes`,
`/net` or `/misc` unless its mount directory exists and is not empty (an empty folder there
is usually an unmounted mount point). A live share with a really missing target is now
skipped too: under-reporting a broken link is the safe direction for a pre-selected row.
Not verified: the Windows error mapping and `read_link` spelling of a junction to a missing
drive (CI's Windows runner has no second drive to unplug).

## D-B28 The system language comes from the OS (RF-9)

With "Languages to keep" empty, the detector kept the language named by `LC_ALL`,
`LC_MESSAGES` or `LANG`, none of which a Windows program started from Explorer inherits:
a French Windows saw its own `fr.pak` files listed as unused. `mj_platform::system_language`
asks the OS (Windows: one kernel32 call, `GetUserDefaultLocaleName`, declared in-house like
the other FFI; elsewhere the environment, `C`/`POSIX` meaning none), the engine passes it
to detectors as `env.system_language`, and `language::kept_languages_with` uses it when the
setting is empty. When neither is known, only English is kept and the card carries a note
saying so instead of claiming the system language silently. Rejected: sending
`navigator.language` from the UI into the setting; it would rewrite a user setting and
the UI files belong to the other lane. Not verified on Windows here (the Windows clippy
target compiles the call).

## D-B29 Content search rejects a binary file from its sample (RF-10, part)

`content::search_file` read up to `max_bytes` (8 MiB) of every file and only then looked at
the first 8 KiB to decide it was binary. It now reads the 8 KiB sample first, returns
`Binary` when the sample says so (the same `decode_text` decision as before, which only ever
looked at that sample), and reads on to the limit only for text. A `content:` search over
a photo and video library costs 8 KiB per file instead of 8 MiB. Not done: making
`search_run` a cancellable job with progress. It is a synchronous IPC command today; a job
needs a contract change in `docs/IPC.md` and `ui/js/api.js` plus the search view, which
belong to the other lane in this wave. Left as an open item.

## D-B30 Only what its program recreates is "rebuildable" (RF-11)

`rules::build` badged every rule-table hit `rebuildable` ("Can be recreated by the tool
that made it"), including Office's unsaved files and AutoRecover copies, Outlook `.ost`,
`Windows.old`, crash dumps, rotated logs and temp files, and the whole AppCaches card's
class made every folder row own the repositories inside it (F-01 exemption). `RuleDef`
and `Hit` now carry `rebuildable` (default true; `.not_rebuildable()` turns it off). Off:
the Office unsaved/AutoRecover and offline-mail rules, `Windows.old`, every dump and
error-report rule, rotated and old logs, the four temp-folder rules, and every winapp2 hit
(a user's own rule file says nothing about rebuilding). Such a row has no badge and
`columns.safety = "reversible"`, so `candidate_safety` is `Reversible`, the apply does not
mark it an owned tree, and the scan's badger (now per candidate, not per card) badges a
folder holding a repository `protected`, as on any other card. Caches proper (browser,
shader, package, thumbnails, Office file cache, Teams) keep the badge and the exemption.

## D-B31 winapp2 exclusions fail closed (RF-12)

An `ExcludeKey` whose variable is unknown used to disappear (`filter_map`), and
`whole_folder` was decided from the compiled list, so a `RECURSE|REMOVESELF` key proposed
the entire folder, pre-selected, as if nothing had been excluded; an exclusion folder with
`*` never matched. Now: an entry with any unresolvable exclusion is skipped whole
(`winapp2_unknown_variable`); `whole_folder` needs the parsed exclusion list to be empty;
an exclusion folder with `*` or `?` is a path glob (`PATH`: that folder or anything below
it; `FILE`: files in a matching folder). CCleaner's own variables (`%CommonAppData%`,
`%LocalLowAppData%`, `%Documents%`, `%Pictures%`, `%Music%`, `%Video%`) are derived from
`ProgramData` and `USERPROFILE` when not set, so real rule files resolve instead of being
skipped. Wildcards in `ExcludeKey` folders are taken as valid syntax from the format as
commonly written; no published specification was checked.

## D-B32 The MFT pass is for whole drives only (SCAN-9, part)

`fast_enumerate` reads every MFT record of the volume whatever the root, then keeps what
lies below it, with no progress until the end; a scope of Downloads, Documents, Desktop and
Pictures paid four whole-volume passes. It now answers `Unsupported` (so the scan walks)
unless the root is a whole drive (`C:\`, `\\?\C:\`, `mft::is_volume_root`): a folder is
listed faster by the parallel walk, which also reports progress. Not done: sharing one
enumeration between several drive roots on one volume (only one root can be a given
drive's root, so it does not arise), and progress during the enumeration of a whole drive
(needs the record loop in `imp::enumerate` to report, which cannot be run here). Not
verified on Windows: the Windows clippy target compiles the change; CI's elevated runner
used the fast path on a temp-folder root, which now walks.

## D-B33 An external image tool's output must hold everything the in-house result holds (OPT-1)

libjpeg (jpegtran) stops at the EOI and oxipng does not copy data after IEND, so a motion
photo's video, a Samsung trailer or any appended file was missing from the tool's output,
which was then accepted because it was smaller and showed the same picture. Rule now: the
in-house result (which applied the user's policy exactly) is the reference, and a tool's
output replaces it only when it can be shown to hold all of it.
- JPEG: `mj_formats::jpeg::image_end` finds the EOI that ends the first image (marker
  segments skipped by length; in entropy data an `FF` is stuffing, a restart marker, fill or
  the next marker, so the first marker EOI after a scan ends the image). jpegtran's output
  must be exactly one complete image with the same frame and kept segments; the bytes after
  the in-house result's image are re-appended byte for byte. Re-appending rather than
  skipping keeps the Huffman gain on motion photos; their video is located from the end of
  the file (XMP `MicroVideoOffset`), which the in-house rewrite already moves the same way,
  and MPF files (absolute offsets) stay refused.
- PNG: oxipng's output gets the in-house result's trailer re-appended and must pass
  `png::same_metadata`: every chunk except the pixels and the chunks a colour reduction
  rewrites (`IHDR PLTE IDAT IEND tRNS`, verified by decoding; `bKGD sBIT hIST` by presence)
  equal, `iCCP` and `zTXt` compared inflated because oxipng re-deflates them, no extra chunk,
  same trailer. This also stops `--strip safe` dropping a chunk the in-house policy keeps.
- The in-house PNG pass kept the trailer only under "Keep everything"; it now keeps a
  non-zero trailer under every policy (data after IEND is not metadata the user asked to
  remove) and drops zero padding only when stripping (`trailing_bytes_follow_the_policy`
  changed accordingly).
- Photo conversion (WebP, AVIF, JPEG XL) cannot carry such data: a JPEG or PNG followed by
  anything other than zero padding is skipped with a reason, also for the lossy presets
  (the user asked for a smaller picture, not for its video to go).

## D-B34 The "Reversible" JPEG XL preset routes by content (OPT-3)

The source is sniffed before encoding: JPEG content takes `--lossless_jpeg=1` (bit-exact
transcode), PNG content takes `-d 0`, anything else is skipped ("not a JPEG or PNG picture,
whatever its name says"). The estimate still uses the extension (a cheap model, no safety
claim). The JPEG must have a findable end of image, otherwise it is skipped.

## D-B35 A JAR keeps its stored entries stored (OPT-2)

Spring Boot refuses a nested jar that is not stored, and nothing in a JAR says which stored
entries a loader depends on, so in a Java archive (JAR content, `.jar/.war/.ear`, or a
`META-INF/MANIFEST.MF`) every stored entry is written stored and only deflated entries are
recompressed (`RepackPlan::keep_stored`). EPUB, ODF, CBZ, Office and plain `.zip` still
convert stored to deflate where it helps: their formats allow both methods. The estimate
models the same rule. Not done: a `.zip` that is a renamed JAR goes through archive
recompression, which does not apply the rule.

## D-B36 Content routing compares entry names as bytes and is guarded (OPT-6)

`refine_zip` sliced names at byte offsets and panicked on a multi-byte character; it now
compares bytes. `analyse_by_content` also runs the routing under `guarded`, so a reader
panic on one hostile file counts that file unreadable instead of failing the whole card.

## D-B37 Media lengths and tracks come from ffmpeg, and the checks fail closed (OPT-4, OPT-5)

MyJanitor's media prober reads WAV, AIFF, FLAC, Ogg, MP3 and MP4/MOV only; for Matroska,
WebM, AVI and ASF the length check was skipped. `ffmpeg -i` (same local-only input
arguments as an encode) now reports the length and every stream of the source and of the
output (`InputReport`). Length: an unknown source length skips the file before encoding; an
unknown or different output length is an error and nothing is changed (`check_length`; video,
audio, and the screen recordings' voice). Tracks (`StreamPlan`): the output must hold every
track of the source minus what the user asked to drop (extra audio, subtitles). Matroska
attachments (subtitle fonts) are mapped and copied into Matroska outputs; a timecode track is
expected back (the muxer writes it again from the timecode it reads) and checked; data tracks
other than timecode (GoPro GPS, iPhone camera metadata), cover pictures, further video tracks
and attachments going to MP4 skip the file before encoding, because this ffmpeg mapping cannot
carry them. After the encode the output's report is compared with the plan; a missing track
removes the output and skips the file. Consequence: iPhone videos with `mebx` tracks and GoPro
videos are now skipped by the video card instead of losing those tracks; carrying data
tracks over (`-map 0:d -copy_unknown` into MOV, per-tag) can be added once verified per
camera. The remux setting's help now says what it does ("Keeps every track as is").
The audio card's own cover-art handling was not part of this change.

## D-B38 Comments go only with "Remove all metadata" (OPT-7)

JPEG COM segments (captions, scanning notes) were removed by the default thumbnail pass and
by "Remove GPS position only", without a word in the labels. They are now removed only by
"Remove all metadata", which says so; the labels of the other policies are true again.

# W6-B, fourth task: review round 2, duplicate detection

Findings DUP-1 and DUP-3 to DUP-8 (DUP-2 is the other lane's). Rule applied wherever a fix
was a judgement call: a false duplicate is the worst outcome, so report fewer matches and
pre-select fewer.

## D-B39 A thumbnail is a hint, never a verdict (DUP-1)

A JPEG was fingerprinted from its EXIF thumbnail whenever one decoded, and nothing compared
it with the main picture. Editors that copy EXIF verbatim (lossless crop or rotation,
retouching) leave the old thumbnail behind, so a retouched or cropped picture was "100 %
similar" to its original and pre-selected; the test `embedded_thumbnail_is_used_before_a_full_decode`
pinned that as intended. Now:

- the thumbnail is read only inside the APP1 segment (it was read from anywhere after it),
  only up to 1024x1024 pixels (refused from its header, before any pixel), and only when
  its shape matches the main frame within 3 % (orientation-aware: a lossless rotation, a
  crop or a padded thumbnail fails, and the image is decoded in full instead);
- a thumbnail fingerprint never decides a match: after grouping, every thumbnail-sourced
  image that is part of a group is decoded in full (gated like any decode), its hashes
  replace the thumbnail's (cached with the new `MAIN_DECODED` flag, since the cache merge
  keeps `FROM_THUMBNAIL`), and the grouping is redone until no unchecked thumbnail is in a
  group. This also corrects fingerprints cached by earlier versions, so no cache version
  bump was needed. The check runs whatever `decode_full` says: that setting is about images
  without a thumbnail, and checking a proposed match costs one decode per candidate only;
- an image whose main picture cannot be decoded (too large, corrupt) keeps its thumbnail
  fingerprint, and its group is shown with nothing pre-selected.

The fast path stays for the common case (an image that matches nothing is never decoded).
The old test now asserts the opposite and three tests were added (stale thumbnail never
groups in either `decode_full` mode, a good thumbnail still groups and is pre-selected,
a thumbnail of another shape is not used). Not covered by a test: a thumbnail whose IFD
offset points past the APP1 segment (building one needs a hand-made EXIF layout).

## D-B40 A hard-linked copy frees nothing and is kept in preference (DUP-4)

The dupes lanes keep one path per file id, so any member with `links > 1` has its other
names outside the group: removing it frees nothing. The row's size is now 0 (like an
online-only row, `Candidate.size` is reclaimable bytes), so the savings account
(`crates/engine/src/savings.rs`), which counts a removed row's whole size, agrees with the
dry run's `hardlink_not_freed` without any change of its own. Such a row is never
pre-selected and says why ("Shares its data with another file name: removing it frees no
space"). In a group of identical files (exact, attachment copies, fonts) a hard-linked member
is kept in preference, the keep rule choosing among such members
(`KeepRule::pick_same_content`), because keeping it lets removing the plain copies free
their space; the exception is a preferred-folder rule whose choice lies in that folder (an
explicit wish wins; the hard-linked name it leaves is listed, not pre-selected). Similar
images and version chains are different files, so their keeper is the rule's alone. The
keep rule re-applied from the UI (`mj_actions::select::apply_keep_rule`) follows the same
two rules. Hard-linked rows of the other detectors (`crates/detect/src/common.rs`) still
count their whole size; that is outside this task (open item).

## D-B41 A zero dHash is unusable in every mode (DUP-3)

The pHash of a flat image is rounding noise that is identical for different grey levels, so
pHash mode grouped a black, a grey and a white picture at distance 0 and pre-selected them.
A fingerprint with a zero dHash (no cell darker than its left neighbour: flat images first
of all) is now unusable in every mode and counted as `flat_image`. Checking the dHash rather
than changing `phash()` also covers hashes already cached. Cost: an image whose rows only
ever brighten left to right is no longer matched in pHash mode either (it already was not in
dHash and "both" modes). Two unit assertions that pinned the old pHash behaviour were
changed.

## D-B42 Decodes stay within a memory bound (DUP-5)

The PNG decoder inflated the whole raw image (8 bytes a pixel for RGBA16, 128 MB at 16 MP,
from a file that compresses ~1000:1) on every I/O worker. It now inflates the IDAT chunks in
place, one scanline at a time, unfilters against the previous line and feeds the grid
accumulator: memory is O(width) (a counting-allocator test decodes 2000x2000 RGBA16 under
4 MB). Files over 16 MiB are read and decoded under the two-at-a-time gate, and so is a JPEG
whose frame header lies outside the 128 KiB window (its size is unknown). Whole-file reads of
JPEGs and PNGs up to 16 MiB still run on every worker (bounded by the 64 MiB read limit and
the worker count); that is the accepted remaining cost.

## D-B43 One read per file and stage across detectors (DUP-7)

The engine runs the content detectors together and each runs the exact lane over its files,
reading the cache only at the start of a stage. Partial and full reads are now single-flight
within the process (`staged/flight.rs`): the first reader stores its result in the
fingerprint cache before it lets the others go, and they take it from there. A failed read
is redone by each waiter (each reports its own error), never shared. This was chosen over
reordering the engine (running exact duplicates alone first), which is in the other lane's
files and would serialise the scan. Not done: a cap on the total number of I/O threads across
detectors running together (open item).

## D-B44 Huge near-identical sets (DUP-8)

Leader refinement partitions the remaining set in one pass instead of `Vec::contains` per
item; a cluster over 2048 members reports the bound 2t as its diameter (every member is
within t of the leader), which can only lower the confidence and so never adds a
pre-selection; the cancel token is checked for every item searched and every leader. The
BK-tree candidate search itself stays O(n x neighbours) for one huge dense set; it is now
cancellable.

## D-B45 Permanent read failures are neutral for the serial fallback (DUP-6)

The adaptive reader counted every failure towards "concurrent reads fail on this
filesystem", so a run of permission-denied files (a non-admin scan of `C:\`) dropped the rest
of the stage to one thread. Only retryable outcomes count now; denied, vanished, changed
and panicked items are neutral (neither done nor failed for the verdict).


## D-B46 CI builds with --locked; no long benchmark in CI (XC-5)

Every cargo invocation in `ci.yml` that builds (clippy, test, `cargo build -p mj-cli`, and
`cargo tauri build ... -- --locked`) now passes `--locked`, so a dependency edit that forgets
`Cargo.lock` fails the build instead of being re-resolved silently on the runner (`cargo fmt`
and `cargo install --locked` needed nothing). `cargo metadata --locked` passes at this commit.
The ignored large-tree benchmarks stay out of CI on purpose: they take minutes on shared
runners and wall-time budgets there would be flaky; they remain a manual pre-release check.

## D-B47 System language (XC-1): already fixed by RF-9

`c2d7505` (RF-9) made the engine supply the OS language (`crates/platform/src/locale.rs`,
`SYSTEM_LANGUAGE` env param) and the detector add a diagnostics note when the language is
unknown. Verified by reading `language.rs` and its tests; no change.

## D-B48 Stale status claims corrected (XC-6)

PLAN.md: W4-3 CI is "done (CI run green, see STATUS.md)", W4-4 Docs "done". FEATURE-STATUS
CLN-ENG-05 now says 19 keys and lists them, matching `mj_filters::query::KEYS` (the
quick-search autocomplete test pins the same count). Not changed: the "Owner: W3-D" source
headers (they sit in files lane-a owns and are history, not a claim about behaviour) and
`bundle.targets` in tauri.conf.json (src-tauri is lane-a's; CI always passes `--bundles`
explicitly and the README documents the Windows installer command).
