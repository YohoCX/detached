# An untrusted subject gets a read-only review, and no Fix

Decided by the owner on 2026-11-12, after review round 08. It replaces the sandboxed clone of the
orchestrator-mode decision for untrusted subjects.

- **What:**
  - On an untrusted subject, participants read and report findings in the pinned tree, under the
    isolation spike 0b verified:
    - the sandboxed shell plus `Read`, `Grep`, `Glob` and the host tools;
    - `denyWrite` on the tree;
    - `denyRead` on the enumerated secret classes, peers' trees and the records;
    - network deny with `strictAllowlist`;
    - a temp dir per session.
  - Nothing writes and nothing commits.
  - There is no Fix: the card offers "mark the repo trusted" or "fix it yourself".
  - Trusted subjects keep orchestrator mode.
- **Why:** round 08 showed that a free, committing agent on untrusted code inside Claude's sandbox
  does not hold yet:
  - the sandbox's mandatory denies on `.git/hooks` and `.git/config`;
  - toolchains under a denied `$HOME`;
  - the CLI's login in its config dir;
  - no git identity;
  - deny-wins `Read` rules;
  - no sandbox for the host's `git bundle`;
  - `fsckObjects` on a bundle fetch only from git 2.48;
  - a clone that lacks MR commits and carries every local branch;
  - clones filling the disk.
- **Rejected:** the sandboxed clone with an in-sandbox bundle, kept as a backlog feature with its
  own spike.
- **Plan:** [PLAN-PANELS.md](../../PLAN-PANELS.md) §3, §4, §12;
  [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §5.
