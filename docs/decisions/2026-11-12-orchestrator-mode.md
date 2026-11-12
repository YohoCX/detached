# Orchestrator mode, and trust by workspace or repo

Decided by the owner on 2026-11-12.

- **Free agents, reviewed as commits.** On trusted work, agents get free access while the owner
  orchestrates and reviews their work as commits.
- **Trusted subject: orchestrator mode.**
  - Agents and panel finders get the full shell, the file tools and commits, with no permission
    prompts and no sandbox.
  - Each works in its own worktree on its own branch. Verifiers read in the pinned tree, with no
    branch.
  - Detached shows each one's commits, and the owner merges what he wants.
  - Fix on a trusted subject is the same flow.
- **Untrusted subject:** a sandboxed, read-only review, and no Fix
  ([decision](2026-11-12-untrusted-read-only.md)).
- **Trust by workspace or repo.**
  - The owner can mark a whole workspace or a single repo trusted, such as work projects.
    Everything in it is trusted: local branches, colleagues' remote branches, and MRs and branches
    from that repo's own remote.
  - MRs from a fork stay untrusted unless "include forks" is ticked for that repo or workspace.
  - The per-branch mark stays for repos not trusted as a whole.
  - Marks are shown on the Sharing page and in each panel's start dialog, stored per machine in the
    personal scope, and never shared through git.
- **Rejected:** trust only per branch.
- **Plan:** [PLAN-PANELS.md](../../PLAN-PANELS.md) §3, §4, §12;
  [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §4, §5.
