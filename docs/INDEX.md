# Docs index

One line per module doc: `- <doc> — <covered paths> — read when <trigger>.` Agents read docs
through the read tool's `docs(query)`, which routes by these lines; PLAN-AGENT-HOST §12 has the
layout.

- docs/crates/core/src/docs.md — crates/core/src/docs.rs, crates/kernel/src/docs.rs — read when changing how docs are indexed, routed, flagged stale or offered for an update after a turn, or a docs query that finds the wrong doc.
- docs/crates/core/src/host.md — crates/core/src/host.rs — read when changing the MCP server behind `detached_read` and `detached_act`, the actions (finding, verdict, done, proc) and their JSON schemas, or how tool errors reach the agent.
- docs/crates/core/src/skills.md — crates/core/src/skills.rs, crates/kernel/src/skills.rs — read when asking where a session's skills come from, why a skill does not load (trust, a link inside, overridden by name), or how skills are copied into a session.
- docs/crates/core/src/watch.md — crates/core/src/watch.rs — read when changing the file watcher or how file events reach a repo's status, or debugging a watcher that misses, repeats or over-reports events (ignored paths, .gitignore, refs, Linux, macOS and Windows differences).
- docs/crates/kernel/src/host.md — crates/kernel/src/host.rs, crates/kernel/src/host — read when adding a field to `detached_read`'s GraphQL schema, changing what a session or review may see or do, or debugging a read query that returns null or is refused.
- docs/crates/kernel/src/inbox.md — crates/kernel/src/inbox.rs, crates/kernel/src/inbox — read when changing messages between sessions, groups, wake and its cap, a group's Stop or `watch`, or debugging a session woken too often, never, or in the wrong mode.
- docs/crates/kernel/src/resource.md — crates/kernel/src/resource.rs — read when adding or changing a kernel Resource or Interest, or debugging a value that reloads too often or never (reload on status or head change, window activation, polling, stale data, Refresh button).
