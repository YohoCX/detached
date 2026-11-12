# Where agent data lives, and a share-or-local toggle

Decided by the owner on 2026-11-12, after looking at `~/.agents` and `wt`.

- Agent assets have three scopes:
  - personal, in the app's config directory;
  - local to a repo, in `<git common dir>/detached/`;
  - shared with the repo's team, in `<repo>/.detached/`, committed.
- A per-repo toggle per kind (skills, panel roles and modes, the project profile, the worktree
  recipe, process definitions) and per item says whether it is shared through git or kept local.
  Run records are always local.
- Why: the owner decides what a team sees. Records in the git directory follow every linked
  worktree and stay out of the working tree.
- Rejected: `.revmux/`-style folders in the working tree, symlinked into worktrees, and Detached
  committing anything itself.
- Plan: [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §9.
