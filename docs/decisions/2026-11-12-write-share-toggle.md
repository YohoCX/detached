# Write: Share and Make local move files in the working tree

Proposed on 2026-11-12, decided at the agent host's H1.

- What: Share moves an asset from `<git common dir>/detached/local/` into `<repo>/.detached/`;
  Make local moves it back. The move shows in Changes; Detached commits nothing.
- Why: the owner chooses, per item, what the team gets through git.
- Rejected: Detached committing the move.
- Test: PLAN-AGENT-HOST H2.
