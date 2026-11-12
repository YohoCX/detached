# Write: managed worktrees and their recipe

Proposed on 2026-11-12, decided at the agent host's H1.

- What:
  - `git worktree add` and `remove` under `<workspace>/worktrees/<repo>/<branch>`, with hooks off;
  - the recipe copies ignored files (`.env`, `.env.local`) at mode 0600 and runs its install and
    teardown commands;
  - remove keeps the branch.
- On an untrusted subject the agent works in a tree git checked out with no `.git`, through a temp
  index. The host stages it back with `add -A` into that index, so git's own path checks refuse
  every `.git` spelling, and commits with `commit-tree`, with hooks, `core.fsmonitor` and signing
  off. The Fix branch is checked out by git into a managed worktree the agent never touched. No
  recipe copy or `install` runs. The installed git must pass a probe of the bad-path cases
  (PLAN-PANELS §12).
- Why: Fix and the owner's parallel work get their own trees, as `wt` gives them today.
- Rejected:
  - symlinks: they need privilege on Windows;
  - edits to the shared `info/exclude`, which would also hide paths in the main checkout. A copy of
    a file the repo does not ignore is refused instead.
- Test: PLAN-AGENT-HOST H6.
