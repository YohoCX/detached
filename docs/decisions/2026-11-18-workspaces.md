# Workspaces are the top-level context

**Decided by the owner on 2026-11-18**: six decisions, then his answers to the draft plan's nine
questions and to the four each of its first two reviews raised; read-only's boundary on
2026-11-19, and the same day, after the W0 spike, Codex back and read-only kept out of the way
(last section).

## What was decided

- **The workspace is the top-level context, one at a time in the window.** Chats and Council runs
  belong to a workspace, not to a project. A chat is for the workspace: its agent works across the
  workspace's projects.
- **A chat can also start inside one project**, chosen when it is made: that project is its cwd,
  so its own `.claude/settings.json` (hooks, allow rules) loads under the repo's trust acceptance
  as today, and the workspace's other projects stay attached with their roles.
- **A project can be attached to several workspaces.**
- **A project can be attached read-only**, as a knowledge base: agents and the owner read and
  search it, Docs indexes it, and Detached keeps it fresh with fetch and pull every 15 minutes, at
  a switch to the workspace and in Fetch/Pull All. Detached writes nothing else to it: no switch,
  stash, worktree, Fix or commit, refused in core, not only hidden in the UI. Agents' file tools
  are denied there; Bash is not confined (2026-11-19, below).
  - It ships only together with its enforcement for chats and Council, never before it.
  - **Its boundary (2026-11-19):** it protects against a cooperative agent's accidental writes,
    not against an agent deliberately escaping: planting git config or hooks (`core.hooksPath`,
    submodule git dirs, `~/.gitconfig`), settings that widen the sandbox, edits to code a process
    Detached runs. Untrusted code goes through Council's untrusted path (sandboxed, an export, no
    Fix), which is the real isolation. So the plan keeps the file-tool denies on read-only paths
    and core refusing Detached's own writes, and drops hardening that only served against a
    deliberate escape.
  - Its MRs can be Council subjects: the head is fetched into `refs/detached/subjects/`; review
    yes, Fix no.
  - A Codex chat is never pinned to a read-only project.
  - Hooks from the owner's settings, accepted repos and the plugins they enable can still write
    it: the owner's own code, a stated limit.
- **Third-party MCP servers are off in every Detached chat and Council run**, the user's and the
  projects'; Detached's own server stays (the read tool, Council's report tools). Agents are
  extended through skills with scripts. Hooks load only from the user's own settings and repos the
  owner accepted, never from untrusted ones, as today. This changes today's chat and ships early,
  with a release-notes line. Codex's servers are turned off by name (2026-11-19, below).
- **A chat `--add-dir`s the workspace's trusted read-write projects**, so edits there work under
  `acceptEdits` and their skills, agents and commands load as the owner accepted them; the trust
  check covers them as it covers the cwd. Read-only and untrusted projects are files only: read
  rules, nothing loaded. A chat pinned to a project not accepted adds none, since added dirs load
  only under the project setting source it does not get.
- **Start clean.** Existing per-project chats and Council runs are not migrated; they stay in an
  archive, read-only. Workspaces and their folders carry over.
- **Deleting a workspace moves its chats and runs to the archive.** The archive clears per item
  and all at once, each after a confirm. Clearing removes only what Detached made and can prove;
  it never touches a transcript Detached did not start, such as the owner's own `claude` sessions
  in a terminal, and chats and runs from before workspaces stay, read-only, without Clear.
- **Document tabs by category**, Docs | Chats/Council | Code, as category chips with counts at the
  start of one strip, each category with its own tabs. Every Markdown file is a Doc.
- **Open in Zed is removed everywhere**: the headers, the action and its keymap entry, the
  conflict bar, the conflict notice, the sidebar menu, and the code it leaves dead.

## Why

The center's single strip mixed a chat tab next to a file tab, and a chat or run tied to one repo
did not match work that spans several repos and a shared knowledge base. MCP servers run outside
the sandbox and the deny rules, so read-only could not hold with them, and they cost context
tokens on every turn; skills with scripts cover the same use under the permission rules and
asks.

## Rejected

- Several workspaces shown at once, as before.
- Migrating per-project chats and runs into workspaces.
- Read-only as a UI-only state that agents could still write through.
- Read-only hardened against a deliberate escape (no writable home, denies on every repo's git
  config and hooks): it broke ordinary builds and still left routes open.
- One tab strip for every kind of document; and a strip per category in a fixed center split.
- Deleting a workspace's chats and runs with it.
- Third-party MCP servers only off beside a read-only project: they stay outside the sandbox
  everywhere.

## Plan

[PLAN-WORKSPACES.md](../../PLAN-WORKSPACES.md) builds this; its choices marked *Proposal* wait for
the plan review.

## 2026-11-19: Codex back, read-only out of the way

Decided by the owner after the W0 spike and W2.

- **Codex comes back.** Codex chats and Codex seats in trusted Council runs are on again. W0 found
  that Codex's config layers merge, so no override empties `mcp_servers`, while
  `mcp_servers.<name>.enabled = false` turns a server off by name. So at each Codex start and
  resume Detached reads the server names (the table names under `mcp_servers`, nothing else) of
  the user's Codex config (`$CODEX_HOME/config.toml`, else `~/.codex/config.toml`) and of the
  session cwd's `.codex/config.toml`, and turns each off through the adapter's `CODEX_CONFIG`.
  Detached's own server stays. A server Codex learns from somewhere Detached does not read (a
  project layer above the cwd, a profile, the system or managed config) is not turned off: a
  stated limit.
- **Read-only projects must not get in the way of the normal flow.** No Bash sandbox fence, no
  permission-mode limit, no refusal tied to the sandbox probe or `sandbox.excludedCommands`: ssh
  pushes, docker and everything else work as in any chat. What stays: `Edit`, `Write` and
  `NotebookEdit` denies on read-only project paths; core refusing Detached's own writes (git ops,
  the repo store, worktrees, processes); keep-fresh fetch and pull; Council review-only for a
  read-only subject; one line in the agent's workspace instructions naming the read-only projects
  as read-only. Bash, hooks and processes can still write there: the accidental-write boundary,
  accepted.

Why: Codex was off only because no override emptied its servers, and per-name covers what Codex
loads from the two files a user and a repo keep. The fence broke ssh remotes (W0 row 3) and
brought refusals and mode limits into every chat beside a knowledge base, for writes the owner
expects a cooperative agent not to make.

Rejected: Codex off in chats and trusted runs (W2 as first built); the Bash fence for read-only
projects, with `failIfUnavailable`, its probe and `excludedCommands` refusals; Codex limited to
`read-only` and `workspace-write` beside a read-only project; `default` and `plan` only on native
Windows.
