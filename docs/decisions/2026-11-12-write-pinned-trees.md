# Write: a tree of its own for each panel participant

Proposed on 2026-11-12, decided at the agent host's H1.

- What:
  - **Every run:** a pinned tree, `git worktree add --detach` at the subject's SHA under
    `data/trees/<run>/`, made with hooks off (`-c core.hooksPath=<empty dir>`) and removed at the
    run's end. Verifiers, and every participant of an untrusted subject, read there.
  - **A trusted subject's finders:** each gets its own plain `git worktree add` of a branch
    `detached/<run>/<participant>` at that SHA, with hooks off. At the run's end, a branch with no
    commits beyond the subject is deleted. One with commits stays, shown in the run view, until the
    owner merges or discards it.
- Why: participants work from a fixed point, and the owner's tree is never touched. In orchestrator
  mode their commits are the work the owner reviews.
- Rejected: running participants in the owner's worktree, and a shared detached tree that keeps no
  commits.
- Test: PLAN-PANELS P2: a branch that commits a `post-checkout` hook makes trees with nothing run;
  a finder's empty branch is deleted, and one with commits stays.
