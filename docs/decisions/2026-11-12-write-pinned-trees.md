# Write: a detached worktree per panel run

Proposed on 2026-11-12, decided at the agent host's H1.

- What: `git worktree add --detach` at the subject's SHA under `data/trees/<run>/`, made with hooks
  off (`-c core.hooksPath=<empty dir>`), and `git worktree remove` at the run's end.
- Why: participants read a fixed tree; the owner's tree is never touched.
- Rejected: running participants in the owner's worktree. It changes while they read, and a
  participant's write would land in it.
- Test: PLAN-PANELS P2, a branch that commits a `post-checkout` hook makes a tree with nothing run.
