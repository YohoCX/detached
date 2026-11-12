# Panels isolate by trust, with WSL2 on Windows; agents keep the shell

Decided by the owner on 2026-11-12. These are options E and F of the Windows choice.

- **The shell everywhere.** Every agent and panel participant keeps its terminal access on every
  OS. Host tools (diff, log, search, `detached_read`) are offered in addition, never instead.
- **E, by trust.**
  - A trusted subject runs its participants with the full shell, as the chat does, on every OS and
    with no sandbox. Trusted means the owner's own repo or branch, or an accepted repo.
  - Isolation applies only to an untrusted subject, such as someone else's MR: no write, no egress
    from tools, no cross-read.
- **F, WSL2 on Windows.**
  - For an untrusted subject on Windows, Claude Code runs inside WSL2, where its Linux sandbox
    (bubblewrap) works.
  - The pinned tree lives in WSL's filesystem for speed, and the host's HTTP tool is reached through
    localhost forwarding.
  - Without WSL, Detached offers to install it, or to run unisolated after a consent that names
    the risk.
- **Why:** reviewers need a real shell, and isolation costs only where a subject is hostile. WSL2
  gives Windows the same sandbox as Linux today.
- **Rejected:**
  - moving Claude roles to Codex on Windows (Codex does not stop cross-reads);
  - unisolated runs as the default;
  - no panels on Windows;
  - host tools in place of the shell.
- **Later:** a native Windows sandbox (AppContainer or a restricted token with a proxy allowlist),
  in docs/BACKLOG.md.
- **Plan:** [PLAN-PANELS.md](../../PLAN-PANELS.md) §3, §4;
  [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §5, H0b.
