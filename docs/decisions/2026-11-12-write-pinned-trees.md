# Write: a tree of its own for each panel participant

Proposed on 2026-11-12, accepted by the owner at H1 on 2026-11-16.

- What:
  - **Every run:** a pinned tree at the subject's SHA, made with hooks off
    (`-c core.hooksPath=<empty dir>`) and removed at the run's end.
    - For a trusted subject it is a `git worktree add --detach` at
      `data/trees/trusted/<run>/pinned/`, where verifiers read.
    - For an untrusted subject it is not a worktree but an export at
      `<untrusted root>/<run>/pinned/` (per-user across instances, 0700, host §5), outside
      the config, data and cache tiers, so the repo's git dir never names it. Participants read there. Raw Git trees/blobs supply bytes and modes, without filters or archive
      attributes, with hooks and global/system config off. WSL receives tar entries directly
      from those objects, preserving symlinks without a Windows checkout. One root lock across
      instances covers export through session shutdown and deletion; under it, remove leftovers
      before starting. Waiters create no output; crashed runs start over.
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
  - an existing `detached-run` branch refuses the run;
  - exports and reviewer diffs run no filter; WSL preserves Git symlinks and executable modes;
  - different-config instances serialize with no readable predecessor export or temp output.
