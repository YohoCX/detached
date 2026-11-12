# Write: managed worktrees and their recipe

Proposed on 2026-11-12, decided at the agent host's H1.

- What:
  - `git worktree add` and `remove` under `<workspace>/worktrees/<repo>/<branch>`, with hooks off;
  - the recipe copies ignored files (`.env`, `.env.local`) at mode 0600 and runs its install and
    teardown commands;
  - remove keeps the branch.
- On an untrusted subject the agent works in a plain export with no `.git`. The host copies its
  changed files into a managed worktree the agent never touched, and commits there with hooks,
  `core.fsmonitor` and signing off, through `--git-dir` and `--work-tree` from its own record. No
  recipe copy or `install` runs (PLAN-PANELS §12).
- Why: Fix and the owner's parallel work get their own trees, as `wt` gives them today.
- Rejected:
  - symlinks: they need privilege on Windows;
  - edits to the shared `info/exclude`, which would also hide paths in the main checkout. A copy of
    a file the repo does not ignore is refused instead.
- Test: PLAN-AGENT-HOST H6.
