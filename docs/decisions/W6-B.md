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
