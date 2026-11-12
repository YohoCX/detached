# Write: merging a Fix into the subject's branch

Proposed on 2026-11-12, decided at the agent host's H1.

- What: a fast-forward of the subject's branch to the Fix branch, on the owner's button only.
- When it is refused:
  - the branch's worktree is dirty;
  - the merge is not a fast-forward.

  Nothing is pushed.
- Why: the owner takes a fix without leaving the client, and their tree is never left mid-merge.
- Rejected: merge commits and conflict resolution in the client; an automatic merge.
- Test: PLAN-PANELS P4: a dirty tree, a non-fast-forward subject.
