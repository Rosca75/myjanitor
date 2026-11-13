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
