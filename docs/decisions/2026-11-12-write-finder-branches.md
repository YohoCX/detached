# Write: merging or discarding a trusted finder's branch

Proposed on 2026-11-12, decided at the agent host's H1.

- What: the run view lists each kept `detached-run/<run>/<participant>` branch with two buttons.
  - **Merge** fast-forwards the subject's branch to it, with hooks off. It exists only when the
    subject is a local branch, and it is refused when that branch's worktree is dirty or the merge
    is not a fast-forward.
    - With several finders, the second Merge is usually not a fast-forward any more; the owner
      merges it with git.
    - Nothing is pushed.
  - **Discard** deletes the branch and its worktree after a confirm.
- Why: in orchestrator mode the owner takes a finder's work, or drops it, without leaving the run.
- Rejected: merge commits and conflict resolution in the client; Merge for a remote-branch or MR
  subject, whose branch is kept for the owner to push.
- Test: PLAN-PANELS P2.
