# Write: managed worktrees and their recipe

Proposed on 2026-11-12, decided at the agent host's H1.

- What:
  - `git worktree add` and `remove` under `<folder>/worktrees/<repo>/<branch>` for a workspace
    folder that holds repos, as `wt` does, or `data/worktrees/<repo>/<branch>` when the folder is
    itself a repo, with hooks off;
  - the recipe copies ignored files (`.env`, `.env.local`) at mode 0600 and runs its install and
    teardown commands;
  - remove keeps the branch.
- There is no Fix on an untrusted subject (PLAN-PANELS §12).
- Why: Fix and the owner's parallel work get their own trees, as `wt` gives them today.
- Rejected:
  - symlinks: they need privilege on Windows;
  - edits to the shared `info/exclude`, which would also hide paths in the main checkout. A copy of
    a file the repo does not ignore is refused instead.
- Test: PLAN-AGENT-HOST H6; PLAN-PANELS P4 for a Fix.
