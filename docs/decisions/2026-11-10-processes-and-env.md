# Defined processes, started by the owner or an agent, with an env store

Decided by the owner on 2026-11-10.

- A process (a dev server, a watcher) is defined once and started from the client by hand or by
  an agent; agents read its output.
- The client keeps env variables and starts a process with them (docs/BACKLOG.md's "Local env
  store").
- Why: an agent that edits a server's code must see it run without the owner relaying logs.
- Rejected: agents typing into the owner's shells. An agent gets a process's log and a start or
  restart only where the definition allows it; [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §7
  says what that costs.
