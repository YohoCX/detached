# Claude through the user's own `claude`, on any login

Decided by the owner on 2026-11-12.

- **The chain.** Detached → `claude-agent-acp` (the Agent SDK) → the user's own `claude` →
  Anthropic.
  - The CLI comes from Anthropic's official installer and updates itself. Detached never ships or
    downloads one.
  - The adapter is our own lockfile-pinned bun build.
- **Logins.** Subscription logins are used in personal builds now. Before the public release the
  owner asks Anthropic for the same permission, citing the OpenCode precedent. If refused, public
  builds take an API key or another login through the same `claude`, with no code change.
- **Rules, as OpenCode's spec sets them:**
  - only the user's `claude`, from PATH or a setting;
  - the user logs in with `claude` in a terminal tab, and Detached never reads the saved login or
    passes a token;
  - only the public SDK, never between Claude Code and Anthropic or changing its requests;
  - any login type, with a subscription's cost shown as $0;
  - the name "Claude Agent", never "Claude Code".
- **Evidence:**
  - Zed's blog, <https://zed.dev/blog/anthropic-subscription-changes> (2026-05-14, updated
    2026-06-16):
    - Anthropic paused a planned separate "Agent SDK credit" pool.
    - ACP, `claude -p`, the Agent SDK and apps built on it "continue to work with Claude
      subscriptions exactly as they did before".
    - Anthropic promised advance notice of any change.
  - OpenCode merged a built-in "Claude Agent" provider on 2026-10-30
    (<https://github.com/anomalyco/opencode/pull/58782>). Its author wrote: "We got permission
    from Anthropic and everything here is fully tos compliant". That is a third party's claim;
    there is no public Anthropic confirmation.
  - OpenCode's spec:
    <https://github.com/Tarquinen/opencode/blob/claude-agent-provider/specs/v2/claude-agent-provider.md>.
- **If the credit pool comes back:**
  - the pool a session draws from is shown;
  - extra-usage errors get a clear message;
  - the official `claude` in a terminal tab stays the documented fallback on plan limits, as Zed
    recommends with its Terminal Threads;
  - panels' estimates and cap guard the biggest spender.
- **Rejected:**
  - PLAN-EXTENSIBLE's API-key-only rule for public builds, unless Anthropic refuses;
  - shipping our own copy of the Claude CLI, which upstream's Node-free archive would do.
- **Plan:** [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §2, §3, H12.
