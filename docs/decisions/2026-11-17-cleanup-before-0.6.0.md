# Claude's old paths and the closed plans go before 0.6.0

**Decided by the owner on 2026-11-17**, on the cleanup inventory of that day.

## What was decided

- **Claude's ACP adapter path and the chat's sidecar go now**, without waiting for
  `make agent-login`: `adapters/claude-agent-acp`, its AIR declaration and Install on the Agents
  page; `sidecars/claude`, `core::agent`, `fake_sidecar` and `make sidecar`. Claude runs on the
  [direct connection](2026-11-16-claude-direct-connection.md) alone. That decision removed the
  adapter path only after every OS passed; this one does not wait.
- **All Claude-on-ACP code goes:** `meta()`, `DENIED`, `Options.meta`, `failure()`'s Claude text,
  `initial()`'s `configOptions` where only Claude read them, and the test manifest `CLAUDE_ACP`.
  The fake-ACP tests run on a Codex-shaped manifest; Codex's ACP path keeps working.
- **`spikes/claude-direct/` and `.github/workflows/h4c-evidence.yml` go**, and the owner's rerun
  of H4c part 0 is dropped.
- **`PLAN.md`, `PLAN-M2.md`, `PLAN-EXTENSIBLE.md` and `crates/app/examples/spike.rs` are
  deleted**; git history keeps them. PLAN-EXTENSIBLE's steps 4 and 5 (plugins) become BACKLOG
  rows, and PLAN.md's "Git runner" rules a doc comment on `core::git::runner`.
- **Also:** the `LEGACY` settings migration goes, duplicated helpers become one, test-only API
  moves under `#[cfg(test)]` or goes (`impl Serialize for Error` with it), and `sha2` moves from
  0.10 to 0.11.

## Why

- The direct connection replaced both Claude paths. What was left of them only cost builds, CI
  jobs and a bun toolchain, and kept a manifest no user runs.
- The spike's fixtures already live in `crates/core/tests/fixtures/claude`; its `sdk` and
  `adapter` rows needed the paths that go.
- The three plans are closed: PLAN.md since 2026-10-29 and PLAN-M2 as 0.2.0, both on Tauri;
  PLAN-EXTENSIBLE's steps were built, replaced by H11 or never started. Their links sent readers
  to rules the code no longer follows.
- Only 0.1 and 0.2 wrote the top-level `folders` and `excludedRepos`; workspaces came in 0.3.

## The way back

The owner gave up the fallback to the adapter path. If the direct connection fails on the owner's
login, the way back is a `git revert` of the removal commits.

## Rejected

- Keeping `CLAUDE_ACP` for the fake-ACP tests, as PLAN-AGENT-HOST H4c part b had it.
- Waiting for the part-0 rerun and for `make agent-login` before removing the adapter and the
  sidecar.
- Archiving the plans instead of deleting them.
