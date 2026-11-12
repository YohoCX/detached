# Write: a tree of its own for each panel participant

Proposed on 2026-11-12, decided at the agent host's H1.

- What:
  - **Every run:** a pinned tree, `git worktree add --detach` at the subject's SHA, made with hooks
    off (`-c core.hooksPath=<empty dir>`) and removed at the run's end.
    - For a trusted subject it is at `data/trees/trusted/<run>/pinned/`, where verifiers read.
    - For an untrusted subject it is at `data/untrusted/<root>/pinned/`, where every participant
      reads. It is made with the global and system config off, `GIT_LFS_SKIP_SMUDGE=1`, and every
      filter's `process` and `smudge` empty, so no filter runs under the subject's attributes.
  - **A trusted subject's finders:** each gets its own plain `git worktree add` of a branch
    `detached-run/<run>/<participant>` at that SHA, at
    `data/trees/trusted/<run>/finders/<participant>/`, with hooks off. At the run's end:
    - with no commits and no changes, the worktree and the branch are removed;
    - with commits and a clean tree, the worktree is removed and the branch kept;
    - with uncommitted changes, both are kept, marked in the run view.

    Merge and Discard of a kept branch are
    [their own write](2026-11-12-write-finder-branches.md).
- Why: participants work from a fixed point, and the owner's tree is never touched. In orchestrator
  mode their commits are the work the owner reviews.
- Rejected: running participants in the owner's worktree, and a shared detached tree that keeps no
  commits.
- Test: PLAN-PANELS P2:
  - a branch that commits a `post-checkout` hook and an LFS filter makes trees with nothing run;
  - the run-end cases;
  - an existing `detached-run` branch refuses the run.
