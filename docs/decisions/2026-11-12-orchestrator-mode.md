# Orchestrator mode, and trust by workspace or repo

Decided by the owner on 2026-11-12.

- **Free agents, reviewed as commits.** Agents get free access while the owner orchestrates and
  reviews their work as commits. Isolation guards only what a commit review cannot undo: reading
  secrets, network egress, and outward actions.
- **Trusted subject: orchestrator mode.**
  - Agents and panel participants get the full shell, the file tools and commits, with no
    permission prompts and no sandbox.
  - Each works in its own worktree on its own branch.
  - Detached shows each one's commits, and the owner merges what he wants.
  - Fix on a trusted subject is the same flow.
- **Untrusted subject: freedom inside a sandboxed clone.**
  - The agent gets its own full clone (`git clone --no-local --no-checkout`, then a checkout, hooks
    off for the host's part), and inside the sandbox everything: the shell, `Edit`, `Write`,
    commits, its own hooks.
  - The sandbox denies `$HOME` except what the CLI needs, other trees, the records, and all network
    except the model API. Package registries can be opened per run behind a consent that names the
    risk.
  - The host never runs git in the clone or reads its config. It takes the result from a bundle
    made inside the sandbox, fetched with `transfer.fsckObjects` and `fetch.fsckObjects` into
    `refs/detached/fix/<id>`.
  - On Windows the clone lives in Detached's WSL distro, and the bundle comes out through
    `wsl.exe` stdout.
- **Trust by workspace or repo.**
  - The owner can mark a whole workspace or a single repo trusted, such as work projects.
    Everything in it is trusted: local branches, colleagues' remote branches, and MRs and branches
    from that repo's own remote.
  - MRs from a fork stay untrusted unless "include forks" is ticked for that repo or workspace.
  - The per-branch mark stays for repos not trusted as a whole.
  - Marks are shown on the Sharing page and in each panel's start dialog, stored per machine in the
    personal scope, and never shared through git.
- **Rejected:**
  - the export with a temp index and host-side `add -A` for an untrusted Fix;
  - a git version probe;
  - a patch-only Fix;
  - trust only per branch.
- **Plan:** [PLAN-PANELS.md](../../PLAN-PANELS.md) §3, §4, §12;
  [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §4, §5.
