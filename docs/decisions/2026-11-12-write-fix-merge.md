# Write: merging a Fix into the subject's branch

Proposed on 2026-11-12, decided at the agent host's H1.

- What: a fast-forward of the subject's branch to the Fix branch, on the owner's button only, and
  only when the subject is a local branch. For a remote branch or an MR, the Fix branch is kept for
  the owner to push.
- When it is refused:
  - the branch's worktree is dirty;
  - the merge is not a fast-forward.

  Nothing is pushed.
- How: with `-c core.hooksPath=<empty dir>`, so no hook the Fix wrote (a `.husky/post-merge`) runs
  as the owner. Changed hook files are shown as their own diff before the click.
- Why: the owner takes a fix without leaving the client, and their tree is never left mid-merge.
- Rejected: merge commits and conflict resolution in the client; an automatic merge.
- Test: PLAN-PANELS P4: a dirty tree, a non-fast-forward subject, a `.husky/post-merge` that never
  runs, an MR subject with no Merge.
