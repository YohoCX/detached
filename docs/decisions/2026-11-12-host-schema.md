# The host's read tool: an SDL file and our own synchronous executor

Decided by the owner on 2026-11-12.

- `detached_read` is described by one SDL file in the repo, the single source of truth.
- Queries and the SDL are parsed with `graphql-parser` 0.4.1: synchronous, about 13 crates and
  4.4k lines in its tree.
- Our own synchronous executor runs over a kernel snapshot through `kernel::blocking`. It validates
  against the SDL and limits depth and size, in about 600–900 lines.
- No introspection protocol: asking what is there returns the SDL itself.
- No GraphQL subscriptions: events and messages go through the host's delivery.
- Why: agents get one precise query surface, and the kernel stays synchronous with no async
  resolvers.
- Rejected: `async-graphql`, at about 100 crates and 47k lines with HTTP, multipart, `async-io`,
  federation and dataloader, plus async resolvers against the kernel's model.
- Plan: [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §7.
