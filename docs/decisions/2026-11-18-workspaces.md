# Workspaces are the top-level context

**Decided by the owner on 2026-11-18.**

## What was decided

- **The workspace is the top-level context.** Chats and Council runs belong to a workspace, not to
  a project. A chat is for the workspace: its agent works across the workspace's projects.
- **A project can be attached to several workspaces.**
- **A project can be attached read-only**, as a knowledge base: agents and the owner read and
  search it, Docs indexes it, and Detached keeps it fresh with fetch and pull. Nothing else writes
  to it: no switch, stash, worktree, Fix or commit. This is enforced for agents (permission rules,
  the sandbox's `denyWrite`), not only hidden in the UI.
- **Start clean.** Existing per-project chats and Council runs are not migrated; they stay
  readable in an archive, and new ones live in workspaces. Workspaces and their folders carry over.
- **Document tabs by category**, roughly Docs | Chats/Council | View/Git, each with its own tabs.
- **"Open in Zed" leaves the file header.**

## Why

The center's single strip mixed a chat tab next to a file tab, and a chat or run tied to one repo
did not match work that spans several repos and a shared knowledge base.

## Rejected

- Migrating per-project chats and runs into workspaces.
- Read-only as a UI-only state that agents could still write through.
- One tab strip for every kind of document.

## Plan

[PLAN-WORKSPACES.md](../../PLAN-WORKSPACES.md) builds this. Its other choices (one current
workspace, the chat's cwd, where runs live, the tab layout) are proposals for the owner's review,
listed in its §15.
