# Decisions

One entry per non-trivial decision: *context → options → choice → why*.

## D-001 Working branch
- **Context:** the brief suggests `build/initial-app`; the session environment only permits pushing to `claude/eloquent-shannon-4pcqld`.
- **Options:** (a) `build/initial-app`, (b) the session branch.
- **Choice:** (b).
- **Why:** pushing elsewhere is not permitted in this session; the brief's name was an example.

## D-002 Whole-page wireframe preview
- **Context:** the brief expects `docs/design/Option B.png`; only the individual frames and the `.figcraft` source were supplied.
- **Choice:** work from `docs/design/option-b-frames/*.png` and the parsed `.figcraft` (the brief's stated fallback).

## D-003 Subagent isolation
- **Context:** the brief asks for one git worktree per code-writing subagent.
- **Choice:** worktrees are created by the orchestrator under `/home/user/myjanitor-wt/<workstream>` on `ws/<workstream>` branches and merged one at a time into the working branch.
- **Why:** the session's primary directory is not itself a git repository, so automatic worktree isolation is unavailable; manual worktrees give the same isolation.

## D-004 Light theme only
- **Context:** the brief asks for light and dark themes if the palette has both.
- **Choice:** ship light only.
- **Why:** the doc-anonymiser palette defines a single light `:root` and no dark variant (see `docs/research/palette.md`).

## D-005 Partial hash windows
- **Context:** dedup-photos partial-hashes only the first 64 KB; general files (video, disk images, archives) often share headers.
- **Choice:** partial hash covers head, middle and tail windows of 64 KB each.
- **Why:** fewer false partial collisions on large files for three bounded reads; full-hash confirmation is unchanged, so correctness does not depend on it.

## D-006 Known flaws in the reference cache are not copied
- **Context:** R2 found that dedup-photos buckets only cache misses by size (a new identical copy of a cached file can be missed), never prunes deleted entries, and does not count some failed opens.
- **Choice:** MyJanitor buckets all files by size regardless of cache state, prunes entries not seen in a completed scan of their root, and reports every failed read.
- **Why:** user data safety and "a failed read must never look like no duplicates".
