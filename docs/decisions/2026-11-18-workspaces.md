# Workspaces are the top-level context

**Decided by the owner on 2026-11-18**, first as six decisions, then in his answers to the draft
plan's nine questions.

## What was decided

- **The workspace is the top-level context, one at a time in the window.** Chats and Council runs
  belong to a workspace, not to a project. A chat is for the workspace: its agent works across the
  workspace's projects.
- **A chat can also start inside one project**, chosen when it is made: that project is its cwd,
  so its own `.claude/settings.json` (hooks, MCP servers, allow rules) loads under the repo's
  trust acceptance as today, and the workspace's other projects stay attached with their roles.
- **A project can be attached to several workspaces.**
- **A project can be attached read-only**, as a knowledge base: agents and the owner read and
  search it, Docs indexes it, and Detached keeps it fresh with fetch and pull every 15 minutes, at
  a switch to the workspace and in Fetch/Pull All. Nothing else writes to it: no switch, stash,
  worktree, Fix or commit. This is enforced for agents (permission rules, the sandbox's
  `denyWrite`), not only hidden in the UI.
  - Its MRs can be Council subjects: the head is fetched into `refs/detached/subjects/`; review
    yes, Fix no.
  - On native Windows, where Claude has no sandbox, Bash in such a chat is not enforced; the limit
    is accepted and goes to docs/TECHDEBT.md.
- **Start clean.** Existing per-project chats and Council runs are not migrated; they stay in an
  archive, read-only. Workspaces and their folders carry over.
- **Deleting a workspace moves its chats and runs to the archive.** The archive clears per item
  and all at once, each after a confirm.
- **Document tabs by category**, Docs | Chats/Council | Code, as category chips with counts at the
  start of one strip, each category with its own tabs. Every Markdown file is a Doc.
- **Open in Zed is removed everywhere**: the headers, the action and its keymap entry, the
  conflict bar, the conflict notice, the sidebar menu, and the code it leaves dead.

## Why

The center's single strip mixed a chat tab next to a file tab, and a chat or run tied to one repo
did not match work that spans several repos and a shared knowledge base.

## Rejected

- Several workspaces shown at once, as before.
- Migrating per-project chats and runs into workspaces.
- Read-only as a UI-only state that agents could still write through.
- One tab strip for every kind of document; and a strip per category in a fixed center split.
- Deleting a workspace's chats and runs with it.

## Plan

[PLAN-WORKSPACES.md](../../PLAN-WORKSPACES.md) builds this; its choices marked *Proposal* wait for
the plan review.
