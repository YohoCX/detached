# Write: files and refs under the git directory

Proposed on 2026-11-12, decided at the agent host's H1.

- What:
  - files under `<git common dir>/detached/`: run records and local assets;
  - refs under `refs/detached/subjects/` for MR heads fetched for a panel, and under
    `refs/detached/fix/` for an untrusted Fix's result, besides today's `refs/detached/turns/`.
- Why: per-repo data follows every linked worktree, stays out of the working tree and is never
  pushed or cloned.
- Rejected: a `.revmux/`-style folder in the working tree.
- Test: PLAN-AGENT-HOST H2, a write there reloads no status; PLAN-PANELS P5, a fetched MR head is
  pruned with its run.
