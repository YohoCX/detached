---
covers:
  - crates/core/src/settings.rs
  - crates/kernel/src/workspace.rs
  - crates/app/src/dock
  - crates/app/src/repos/workspaces.rs
  - crates/app/src/chat
---
# Detached — workspaces plan

The workspace becomes the top-level context: chats and Council runs belong to it, its agents work
across its projects, a project can sit in several workspaces and be attached read-only as a
knowledge base, and the center's tabs are split by category.

**Status (2026-11-18): drafted, waiting for the owner's review.** Every choice below that the
owner did not make is marked *Proposal*; §15 lists the questions.

Owner's decisions, 2026-11-18 ([decision](docs/decisions/2026-11-18-workspaces.md)):
1. The workspace is the top-level context. Chats and Council runs belong to a workspace; a chat's
   agent works across the workspace's projects.
2. One project can be attached to several workspaces.
3. A project can be attached read-only: read, search, Docs, and fetch/pull to keep it fresh.
   Nothing else writes to it, enforced for agents, not only hidden in the UI.
4. Start clean: per-project chats and Council runs are not migrated; they stay readable in an
   archive.
5. Document tabs by category, roughly Docs | Chats/Council | View/Git.
6. The "Open in Zed" button leaves the file header.

Words: a **project** is a repo the workspace lists (today's `RepoInfo`); an **attachment** is a
project in a workspace, with a **role**, read-write or read-only. Council is today's Panels; its
code rename is separate (§13).

## 1. The model

*Proposal.* `workspaces` stays core's known key; `Workspace` gains `id` and `readOnly`:

```json
"workspaces": [
  {"id": "3f9a2c1e", "name": "Detached", "folders": ["/Users/me/dev/personal"],
   "excludedRepos": [], "readOnly": ["/Users/me/dev/kb"]}
]
```

- **Attachments are today's folders.** A folder (a repo, or a folder of repos discovered as now)
  minus `excludedRepos`. A repo two workspaces list is attached to both: many-to-many with no new
  table.
- **Role.** Read-only when the repo's path or an ancestor folder is in `readOnly`; a linked
  worktree takes its repo's role; else read-write. Per attachment: the same repo can be read-only
  in one workspace and read-write in another.
- **Identity is `id`**, 8 random hex, made once. Chats, runs, tabs, trust marks and ui_state keys
  name the id; the name is a unique label.
- **Rename** changes the name alone. **Delete** confirms with the counts of its chats and Council
  runs and deletes them with its folder (§4.1); its projects are not touched.
- **A project in no workspace** is not listed, as today. Its old chats and runs stay in the
  archive (§7).
- Core: `list_repos(ws)`; `RepoInfo.read_only`; `set_read_only(ws, path, on)`; the folder calls
  take the id.

## 2. One current workspace

*Proposal* (Q1). The window shows one workspace: its projects in the sidebar and the repo
switcher, its chats, its Council runs, its tabs. The top bar's Workspaces widget, which turns
several on and off today, becomes a switcher.

- The current id is `ui_state` `workspace.current`; the first workspace when it is unset or gone.
  The current repo is remembered per workspace (`workspace.repo.<id>`).
- Kernel: `Workspace` gains `id()`, `name()`, `role(repo)` and `switch(id)`, which re-lists the
  repos, restores the workspace's current repo and emits `Switched` once. The dock's center, the
  Chats app and the Council app follow it.
- A switch stops nothing: other workspaces' chat turns and runs go on, their jobs in Activity under
  the workspace's name; processes and terminals belong to projects (§6).
- Why one: a new chat, a run and a set of tabs each need one owner, and a union of several
  workspaces has none. Rejected: today's several active at once.

## 3. Read-only projects

- **Detached reads everything:** tree, files, diffs, log, MRs, pipelines, find, Docs.
- **Keeps it fresh:** fetch, then `pull --ff-only` when the worktree is clean and the branch
  tracks; otherwise the pull is skipped and the project's row says why. At a switch to the
  workspace, every 15 minutes while the window shows, and in Fetch All and Pull All. *Proposal:* a
  fixed interval until the backlog's Auto-fetch makes it a setting.
- **Refuses every other write** of CLAUDE.md's list: switch and checkout, new and delete branch,
  stash, prune, managed worktrees and recipes, chat snapshots and restores, Council finder
  branches, pinned worktrees and Fix, and MR subject refs (Q4). Processes do not start (§6).
- **In core, not only in the UI.** Each write entry point takes the workspace id and refuses a
  read-only attachment: "`kb` is read-only in this workspace".
- **UI:** a lock on the project's row and in the file and diff headers; the branch widget shows
  the branch without its popup; write items leave menus and the VCS popup; a chord for one says
  why through Activity.
- **The user's own terminal is not limited:** what the owner types is the owner's.
- Agents: §4.3.

## 4. The workspace chat

### 4.1 Its cwd

*Proposal.* `data/workspaces/<id>/chat/`, made by Detached, holding two generated files:
`AGENTS.md` (the workspace's name, each project's path and role, and "read-only projects: read and
search only; never write, switch, stash or commit there") and `CLAUDE.md`, which is `@AGENTS.md`.
They are rewritten when the attachments change.

- Why: the CLI keeps transcripts by cwd, so one folder per workspace lists its chats (§4.6); no
  project is primary; a rename moves nothing.
- Rejected: a primary project as cwd. Its transcripts mix with the project's own; a read-only
  primary would be the cwd the CLI writes by default.
- Cost: a project's `.claude/settings.json` (hooks, MCP servers, allow rules) does not load in a
  workspace chat (Q3).

### 4.2 The projects given to the agent

- `Spec` gains `dirs: Vec<Dir { path, read_only }>`; `Spec.repo` stays the cwd. No protocol word.
- **Claude:** `--add-dir <path>` for every attached project, read-only ones included, so `Read`,
  `Grep` and `Glob` there do not ask; `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`, so each
  project's `CLAUDE.md` loads. W0 confirms both on `min_cli`.
- **Codex:** read-write projects as additional writable roots (W0 names the adapter's field).
  Read-only projects are not added: `workspace-write` reads the whole disk and writes its roots
  alone (§11 of the host plan: added directories become writable roots).

### 4.3 Read-only, enforced

- **Claude's file tools, every OS:** `Edit`, `Write` and `NotebookEdit` deny rules on
  `//<project>/**` and its git common dir, in `--disallowedTools`. A deny wins over `--add-dir` and
  every mode (W0 row 2).
- **Claude's Bash, macOS and Linux:** a workspace with a read-only project runs its chat in
  Claude's sandbox: `filesystem.denyWrite` on those projects and their git dirs, writes elsewhere
  allowed, `allowUnsandboxedCommands: false`, the network as today (W0 row 4 finds the settings).
  The "sandbox ready" probe must pass; when it fails, the rule below applies and the chat says so.
- **Claude's Bash on native Windows**, which has no sandbox: in that workspace the modes that run
  Bash unasked (`bypassPermissions`, `auto`) are off, and each Bash ask's card says the shell can
  write the read-only projects. A stated limit, a TECHDEBT row.
- **Codex:** its asking mode's `workspace-write` holds; full access is refused in a workspace with
  a read-only project.
- **Host tools:** `detached_act` has no repo write; `proc` refuses a read-only project's processes.
- **A change** of attachments or roles ends the workspace's live sessions at their turn's end; the
  next message resumes with the new flags, as a trust change does (host §4).

### 4.4 Trust and skills

- Acceptance stays per repo and provider, against its trust files' hash. A workspace chat loads
  from a project only its skills and instruction files: an added directory's settings do not load
  (W0 row 3), and the cwd has none.
- **Skills:** each trusted project's skill folders join the folder list; an untrusted project's
  are turned off by name, as today, and listed with why. *Proposal:* a name two projects share
  loads from the project attached first; the Skills page lists the other as shadowed. Codex reads
  the rest through `detached_read skills`, as now.
- The chat's start screen lists each untrusted project with its Trust… button, where today it has
  one banner.
- The watcher holds every attached project's trust paths while the session lives.

### 4.5 The read tool

A chat reads its own workspace, not the current one: `Workspace { id name repos current }`,
`Repo.readOnly: Boolean!`, `sessions(workspace: ID)`. A Council participant's scope is unchanged.

### 4.6 History, list, resume, delete

- Claude keeps a workspace's transcripts in one CLI project folder, the cwd's; `past(cwd)` lists
  them as it lists a repo's today. Resume passes the same `Spec`, `dirs` included; delete and the
  archive flag are as now. Codex: `session/list` narrowed to the host's index, cwd the same folder.
- `Doc::Chat { workspace, n }`; the chats global is keyed by `(workspace, n)`; the Chats app lists
  the current workspace's open and past chats.
- **Snapshots** cover the workspace's read-write projects, where `Session::repos` takes a folder's
  repos today; never a read-only one.

## 5. Council

- A run belongs to a workspace; its subject names a project's branch, MR, issue or document.
- *Proposal:* records go to `data/workspaces/<id>/council/<task>/<run>/`, not
  `<git common dir>/detached/panels/`. Why: the run is the workspace's; a project in two
  workspaces would show it in both; a read-only project's git dir takes no write. This amends
  [storage scopes](docs/decisions/2026-11-12-storage-scopes.md): run records stay local, per
  workspace. Subject refs, finder branches, pinned worktrees and Fix stay in the subject's repo.
- revmux imports (`.revmux/`) stay per project, under the project in the workspace's Council app.
- Trust marks: `panels.workspaces` by id.
- **A read-only subject** gets a read-only run in every mode: participants read an export from raw
  blobs (the untrusted path's export), no pinned worktree, no finder branches, no Fix; the card
  says "read-only in this workspace". Trust marks still decide the sandbox and which settings
  load. Its MRs: Q4.
- `Doc::Panel`, `PanelTask` and `PanelStats` take `workspace` in place of `repo`; stats read the
  workspace's runs.

## 6. Processes and worktrees

- *Proposal:* **processes stay per project.** Definitions are the project's store items and a run
  belongs to its project, running on across a switch. The Processes app lists the current
  workspace's read-write projects, grouped, where today it lists the current repo's. A read-only
  project's processes are neither listed nor started, by the owner or an agent: they run repo code
  that writes.
- A workspace chat's `proc` reaches its read-write projects' processes at their `agents` level.
- Managed worktrees stay per project (`<folder>/worktrees/<repo>/<branch>`), shown in every
  workspace that lists it; none for a read-only one.

## 7. The archive

- **Chats:** a collapsed Archive section at the end of the Chats app: today's list, past chats by
  repo and folder. One opens read-only as `Doc::PastChat { provider, cwd, id }` (kind
  `chat-archive`): its transcript, no composer, no resume (Q8). Delete stays.
- **Council:** runs under the workspace's projects' `<git common dir>/detached/panels/` show under
  Archive in the Council app: rounds, findings, outcomes, no next round and no Fix. A kept finder
  branch keeps Merge and Discard in a read-write project, so none is stranded.
- Nothing is moved or copied.

## 8. Tabs by category

**The model**, the same in both options:
- `dock::register_category(cx, id, title, order)`; `DocKind::category(&self, doc) -> &'static str`
  with no default, so a new kind names one; a test fails on a kind whose category is not
  registered.

  | Category | Kinds |
  | --- | --- |
  | `docs` "Docs" | `file` of a Markdown path (`.md`, `.mdx`, `.markdown`) |
  | `chats` "Chats" | `chat`, `chat-archive`, `panel`, `panel-task`, `panel-stats` |
  | `code` "Code" | `file` otherwise, `diff`, `mr`, `job`, `terminal` |

- Each category has its own tabs, active tab and preview tab: a file from the tree replaces the
  code preview, a plan from Docs the docs preview. `show_doc` opens a doc in its category and
  brings the category forward.
- Side zones are unchanged: a tab dragged to a zone leaves its category.
- The center is the current workspace's: a switch swaps it.
- ui_state: `dock.layout` keeps the zones; `tabs.<workspace id>` keeps the category shown and a
  dump of each category's tabs.
- Keys, ids in `actions.rs`, chords in `keymap.rs` and docs/KEYMAP.md:

  | Action id | Action | Keys |
  | --- | --- | --- |
  | `tabs.nextCategory` / `tabs.prevCategory` | Next / previous category | Ctrl+Tab / Ctrl+Shift+Tab |
  | `tabs.next` / `tabs.prev` | Next / previous tab in the category | Cmd+Shift+] / Cmd+Shift+[ |
  | `tabs.docs` / `tabs.chats` / `tabs.code` | Show that category | |

  A terminal tab's interceptor passes the four chorded actions on, as it does `panel.project`.

**Option A, a category switcher at the start of one strip** (recommended):

```
┌────────────────────────────────────────────────────────────────────┐
│ Docs 2 │▌Chats 3▐│ Code 5 ┃ align-2-0 × │ review: ws-plan × │        │
├────────────────────────────────────────────────────────────────────┤
│                                                                    │
│            the Chats category's active tab                         │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
```

Three chips with counts, drawn by our `skin` at the strip's start; the strip shows the chosen
category's tabs alone. Built on the kit's `DockArea::set_center` with each category's layout, its
tab entities kept, so a switch rebuilds no view.
- For: one strip, nothing new to lay out, the whole center for a diff or a chat; a category can
  still split the center by dragging a tab to an edge.
- Against: a chat beside a diff takes a drag to a side zone or a split.

**Option B, a strip per category in a fixed center split:**

```
┌───────────────────────────────┬────────────────────────────────────┐
│ Chats │ align-2-0 │ review     │ Code │ biome.json │ main.rs Δ      │
│                               ├────────────────────────────────────┤
│  chat                         │  file or diff                      │
│                               │                                    │
├───────────────────────────────┴────────────────────────────────────┤
│ Docs │ PLAN-WORKSPACES.md │ KEYMAP.md                               │
│  doc                                                               │
└────────────────────────────────────────────────────────────────────┘
```

Each category is a tab panel of its own; an empty one collapses; split sizes persist in
`tabs.<workspace id>`.
- For: a chat beside the code it talks about, with no switching.
- Against: three panes on a laptop screen; keys need a focused pane; the kit re-lays the center
  out when a pane collapses.

Recommendation: A. The complaint is the mixed strip; A fixes it without taking screen, and B's
side-by-side is one drag away in A.

## 9. Open in Zed

The file header's `viewer::zed_button` goes (`files/file.rs`). *Proposal* (Q5): the
`file.openInZed` action and its KEYMAP row go too; the conflict bar's button, the conflict
notice's and the sidebar menu's item stay, since resolving a conflict needs an editor.

## 10. Migration

- **Workspaces carry over.** At boot, one `update` gives every workspace without an `id` one and
  rewrites `panels.workspaces` marks from names to ids.
- **`active`** is read (`skip_serializing`) and dropped on save: the first active workspace becomes
  `workspace.current`.
- **Chats and runs are not migrated** (owner's 4); §7 shows them.
- **The saved layout:** the center's tabs of the old `dock.layout` load once into the current
  workspace's categories; old `Doc::Chat` and `Doc::Panel` tabs, keyed by repo, are dropped.
- **A stated limit:** a build older than this drops `id` and `readOnly` on its next save, and a
  later boot then makes new ids: that workspace's chats and runs are no longer found.

## 11. Written rules this changes

- CLAUDE.md: the write list says none of it happens in a read-only attachment; Native UI gains the
  tab categories.
- PLAN-FOUNDATION §3 and PLAN-NATIVE Contracts: `Workspace::switch`, `id`, `role`;
  `dock::register_category`, `DocKind::category`.
- PLAN-AGENT-HOST §7 (a chat reads its workspace), §10 (run records), §11 (skills of several
  projects), §13 (extra folders are the workspace's projects).
- PLAN-PANELS §3 (read-only subjects), §14 (where runs live).
- docs/KEYMAP.md: the tab rows; `file.openInZed` per Q5.
- README: workspaces and read-only projects.

## 12. Steps

Each step passes the host plan's gate: `make check`, a revmux round without blocking findings, the
owner's check on macOS. Docs of §11 change in the step that makes them true.

### Step W0. Spike: agents across projects

On the owner's `claude` and `codex`, macOS and Linux, Windows where it runs; answers go in this
step's status and W5 follows them.
1. `--add-dir`: `Read`, `Grep`, `Glob` there ask nothing; `Edit` asks in `default`, not in
   `acceptEdits`.
2. `Edit`, `Write`, `NotebookEdit` denies on an added dir hold in every mode, `bypassPermissions`
   included.
3. An added dir's `.claude/settings.json` does not load; its `.claude/skills` do or do not; its
   `CLAUDE.md` loads with the variable.
4. The sandbox with writes allowed but on the read-only roots: `touch`, `echo >`, `git switch` and
   `git stash` there fail; `cargo build`, `npm install` and `git commit` in a read-write project
   work; network asks reach the host or pass.
5. `--resume` with the same flags keeps rows 1–4.
6. Codex: a writable root is written, a read-only project is read and not written.

Done when every row has an answer per OS, or a fallback the owner accepts.

### Step W1. Open in Zed out

Work: §9. Tests: `tests/files.rs`: the header has no `zed` button. Done when it is gone and
KEYMAP matches. Independent of the rest.

### Step W2. The workspace as context

Work:
1. Core: `id`, `readOnly`, `list_repos(ws)`, the folder calls by id, the boot migration (§10).
2. Kernel: `Workspace::{id, name, role, switch}`, `workspace.current`, the current repo per
   workspace.
3. The top bar's switcher; the sidebar shows the current workspace; rename and delete by id.

Tests: core: ids written once and stable, `active` to current, marks from name to id, an unknown
key kept; `list_repos(ws)` and roles by folder and linked worktree. Kernel: a switch re-lists and
emits once, restores that workspace's current repo. `tests/repos.rs`: the switcher swaps the
sidebar; a rename keeps the id; a delete leaves the projects.

Done when the owner's workspaces carry over and switching shows each one's projects alone.

### Step W3. Tabs by category

Needs W2. Work: §8 with the option the owner picks.

Tests, `tests/dock.rs`: a file and a chat open in different categories; a category shows its tabs
alone; each category's preview tab; the four actions; a restart restores each workspace's
categories; a switch swaps the center; an old `dock.layout`'s center loads into categories; every
registered kind has a category.

Done when the owner's `align-2-0` chat and `biome.json` no longer share a strip.

### Step W4. Read-only projects

Needs W2. Work: §3 and §6's read-only rules; attaching read-only from Add folder… and the project
row's menu.

Tests: core: each write refused in a read-only attachment and allowed where the same repo is
read-write; the ff-only pull skips a dirty and a diverged tree. Kernel: keep-fresh runs on GPUI's
clock against a local remote. `tests/repos.rs`: menus and the VCS popup lack writes, a chord's
refusal says why, the Processes app lists no read-only project.

Done when the owner's knowledge-base repo stays fresh with nothing but fetch and pull.

### Step W5. The workspace chat

Needs W0, W2, W4. Work: §4 and §7's chat archive.

Tests, on `fake_claude`: the args carry `--add-dir` per project and the denies per read-only one;
a resume repeats them; the sandbox only with a read-only project; `bypassPermissions` refused on
Windows with one; snapshots of read-write projects alone; an untrusted project's skills off; an
attachment change resumes at the turn's end; the read scope is the chat's workspace; an archived
chat opens without a composer; a workspace's delete removes its transcripts. `make agent-test`
gains one row: an edit into a read-only project is refused by the real `claude`.

Done when the owner chats across his workspace's projects and the knowledge base cannot be
written.

### Step W6. Council per workspace

Needs W2 and W4; after the Council rename. Work: §5 and §7's run archive.

Tests: records under the workspace; a run of a project in two workspaces lists in its own alone; a
read-only subject runs on an export with no worktree or branch and offers no Fix; marks by id; an
old run opens read-only and a kept finder branch still merges.

Done when the owner runs a review from a workspace and finds his old runs in the archive.

## 13. Parallel work

- **ui/polish** (presentation, `crates/app`) touches `dock/skin.rs`, `dock/doc.rs`, `chat/`,
  `repos/sidebar.rs`, `repos/workspaces.rs` and `files/viewer.rs`. W0 and W2's core and kernel
  parts run beside it; W1, W2's UI and W3 rebase on it after it merges.
- **The Council rename** goes before W6, so W6 writes the new names once. W3's category table
  names kinds by string and takes the new ones in the rename's commit.
- W3 and W4 can run in parallel after W2; W5 waits for W0.

## 14. Risks

- **The chat in a sandbox** may break toolchains or ask about the network; W0 row 4 decides, the
  fallback is Windows' rule on every OS.
- **`--add-dir` semantics** move between CLI versions (skills, `CLAUDE.md`); `min_cli` pins what
  W0 saw.
- **Lost project settings** in a workspace chat: hooks, MCP servers and allow rules (Q3).
- **Windows:** Bash can write a read-only project; asks and a card mitigate, nothing enforces.
- **The kit's `set_center`** with kept tab entities is unproven; W3 starts with that test.

## 15. Open questions for the owner

1. One current workspace (proposed), or several shown at once as today?
2. Tabs: Option A (proposed) or B; the category names; whether every Markdown file is a Doc.
3. Should a chat also start "in a project", its cwd, so that project's settings, hooks and MCP
   servers load? Proposed: not in W5.
4. A read-only project's MR as a Council subject needs a fetch into `refs/detached/subjects/`. Is
   that a fetch you allow (proposed), or are such MRs refused?
5. Open in Zed: the header button and the action (proposed), or every place?
6. Windows Bash in a workspace with a read-only project: the stated limit (proposed), or no Bash
   at all there?
7. Deleting a workspace deletes its chats and runs (proposed), or keeps them in the archive?
8. Archived chats: read-only (proposed), or resumable in their old cwd?
9. Keep-fresh every 15 minutes (proposed), or folded into the backlog's Auto-fetch now?
