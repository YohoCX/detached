# Every agent provider goes through ACP, Claude included

Decided by the owner on 2026-11-09: "I'd like one template for providers."

- Every provider runs as an ACP (Agent Client Protocol) adapter behind one host contract: Claude
  through `@agentclientprotocol/claude-agent-acp`, Codex through `codex-acp`, others as adapters
  appear. The Rust side speaks ACP; `core::agent`'s private protocol and `sidecars/claude` go.
- Why: one way to start, stop, configure and trust any agent; a new provider is a manifest, not a
  protocol.
- Rejected: Claude on the Agent SDK sidecar and the rest on ACP, which the agent advised because
  ACP lagged the SDK on usage, plan limits and `auto` mode. Those gaps are now the agent host's
  step 0 questions, not a reason for two paths.
- Plan: [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md).
