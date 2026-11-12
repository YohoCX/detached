# Write: a tree of its own for each panel participant

Proposed on 2026-11-12, decided at the agent host's H1.

- What, for a trusted subject: `git worktree add` of a branch `detached/<run>/<participant>` at the
  subject's SHA, made with hooks off (`-c core.hooksPath=<empty dir>`). The branch stays with the
  participant's commits; the worktree is removed at the run's end.
- What, for an untrusted subject: no repo write. A full clone
  (`git clone --no-local --no-checkout`, then a checkout, hooks off) is made in the data tier.
- Why: participants work from a fixed point, and the owner's tree is never touched. In orchestrator
  mode their commits are the work the owner reviews.
- Rejected: running participants in the owner's worktree, and a shared detached tree that keeps no
  commits.
- Test: PLAN-PANELS P2: a branch that commits a `post-checkout` hook makes a worktree and a clone
  with nothing run outside the sandbox.
