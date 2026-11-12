# Write: fetching an MR's head for a panel

Proposed on 2026-11-12, decided at the agent host's H1.

- What: `git fetch <remote> <MR head ref>` into `refs/detached/subjects/<run>`, removed with the
  run. Nothing else in the repo changes.
- Why: a panel over an MR from someone else needs its commits without a checkout.
- Rejected: checking the MR's branch out in the owner's worktree.
- Test: PLAN-PANELS P5.
