# Write: files and refs under the git directory

Proposed on 2026-11-12, decided at the agent host's H1.

- What:
  - files under `<git common dir>/detached/`: run records, local assets, the session index;
  - refs under `refs/detached/subjects/` for MR heads fetched for a panel, besides today's
    `refs/detached/turns/`.
- Why: per-repo data follows every linked worktree, stays out of the working tree and is never
  pushed or cloned.
- Rejected: a `.revmux/`-style folder in the working tree.
- Test: PLAN-AGENT-HOST H2, a write there reloads no status; PLAN-PANELS P5, a fetched MR head is
  pruned with its run.
