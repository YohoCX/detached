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

**Status (2026-11-18): the owner's answers in, plan review round 01 fixed (10 major, 5 minor), and
the owner's answers to its questions in; waiting for the next plan review.** What the owner
decided is in the [decision](docs/decisions/2026-11-18-workspaces.md); choices marked *Proposal*
are this plan's own, for review.

Owner's decisions, 2026-11-18:
1. The workspace is the top-level context, and the window shows one at a time. Chats and Council
   runs belong to a workspace; a chat's agent works across the workspace's projects.
2. One project can be attached to several workspaces.
3. A project can be attached read-only: read, search, Docs, and fetch/pull to keep it fresh.
   Nothing else writes to it, enforced for agents, not only hidden in the UI.
4. Start clean: per-project chats and Council runs are not migrated; they stay readable in an
   archive, read-only. A deleted workspace's chats and runs go there too. Clearing the archive
   never touches a transcript Detached did not start.
5. Document tabs by category, as category chips at the start of one strip.
6. A chat can be started inside one project, so that project's own context loads.
7. Open in Zed is removed everywhere.
8. Third-party MCP servers are off in every Detached chat and Council run; Detached's own server
   stays. Agents are extended through skills with scripts. Hooks load only from the user's own
   settings and repos the owner accepted.
9. Codex with a read-only project: `read-only` and `workspace-write` only, never pinned to one.
   A trusted Council run beside a read-only project is refused where there is no sandbox.

Words: a **project** is a repo the workspace lists (today's `RepoInfo`); an **attachment** is a
project in a workspace, with a **role**, read-write or read-only. Council is today's Panels; its
code rename is separate (§13).

## 1. The model

*Proposal.* `workspaces` stays core's known key; `Workspace` gains `id`, `readOnly` and a
flattened map of unknown fields, so a later build's fields survive this one's saves:

```json
"workspaces": [
  {"id": "3f9a2c1e", "name": "Detached", "folders": ["/Users/me/dev/personal"],
   "excludedRepos": [], "readOnly": ["/Users/me/dev/kb"], "active": true}
]
```

- **Attachments are today's folders.** A folder (a repo, or a folder of repos discovered as now)
  minus `excludedRepos`. A repo two workspaces list is attached to both: many-to-many with no new
  table.
- **Role.** Read-only when the repo's path or an ancestor folder is in `readOnly`; a linked
  worktree takes its repo's role; else read-write. The same repo can be read-only in one workspace
  and read-write in another.
- **Identity is `id`**, 8 random hex, made once. Chats, runs, tabs, trust marks and ui_state keys
  name the id; the name is a unique label, kept also in `data/workspaces/<id>/name`.
- **Rename** changes the name alone. **Delete** removes the settings entry and nothing else: its
  folder stays, so its chats and runs show in the archive (§7). Its projects are not touched.
- **A project in no workspace** is not listed, as today; the archive keeps its old items (§7).
- Core: `list_repos(ws)`; `RepoInfo.read_only`; `set_read_only(ws, path, on)`; the folder calls
  take the id.

## 2. One current workspace

The window shows one workspace: its projects in the sidebar and the repo switcher, its chats, its
Council runs, its tabs. The top bar's Workspaces widget, which turns several on and off today,
becomes a switcher.

- The current id is `ui_state` `workspace.current`; the first workspace when it is unset or gone.
  The current repo is remembered per workspace (`workspace.repo.<id>`).
- Kernel: `Workspace` gains `id()`, `name()`, `role(repo)` and `switch(id)`, which re-lists the
  repos, restores the workspace's current repo and emits `Switched` once. The dock's center, the
  Chats app and the Council app follow it.
- **A switch stops nothing, so nothing live reads the current list.** A session keeps its own
  projects and roles from its open (§4.3); its snapshots, `proc`, `watch` and the read tool use
  them, not `Workspace::repos()`. `Workspace` keeps a `Repo` entity for every repo a live session
  or run holds, listed or not (`hold`/`release`, `Workspace::repo` finds both), so statuses stay
  readable. Processes and terminals belong to projects (§6).

## 3. Read-only projects

- **Detached reads everything:** tree, files, diffs, log, MRs, pipelines, find, Docs.
- **Keeps it fresh:** fetch, then `pull --ff-only` when the worktree is clean and the branch
  tracks; otherwise the pull is skipped and the project's row says why. At a switch to the
  workspace, every 15 minutes while the window shows, and in Fetch All and Pull All.
- **Allows one more fetch:** a Council run's MR head into `refs/detached/subjects/<run>` (§5).
- **Refuses every other write** of CLAUDE.md's list: switch and checkout, new and delete branch,
  stash, prune, managed worktrees and recipes, chat snapshots and restores, Council finder
  branches, pinned worktrees and Fix. Processes do not start (§6).
- **In core, not only in the UI.** Each write entry point takes the workspace id of the session,
  run or view it acts for and refuses a read-only attachment: "`kb` is read-only in this
  workspace".
- **UI:** a lock on the project's row and in the file and diff headers; the branch widget shows
  the branch without its popup; write items leave menus and the VCS popup; a chord for one says
  why through Activity.
- **The user's own terminal is not limited:** what the owner types is the owner's.
- Read-only ships in one step with its enforcement for chats and Council (W7), never before it.

## 4. The workspace chat

### 4.1 Where it runs

*Proposal.* A chat for the whole workspace runs in `data/workspaces/<id>/chat/`, a folder
Detached makes and keeps empty. Why: the CLI keeps transcripts by cwd, so one folder per workspace
lists its chats (§4.7); no project is primary; a rename moves nothing. Its cwd has no settings; a
chat in one project (§4.2) loads that project's.

### 4.2 A chat in one project

- **Pinned at creation.** The Chats app's New chat has a menu: "In the workspace" (the default)
  and each project of the workspace; a project row's menu has New chat here. `agent.chat` starts
  one in the current repo, as its KEYMAP line says today; `agent.chatWorkspace` starts a workspace
  one. The pin never changes: a resume runs where the chat started.
- **cwd is that project**, so its `CLAUDE.md` and, once its trust files are accepted, its
  `.claude/settings.json` (hooks, allow rules) load as in today's chat: `--setting-sources
  user,project,local` when accepted, `user` until then, with today's banner. Its MCP servers do
  not (§4.8).
- **The other projects are still attached** (§4.3) with their roles; their settings do not load.
- **Pinned to a read-only project:** Claude's chat denies every write to it, the cwd included
  (§4.4); Codex's is refused.
- The tab's title names the project; the Chats app lists it with the workspace's other chats.

### 4.3 The projects given to the agent

- `Spec` gains `dirs: Vec<Dir { path, read_only, trusted }>`, every attached project but the cwd,
  and `instructions: String`: the workspace's name, each project's path and role, the path of each
  trusted project's `CLAUDE.md`, and "read-only projects: read and search only; never write,
  switch, stash or commit there". `Spec.repo` stays the cwd. No protocol word; the backend carries
  the text (W0 row 7). The kernel's session keeps this list for its life (§2); a change of
  attachments or roles ends the session at its turn's end, and the next message resumes with the
  new list, as a trust change does (host §4).
- **Claude:** `--add-dir <path>` for each **trusted** project, read-only ones included, so `Read`,
  `Grep` and `Glob` there do not ask. An untrusted project gets no `--add-dir`, so the CLI loads
  none of its `CLAUDE.md`, skills, agents or commands; `Read`, `Grep` and `Glob` rules on its path
  in `--allowedTools` let the agent read it unasked. *Proposal:* no
  `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD`: the instructions name the trusted projects'
  `CLAUDE.md` files and the agent reads them.
- **Codex:** read-write projects as additional writable roots (W0 names the adapter's field).
  Read-only ones are not added: `workspace-write` reads the whole disk and writes its roots alone.

### 4.4 Read-only, enforced

A chat whose workspace has a read-only project:
- **Claude's file tools, every OS:** `Edit`, `Write` and `NotebookEdit` deny rules on
  `//<project>/**` and its git common dir, in `--disallowedTools`; for a pinned read-only project,
  on the cwd too. A deny wins over `--add-dir` and every mode (W0 row 2).
- **Claude's Bash, macOS and Linux:** Claude's sandbox with `denyWrite` on the read-only projects
  and their git dirs, writes elsewhere allowed (the cwd too, unless it is read-only),
  `allowUnsandboxedCommands: false` and `excludedCommands: []`. The seam changes for it:
  `backend::Sandbox` no longer denies the cwd by itself (untrusted participants and verifiers list
  it), and it gains `auto_bash`, the `autoAllowBashIfSandboxed` flag, false for a chat so its Bash
  asks as today (host §5's "What asks"). Network asks reach the owner as asks. The "sandbox ready"
  probe must pass.
- **Claude's Bash without a sandbox** (native Windows, or a failed probe): only `default` and
  `plan` modes, and each Bash ask's card says the shell can write the read-only projects. Not
  enforced: a command the owner allows, or his allow rules allow, runs unsandboxed. Accepted by the
  owner as a limit; a TECHDEBT row (W7).
- **Hooks** of the user's settings and of accepted repos still load and can write a read-only
  project: the owner's own code, a stated limit, not enforced. So can user or project
  `excludedCommands` if a flag's empty list does not replace theirs (W0 row 5).
- **Codex:** only `read-only` and `workspace-write`; `agent` (its auto-reviewer approves
  escalations unseen) and full access are off. A Codex chat pinned to a read-only project is
  refused, since its cwd is always a writable root.
- **Host tools:** `detached_act` has no repo write; `proc` refuses a read-only project's processes.

### 4.5 Trust and skills

- Acceptance stays per repo and provider, against its trust files' hash. Only the cwd's settings
  can load (§4.2); an added project gives skills, agents and commands, and only when trusted
  (§4.3).
- **Skills:** each trusted project's skill folders join the folder list; an untrusted project's
  never reach the CLI. *Proposal:* a name two projects share loads from the pinned project, else
  from the one attached first; the Skills page lists the other as shadowed. Codex reads the rest
  through `detached_read skills`, as now.
- The chat's start screen lists each untrusted project with its Trust… button, where today it has
  one banner. Accepting one mid-session ends the session at its turn's end, as above.
- The watcher holds every attached project's trust paths while the session lives.

### 4.6 The read tool

A chat reads its own workspace, not the current one: `Workspace { id name repos current }`,
`Repo.readOnly: Boolean!`, `sessions(workspace: ID)`. A Council participant's scope is unchanged.

### 4.7 History, list, resume, delete

- The host's index (`sessions.json`) gains `workspace` and `cwd` per chat it starts; it already
  records every session the host started (`sessions::seen`).
- A workspace's chats are the transcripts of its own folder (`past(cwd)`) and, for chats in one
  project, the transcripts the index names with this workspace. Other transcripts in a project's
  CLI folder are the archive's (§7).
- Resume passes the same `Spec`, `dirs` included; delete and the archive flag are as now. Codex:
  `session/list` narrowed to the host's index.
- `Doc::Chat { workspace, n }`; the chats global is keyed by `(workspace, n)`; the Chats app lists
  the current workspace's open and past chats.
- **Snapshots** cover the read-write projects of the session's own list; never a read-only one.

### 4.8 MCP servers and hooks

Every Detached session, chat and Council alike:
- **Detached's own MCP server alone:** the host's tools (the read tool, `detached_act`, Council's
  report tools). Third-party servers, the user's and the projects', do not load. Why: they run
  outside the sandbox and the denies, cost context tokens on every turn, and skills with scripts
  cover the use, under Bash's sandbox, denies and asks.
- **Claude:** `--strict-mcp-config` on every spawn, the chat's included, which keeps the
  session's own `sdkMcpServers` (host §7) and drops every other server. Setting sources stay as
  today, so the user's and accepted repos' hooks and allow rules still load. If W0 row 5 shows the
  account's claude.ai connectors still join, `core::claude` turns them off through the CLI's
  variable for it. Council's 0600 file of a repo's MCP servers in `data/private/<run>/` and
  `Spec.mcp` go.
- **Codex:** no server in the user's `config.toml` loads: a per-session override of
  `mcp_servers` if the adapter takes one (W0 row 8), else the host-owned `CODEX_HOME` beside a
  copy of the login that Council's Codex participants already use.
- **Hooks:** from the user's own settings and repos the owner accepted; never from an untrusted
  repo, as today.
- This changes today's chat: its third-party MCP servers go away. It ships early, in a step of its
  own (W2), with a release-notes line.

## 5. Council

- A run belongs to a workspace; its subject names a project's branch, MR, issue or document.
- *Proposal:* records go to `data/workspaces/<id>/council/<task>/<run>/`, not
  `<git common dir>/detached/panels/`. Why: the run is the workspace's; a project in two
  workspaces would show it in both; a read-only project's git dir takes no record. This amends
  [storage scopes](docs/decisions/2026-11-12-storage-scopes.md) and
  [the git-dir store](docs/decisions/2026-11-12-write-git-dir-store.md): run records stay local,
  per workspace. Subject refs, finder branches, pinned worktrees and Fix stay in the subject's repo.
- revmux imports (`.revmux/`) stay per project, under the project in the workspace's Council app.
- **Trust marks by id**, in the same step that moves runs to workspaces (W6): `panel::marks`
  rewrites its own block from names to ids at its first load, matches by id, and shows the
  workspace's name in its reasons.
- **A read-only subject** runs as an untrusted subject does today, whatever its marks: sandboxed,
  read-only participants on an export, no worktree, no finder branches, no Fix; without the
  unisolated consent, so it refuses where the sandbox does not run (native Windows until WSL runs
  are built). Its MR's head is fetched into `refs/detached/subjects/<run>` and removed with the
  run, as for any MR.
- **A trusted run in a workspace with a read-only project:** its finders and Fix keep orchestrator
  mode in their own trees, with §4.4's file-tool denies and a sandbox that denies writes to the
  read-only projects (`auto_bash` true, as a finder asks nothing). Where the sandbox does not run,
  such a run is refused.
- `Doc::Panel`, `PanelTask` and `PanelStats` take `workspace` in place of `repo`; stats read the
  workspace's runs.

## 6. Processes and worktrees

- *Proposal:* **processes stay per project.** Definitions are the project's store items and a run
  belongs to its project, running on across a switch. The Processes app lists the current
  workspace's read-write projects, grouped, where today it lists the current repo's. A read-only
  project's processes are neither listed nor started, by the owner or an agent: they run repo code
  that writes.
- A chat's `proc` reaches its own read-write projects' processes at their `agents` level.
- Managed worktrees stay per project (`<folder>/worktrees/<repo>/<branch>`), shown in every
  workspace that lists it; none for a read-only one.

## 7. The archive

Read-only, in a collapsed Archive section at the end of the Chats app and of the Council app,
the same in every workspace. Nothing is moved or copied.

- **Its places:** `data/archive.json`, a list of repo and folder paths. The migration writes
  every repo and folder the settings list then; a folder a workspace drops and the folders of a
  deleted workspace are added. So a project in no workspace keeps its items there.
- **What it holds:**
  - past chats of those places, as today's Chats app lists them, less those the index gives a
    workspace;
  - runs under those places' `<git common dir>/detached/panels/`;
  - a deleted workspace's chats and runs, under its name: the folders under `data/workspaces/`
    whose id the settings no longer list, and the index's chats naming that id.
- **Chats** open as `Doc::PastChat { provider, cwd, id }` (kind `chat-archive`): the transcript,
  no composer, no resume.
- **Runs** open with rounds, findings and outcomes, no next round and no Fix. A kept finder branch
  keeps Merge and Discard in a project read-write in the current workspace, so none is stranded.
- **Clear:** per item, and Clear all per section, each after a confirm with the counts.
  - A chat's clear is its delete (transcript and index entry), and only for a session the host's
    index records. A transcript the owner made with `claude` in a terminal, or any the index does
    not know, is listed with "not started by Detached" and never cleared. A chat live in Detached
    is not cleared.
  - A run's clear removes its record. A record in a repo read-only in any workspace is not
    cleared; Clear all skips it and says how many stayed.
  - A deleted workspace's folder goes with its last item.

## 8. Tabs by category

- `dock::register_category(cx, id, title, order)`; `DocKind::category(&self, doc) -> &'static str`
  with no default, so a new kind names one; a test fails on a kind whose category is not
  registered.

  | Category | Kinds |
  | --- | --- |
  | `docs` "Docs" | `file` of a Markdown path (`.md`, `.mdx`, `.markdown`) |
  | `chats` "Chats" | `chat`, `chat-archive`, `panel`, `panel-task`, `panel-stats` |
  | `code` "Code" | `file` otherwise, `diff`, `mr`, `job`, `terminal` |

- **One strip, chips first.** Three chips with counts, drawn by our `skin` at the start of the
  center's strip; the strip shows the chosen category's tabs alone.

  ```
  ┌────────────────────────────────────────────────────────────────────┐
  │ Docs 2 │▌Chats 3▐│ Code 5 ┃ align-2-0 × │ review: ws-plan × │        │
  ├────────────────────────────────────────────────────────────────────┤
  │            the Chats category's active tab                         │
  └────────────────────────────────────────────────────────────────────┘
  ```

- Built on the kit's `DockArea::set_center` with each category's layout, its tab entities kept,
  so a switch rebuilds no view. A category can still split the center by dragging a tab to an
  edge; side zones are unchanged, and a tab dragged to one leaves its category.
- Each category has its own tabs, active tab and preview tab: a file from the tree replaces the
  code preview, a plan from Docs the docs preview. `show_doc` opens a doc in its category and
  brings the category forward.
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

## 9. Open in Zed

Removed everywhere:
- the file and diff headers' `viewer::zed_button` (`files/file.rs`, `files/diff.rs`);
- the conflict bar's button (`viewer::conflict_bar` keeps its text);
- the conflict notice's button and `open_in_zed` in `notices.rs`;
- the sidebar menu's item (`repos/sidebar.rs`);
- the `file.openInZed` action (`actions.rs`, `files/viewer.rs`) and its docs/KEYMAP.md row;
- the code left dead: `viewer::open_in_zed`, the views' `zed` methods, `api::open_in_zed`,
  `git::files::open_in_zed` with `ZED_CLI` and its test, and that test's mention in
  docs/TECHDEBT.md's Unix-only row.

## 10. Migration

- **Workspaces carry over.** At boot, one `update` gives every workspace without an `id` one: the
  id of the `data/workspaces/<id>/name` holding its name when there is one, else a new one.
- **`active` stays** as it is, read and written but unused after the migration, so an older build
  still lists repos. The first active workspace becomes `workspace.current`.
- **Chats and runs are not migrated**; the archive shows them (§7).
- **The saved layout:** the center's tabs of the old `dock.layout` load once into the current
  workspace's categories; old `Doc::Chat` and `Doc::Panel` tabs, keyed by repo, are dropped.
- **A stated limit:** a build older than this drops `id` and `readOnly` on its next save. Back on
  this build, ids are found again by name, but read-only roles are gone: the projects are
  read-write until set again. The release notes say so.

## 11. Written rules this changes

- CLAUDE.md: the write list says a read-only attachment takes only fetch, pull and the fetch of an
  MR's head; Native UI gains the tab categories.
- PLAN-FOUNDATION §3 and PLAN-NATIVE Contracts: `Workspace::switch`, `id`, `role`, `hold`;
  `dock::register_category`, `DocKind::category`; PLAN-NATIVE's File view loses Open in Zed.
- PLAN-AGENT-HOST:
  - §1's flag table (the MCP config file row goes) and §2's "MCP servers" row (every session is
    strict: the host's server alone);
  - §5 (the chat's sandbox and modes with a read-only project; the seam's `Sandbox`);
  - §7 (a chat reads its workspace; `--strict-mcp-config` now on every spawn);
  - §10 (run records), §11 (skills of several projects);
  - §13 (extra folders are the workspace's trusted projects; the System prompt row gains the
    workspace's instructions; the Untrusted repo row covers attached projects);
  - H4c's test of setting sources and strict MCP per kind.
- PLAN-PANELS §3 (read-only subjects; trusted runs beside read-only projects; "The chat's
  configuration in its worktree" loses the repo's MCP file), §10 (where run records live), and
  P2's and P4's tests of the MCP file.
- Decisions [storage scopes](docs/decisions/2026-11-12-storage-scopes.md) and
  [git-dir store](docs/decisions/2026-11-12-write-git-dir-store.md): run records per workspace.
- docs/KEYMAP.md: the tab rows and `agent.chatWorkspace`; `file.openInZed` goes.
- docs/TECHDEBT.md: the fake-zed test leaves the Unix-only row (W1); the Windows Bash row (W7).
- README: workspaces, read-only projects, no third-party MCP servers in Detached's agents.

## 12. Steps

Each step passes the host plan's gate: `make check`, a revmux round without blocking findings, the
owner's check on macOS. Docs of §11 change in the step that makes them true. Read-only projects
come last but one (W7), with every enforcement they need, so no build ships them unenforced.

### Step W0. Spike: agents across projects

On the owner's `claude` and `codex`, macOS and Linux, Windows where it runs; answers go in this
step's status, and W2, W5, W7 and W8 follow them.
1. `--add-dir`: `Read`, `Grep`, `Glob` there ask nothing; `Edit` asks in `default`. A path not
   added, with `Read`/`Grep`/`Glob` allow rules, is read unasked.
2. `Edit`, `Write`, `NotebookEdit` denies on an added dir and on the cwd hold in every mode,
   `bypassPermissions` included; `acceptEdits`' auto-approved commands (`rm`, `mv`, `touch`) on a
   denied added dir: run or ask.
3. What an added dir loads: settings (expected none), skills, agents, commands, `CLAUDE.md`.
4. The sandbox with `autoAllowBashIfSandboxed: false`, writes allowed but on the read-only roots:
   `touch`, `echo >`, `git switch` and `git stash` there fail; `cargo build`, `npm install` and
   `git commit` in the cwd and a read-write project work; Bash still asks in `default`; network
   asks reach the host.
5. `--strict-mcp-config` with `--setting-sources user,project,local`, against a user config with
   servers in `~/.claude.json`, in user settings and a trusted repo's `.mcp.json` (with
   `enableAllProjectMcpServers`), a logged-in account with claude.ai connectors, and hooks in user
   and project settings: `initialize` lists no server but Detached's, whose tools answer; both
   hooks run; a flag's `excludedCommands: []` replaces or merges the user's list.
6. `--resume` with the same flags keeps rows 1–5.
7. How each backend takes `instructions`: Claude's appended system prompt, Codex's developer
   instructions; both survive a resume.
8. Codex: a writable root is written, a read-only project is read and not written, in
   `workspace-write` and `read-only`; a per-session override of `mcp_servers` drops the user's
   servers, or does not.

Done when every row has an answer per OS, or a fallback the owner accepts.

### Step W1. Open in Zed out

Work: §9. Tests: `tests/files.rs` asserts no `viewer.zed` in the header and no Zed button in the
conflict bar; `tests/repos.rs` the sidebar menu without the item; the action is unknown to the
palette. Done when `rg -i 'open in zed|open_in_zed|openInZed' crates docs` finds nothing outside
decisions and KEYMAP matches. Independent of the rest.

### Step W2. Third-party MCP servers off

Needs W0 rows 5 and 8. Work: §4.8 on today's chat and Council; the README and the release notes
say third-party MCP servers no longer load in Detached's agents, and skills with scripts replace
them.

Tests: on `fake_claude`, every spawn and resume carries `--strict-mcp-config`, the chat's in a
trusted repo with `user,project,local`; no `--mcp-config` and no file in `data/private/`; a
trusted repo's hook still runs. On the fake ACP agent, a Codex session gets no user server.
`make agent-test` gains one row: a user server in `~/.claude.json` is not listed by the real
`claude`, and the host's read tool answers.

Done when the owner's chat lists Detached's server alone and his skills still run.

### Step W3. The workspace as context

Work:
1. Core: `id`, `readOnly` (stored, no role yet), the flattened map, `list_repos(ws)`, the folder
   calls by id, the boot migration (§10).
2. Kernel: `Workspace::{id, name, switch, hold, release}`, `workspace.current`, the current repo
   per workspace. A session keeps the repos it works in from its open; `begin`'s snapshots,
   `proc::act`, `inbox::watch` and the host's snapshot read them, not the current list.
3. The top bar's switcher; the sidebar shows the current workspace; rename and delete by id.

Tests: core: ids written once and stable, found again by name, `active` kept, an unknown key in
a workspace kept; `list_repos(ws)`; a delete keeps the folder. Kernel: a switch re-lists and emits
once, restores that workspace's current repo; a chat woken after a switch snapshots its own repos
and no other, and its `proc` and `watch` still find them. `tests/repos.rs`: the switcher swaps the
sidebar; a rename keeps the id; a delete leaves the projects.

Done when the owner's workspaces carry over, switching shows each one's projects alone, and a
running chat goes on across a switch.

### Step W4. Tabs by category

Needs W3. Work: §8.

Tests, `tests/dock.rs`: a file and a chat open in different categories, a Markdown file under
Docs; a category shows its tabs alone, the chips their counts; each category's preview tab; the
four actions; a restart restores each workspace's categories; a switch swaps the center; an old
`dock.layout`'s center loads into categories; every registered kind has a category; a switch of
category keeps a file's scroll position.

Done when the owner's `align-2-0` chat and `biome.json` no longer share a strip.

### Step W5. The workspace chat

Needs W0, W2 and W3. Work: §4 for a workspace with no read-only project: §4.1, §4.3, §4.5–§4.7;
§7 for chats, with `data/archive.json`.

Tests, on `fake_claude`: the args carry `--add-dir` per trusted project and read rules per
untrusted one, and the instructions; a resume repeats them; an untrusted project's skills,
agents and commands never reach the CLI; snapshots of the session's projects alone; an attachment
change resumes at the turn's end; the read scope is the chat's workspace; an archived chat opens
without a composer; a deleted workspace's chats show in the archive; Clear and Clear all after
their confirms; a transcript the index does not know is listed and never cleared.

Done when the owner chats across his workspace's projects and finds his old chats in the archive.

### Step W6. Council per workspace

Needs W3; after the Council rename. Work: §5 but its read-only rules; §7 for runs.

Tests: records under the workspace; a run of a project in two workspaces lists in its own alone;
marks rewritten to ids by `panel::marks` and a repo trusted by its workspace's mark still trusted;
an old run opens read-only and a kept finder branch still merges; a deleted workspace's runs in
the archive.

Done when the owner runs a review from a workspace and finds his old runs in the archive.

### Step W7. Read-only projects

Needs W0, W5, W6. Work: §3, §4.4, §5's read-only rules, §6's; attaching read-only from Add
folder… and the project row's menu; docs/TECHDEBT.md gains the Windows row: without a sandbox,
Bash in a chat with a read-only project can write it; `default`/`plan` only and the ask card
mitigate, nothing enforces (§4.4).

Tests: core: each write refused in a read-only attachment and allowed where the same repo is
read-write; the ff-only pull skips a dirty and a diverged tree. Kernel: keep-fresh runs at a
switch and every 15 minutes on GPUI's clock against a local remote. On `fake_claude`: the denies
per read-only project; the sandbox with `auto_bash` false, the cwd writable and
`excludedCommands` empty; `bypassPermissions`, `acceptEdits` and `auto` refused without a
sandbox. Codex: `agent` and full access refused, a pinned read-only project refused. Council: a
read-only subject runs untrusted with no Fix, its MR's head fetched and removed after; a trusted
run beside a read-only project carries the denies and the sandbox, and is refused without one.
`tests/repos.rs`: menus and the VCS popup lack writes, a chord's refusal says why, the Processes
app lists no read-only project. `make agent-test` gains one row: an edit and a `git switch` into a
read-only project are refused by the real `claude`.

Done when the owner's knowledge-base repo stays fresh with nothing but fetch and pull, and no chat
or run can write it but through a hook.

### Step W8. A chat in one project

Needs W7. Work: §4.2; `agent.chatWorkspace`.

Tests, on `fake_claude`: a chat pinned to a project runs there with `project` settings once
accepted and `user` before, strict MCP, the other projects attached with their roles; a pinned
read-only project denies writes to the cwd; a Codex chat pinned to one is refused; a resume keeps
the pin; the workspace lists its pinned chats and not the project's other transcripts, which the
archive shows; a trust change of the pinned project ends the session at the turn's end.

Done when the owner's chat in a project runs that project's hooks and still sees the workspace's
other projects.

## 13. Parallel work

- **ui/polish** (presentation, `crates/app`) touches `dock/skin.rs`, `dock/doc.rs`, `chat/`,
  `repos/sidebar.rs`, `repos/workspaces.rs`, `notices.rs` and `files/viewer.rs`. W0, W2 and W3's
  core and kernel parts run beside it; W1, W3's UI and W4 rebase on it after it merges.
- **The Council rename** goes before W6, so W6 writes the new names once. 0.6.0 shipped "Panels",
  so the rename migrates what an installed copy holds, with no alias kept after it:
  - the settings block `panels` (`panel::marks::BLOCK`), moved to its new key at load;
  - the store's roles and modes, if their folders or titles change, moved in every scope;
  - the action id `panel.panels`, rewritten in `keymap.json` overrides;
  - the dock app's saved name `panels` and the doc kinds `panel`, `panel-task`, `panel-stats` in
    `dock.layout`, so the layout loads without a placeholder.
  W4's category table names kinds by string and takes the new ones in the rename's commit.
- W4 runs beside W5 and W6 after W3.

## 14. Risks

- **The chat in a sandbox** may break toolchains; W0 row 4 decides, the fallback is the
  no-sandbox rule on every OS.
- **No third-party MCP servers** in Detached's agents: a workflow that needs one moves to a skill
  with a script, or to the owner's own terminal `claude`.
- **Hooks** of the owner's settings and accepted repos can write a read-only project; stated.
- **`--add-dir` semantics** move between CLI versions; `min_cli` pins what W0 saw.
- **Windows:** without a sandbox, Bash in a chat with a read-only project can write it; the owner
  accepted it; TECHDEBT row from W7.
- **The kit's `set_center`** with kept tab entities is unproven; W4 starts with that test.
