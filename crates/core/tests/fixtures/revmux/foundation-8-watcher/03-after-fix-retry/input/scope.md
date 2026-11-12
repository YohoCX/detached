# Scope

Branch `foundation/8-watcher` (worktree `.claude/worktrees/8-watcher`, the review's workdir). Round 01 reviewed up to 7c144ba; this round reviews the three fix commits on top.

- Change: `git diff 7c144ba..HEAD`, 3 commits: `git log --oneline 7c144ba..HEAD`. 5 files, +205 −34.
- 71aca22: round 01's findings 1–5 (`*.lock` only under refs, inotify forgets a lost directory, excludes reread on each set, activation sets the watcher again, TECHDEBT's Windows overflow).
- 583abda: an OS dropped-events signal marks only the repos at, under or above its path; one under an ignored path marks nothing.
- c22650b: activation loads only statuses older than 30 s; the Refresh button and a settings change load all.
- Read in full: `crates/core/src/watch.rs`, `crates/kernel/src/invalidate.rs`.
- Spec: `PLAN-FOUNDATION.md` §7 and "Step 8" with its Done note. Repo rules: `CLAUDE.md`.
- Ignore: doc wording, formatting, import order.
