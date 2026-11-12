# Project docs written and read by a mechanism, not by habit

Decided by the owner on 2026-11-12: "a good way to document properly and to read docs properly".

- Every doc kind has one place and one template: module docs with an index, plans, decisions,
  contracts, backlog, tech debt.
- Agents read docs through a host tool that routes by the index and returns one section.
- A doc declares the code it covers, so the client sees when that code moved after the doc.
- A turn that touches covered code asks for the matching section to be updated.
- Why: decisions got lost in chat transcripts. The ACP decision of 2026-11-09 was found only by
  searching old sessions.
- Rejected: docs as free text that agents read in full, and the client writing docs by itself.
  Agents write them; the client routes, checks and asks.
- Plan: [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §12.
