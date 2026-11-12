# Agents message each other as Claude Code sessions do

Decided by the owner on 2026-11-10: "make it work the way Claude Code does".

- Sessions in the client send each other messages that arrive by themselves, marked as coming from
  an agent and not from the user.
- Why: coordinated agents without the owner carrying text between tabs.
- Rejected: a shared room every agent reads, and messages pasted into the receiver's prompt as if
  the user wrote them. [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §9 has the rules.
