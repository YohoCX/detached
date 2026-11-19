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

**Status (2026-11-19): accepted by the owner (a workspace is its own context for everything: tabs, chats, Council, processes); W0–W4 started.** After
round 04 the owner set read-only's boundary: it stops a cooperative agent's accidental writes, not
a deliberate escape. What the owner decided is in the
[decision](docs/decisions/2026-11-18-workspaces.md); choices marked *Proposal* are this plan's.

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
   A trusted Council run beside a read-only project is refused where there is no sandbox. The
   accepted holes are Bash on native Windows and the owner's own hooks.
10. Beside a read-only project, a chat or trusted run refuses on macOS and Linux where Claude's
    sandbox does not run, showing the fix, and refuses when settings name
    `sandbox.excludedCommands`, naming the file. Codex runs only if it can drop its MCP servers
    per session. A chat `--add-dir`s the workspace's trusted read-write projects; read-only and
    untrusted ones are files only.
11. (2026-11-19) Read-only protects against a cooperative agent's accidental writes, not against
    an agent deliberately escaping: planting git config or hooks (`core.hooksPath`, submodule git
    dirs, `~/.gitconfig`), settings that widen the sandbox, a process Detached runs. Untrusted
    code goes through Council's untrusted path (sandboxed, an export, no Fix), the real
    isolation.

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
  name the id; the name is a unique label. `data/workspaces/<id>/name` keeps it for the archive.
- **Rename** changes the name alone. **Delete** is refused while something of it runs: a chat's
  turn, a Council run, or a docked terminal tab; the message names them. Otherwise, after a
  confirm, the kernel ends each of its chats (`chat::end`: the session closes, wake goes off),
  open tab or not, then closes its tabs and drops `tabs.<id>` and the settings entry. Its folder stays, so its chats and runs show in the archive
  (§7). Its projects and their processes are not touched.
- **A project in no workspace** is not listed, as today; the archive keeps its old items (§7).
- Core: `list_repos(ws)`; `RepoInfo.read_only`; `set_read_only(ws, path, on)`; the folder calls
  take the id. The untrusted deny list keeps every workspace's repos (`all_repos()`), and Fix takes
  its managed-worktree folder from the run's own workspace.

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
  workspace, every 15 minutes while the window shows, and in Fetch All and Pull All. Never while a
  live session or run holds the repo read-write, in any workspace: the pull waits for the next
  tick.
- **Allows one more fetch:** a Council run's MR head into `refs/detached/subjects/<run>` (§5).
- **Refuses every other write to its tree or git dir**, git's or Detached's: switch and checkout,
  new and delete branch, stash, prune, managed worktrees and recipes, chat snapshots and restores,
  Council finder branches, pinned worktrees and Fix, and the repo store's writes (a new or edited
  item, Share and Make local, the sharing default: `RepoStore::write`, `move_item`,
  `set_default`). Its store items are listed, not edited. Processes do not start (§6).
- **In core, not only in the UI.** Each write entry point takes the workspace id of the session,
  run or view it acts for and refuses a read-only attachment: "`kb` is read-only in this
  workspace".
- **UI:** a lock on the project's row and in the file and diff headers; the branch widget shows
  the branch without its popup; write items leave menus and the VCS popup; a chord for one says
  why through Activity.
- **The user's own terminal is not limited:** what the owner types is the owner's.
- **Agents** beside a read-only project: §4.4 and §5, against accidental writes (decision 11).
  Read-only ships in one step with that enforcement (W7), never before it.

## 4. The chat

### 4.1 Where it runs

- **A workspace chat** runs in a folder of its own, `data/workspaces/<id>/chats/<chat>/`, `<chat>`
  an id Detached makes at creation (*Proposal*). Detached makes it empty then and never empties
  it: it is that chat's scratch, kept with it and removed with its delete; the instructions say
  so. Why: the CLI keeps transcripts by cwd, so the workspace's chats are its folders' transcripts;
  no project is primary; a rename moves nothing; no chat touches another's files.
- **A chat in one project** runs in that project. Chosen at creation from New chat's menu ("In
  the workspace", the default, or a project) or a project row's New chat here; `agent.chat` starts
  one in the current repo, as its KEYMAP line says, and `agent.chatWorkspace` a workspace one. The
  pin never changes. Its `CLAUDE.md` and, once its trust files are accepted, its
  `.claude/settings.json` (hooks, allow rules) load as in today's chat.
- **Setting sources, per kind** (W0 row 1 checks each; added dirs load only under `project`):

  | Chat | `--setting-sources` | `--add-dir` |
  | --- | --- | --- |
  | in the workspace | `user,project` | the trusted read-write projects |
  | in an accepted project | `user,project,local` | the trusted read-write projects |
  | in a project not accepted | `user` | none: every other project is files only, and the banner says why |

  A chat's own folder is a tree like a project's, accepted while it holds no trust file. One
  appearing there is a trust change: the session ends at the turn's end and resumes as a chat in
  a project not accepted (the table's last row) until the owner accepts it with Trust… or removes
  the file. The plugin copy's "the CLI reads it itself" follows the same sources.
- Either way the tab's title names the place, and the Chats app lists both kinds together.

### 4.2 The projects given to the agent

**Trusted read-write projects are added; the others are files to the agent.**
- `Spec` gains `projects: Vec<Project { path, read_only, trusted }>`, every attached project, the
  cwd included, and `instructions: String`: the workspace's name, each project's path and role,
  the path of each trusted project's `CLAUDE.md`, and "read-only projects: read and search only;
  never write, switch, stash or commit there". No protocol word; the backend carries the text
  (W0 row 6).
- **Claude, a trusted read-write project:** `--add-dir <path>`, in attachment order, so edits
  there run under `acceptEdits` as in the cwd, and its skills, agents, commands and `CLAUDE.md`
  (`CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`) load as the owner accepted them. Of its
  settings only `enabledPlugins` and `extraKnownMarketplaces` load (W0 row 1); they are part of
  its accepted trust files.
- **Claude, a read-only or untrusted project:** no `--add-dir`; `Read`, `Grep` and `Glob` rules on
  it in `--allowedTools`, so reading asks nothing and the CLI loads nothing from it. An edit in an
  untrusted read-write project asks.
- **A workspace chat's own skills** (the user's folders, `~/.agents`, the personal store) load
  with no repo (`Skills::load(core, None)`), since its cwd is no repo.
- **Skills** the CLI does not read itself come through Detached's plugin copy (host §11), made at
  the open or resume from projects accepted then. A name two added projects share is the CLI's:
  the cwd's keeps the bare name, else the project added first, and the other loads as
  `/<folder>:<skill>`; the Skills page lists it so.
- **Codex:** read-write projects as additional writable roots (W0 names the adapter's field);
  read-only ones are not, and `workspace-write` writes its roots alone.
- The kernel's session keeps this list for its life (§2). A change of attachments or roles ends
  the session at its turn's end, and the next message resumes with the new list. A stated limit:
  a project made read-only mid-turn is written as read-write until that turn ends, and its end
  snapshot is skipped with a feed note. A role change is refused while a Council run holds the
  project.
- **Trust covers every tree the CLI loads from: the cwd and each added project**, by today's cwd
  rules. The session's fingerprint holds one hash per such tree; the watcher holds their trust
  paths; a turn's start check and end re-hash compare them all; the `ConfigChange` block holds
  mid-turn for each (W0 row 1: an added project's skills, agents, commands and settings). If the
  CLI does not block a mid-turn change in an added project, no project is added: all are files
  only. A change, by a pull, the owner or the agent, ends the session at the turn's
  end, or unsent at the next start; the next message resumes with that project as files only until
  the owner accepts it again, the banner offering Trust…. Accepting a project mid-session adds it
  at the next resume.

### 4.3 History, list, resume, delete, snapshots

- The host's index (`sessions.json`) gains `workspace`, `cwd` and `started`, which only the open of
  a new session Detached made sets (W5 on); a resume or the archive flag never sets it. Its entry
  gains a flattened map of unknown fields, as `Workspace` does.
- A workspace's chats are the transcripts of its chats' folders and the index's entries with this
  workspace. Other transcripts of a project's CLI folder are the archive's (§7).
- Resume passes the same `Spec`; delete and the archive flag are as now. Codex: `session/list`
  narrowed to the host's index.
- `Doc::Chat { workspace, chat, project }`: `chat` the id Detached makes at creation (a workspace
  chat's folder name), `project` the pin or none. The chats global is keyed by `chat`, so a tab
  restored at boot rebuilds the chat in its place, with its sources; `fresh` reuses an untouched
  chat only of the same place.
- **Snapshots** cover the read-write projects of the session's own list; never a read-only one.
- **The read tool** reads the chat's own workspace: `Workspace { id name repos current }`,
  `Repo.readOnly: Boolean!`, `sessions(workspace: ID)`. A participant's scope is unchanged.

### 4.4 Read-only, enforced

Against accidental writes (decision 11). A chat whose list has a read-only project, the cwd
included:
- **Claude's file tools, every OS:** `Edit`, `Write` and `NotebookEdit` deny rules on each
  read-only project and its git common dir. A deny wins in every mode (W0 row 2).
- **Claude's Bash: the fence**, on macOS and Linux. `Spec` gains `fence: Option<Fence { write,
  deny }>` beside `sandbox`, which keeps its meaning for the probe and participants; a spec has one
  or neither. The backend turns a fence into Claude's sandbox, with `failIfUnavailable: true` (host
  §5's backstop when a cached probe pass is wrong), `allowUnsandboxedCommands: false` and
  `autoAllowBashIfSandboxed` left off, so Bash asks as its mode says (W0 row 3).
  `allowWrite`: the home, the temp dirs, and each read-write project with its git common dir;
  `denyWrite`: each read-only project and its git common dir. The network is not restricted: every
  domain and every unix socket (read-only is about file writes). If W0 row 3 shows Claude's
  sandbox cannot keep ssh, docker and local sockets working, there is no fence: Bash runs
  unsandboxed under the Windows rule below on every OS, and the file-tool denies stay.
- **The fence needs the probe's pass**, since without it an ordinary `git switch` an allow rule
  lets through writes the project: on macOS and Linux, where it fails
  (bubblewrap or socat missing, a user-namespace limit), the chat does not start and shows the
  probe's fix.
- **`excludedCommands`:** when settings the session loads (the user's, the pinned project's)
  name `sandbox.excludedCommands`, an ordinary excluded `git` or `make` would run outside the
  fence, so the chat does not start and names the file.
- **Not covered**, by decision 11: what other settings add to the sandbox (`allowWrite`, `Edit`
  allow rules, `additionalDirectories`, sockets, managed settings), file-tool writes to git config,
  hooks or Detached's files that unsandboxed code later runs, and a `proc` restart of code the
  agent edited (§6).
- **Native Windows**, which has no sandbox: the owner's accepted limit. Only `default` and `plan`
  modes; each Bash ask's card says the shell can write the read-only projects; a command the owner
  or his allow rules allow runs unconfined. A TECHDEBT row (W7).
- **Hooks** of the user's settings, of accepted projects and of the plugins those enable still
  load and can write a read-only project: the owner's own code, a stated limit.
- **Codex:** only `read-only` and `workspace-write`; `agent` and full access are off. A Codex chat
  pinned to a read-only project is refused by the kernel, since its cwd is always a writable root.
- **Host tools:** `detached_act` has no repo write; `proc` refuses a read-only project's processes.

### 4.5 MCP servers and hooks

Every Detached session, chat and Council alike:
- **Detached's own MCP server alone:** the read tool, `detached_act`, Council's report tools.
  Why: third-party servers run outside the sandbox and the denies, cost context tokens on every
  turn, and skills with scripts cover the use under Bash's sandbox, denies and asks.
- **Claude:** `--strict-mcp-config` on every spawn, which keeps the session's own `sdkMcpServers`
  (host §7) and drops every other server. Setting sources stay as today, so the user's and
  accepted repos' hooks and allow rules load. If the account's claude.ai connectors still join (W0
  row 4), `core::claude` turns them off through the CLI's variable for it. If strict mode keeps a
  plugin's servers or an agent file's inline `mcpServers`, a session whose loaded settings enable
  such a plugin, or whose agents declare such servers, does not start and names the file.
  Council's 0600 file of a repo's MCP servers in `data/private/<run>/` and `Spec.mcp` go.
- **Codex:** Codex chats and trusted Codex participants run only if W0 row 7
  finds a per-session override that empties `mcp_servers` in the user's config and the project
  layer both; otherwise they are off, and Claude stands in for Council's Codex roles, as it does
  today on a tree not accepted. Untrusted Codex participants already run on a host-written project
  layer and their own home, with no third-party server.
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
  [the git-dir store](docs/decisions/2026-11-12-write-git-dir-store.md). Subject refs, finder
  branches, pinned worktrees and Fix stay in the subject's repo.
- revmux imports (`.revmux/`) stay per project, under the project in the workspace's Council app.
- **Trust marks by id**, in the same step that moves runs to workspaces (W6): `panel::marks`
  rewrites its own block from names to ids at its first load, matches by id, and shows the
  workspace's name in its reasons.
- **A read-only subject** runs as an untrusted subject does today, whatever its marks: sandboxed,
  read-only participants on an export, no worktree, no finder branches, no Fix, and no unisolated
  consent, so it refuses where the sandbox does not run. Its MR's head is fetched into
  `refs/detached/subjects/<run>` and removed with the run.
- **A trusted run in a workspace with a read-only project** keeps orchestrator mode with §4.4's
  denies and fence, whose `write` holds the subject repo's git common dir, so finders' and Fix's
  commits from their linked worktrees work (W0 row 3). Codex participants give way to Claude, as
  above. The run is refused where the fence cannot run (native Windows, a failed probe), and when
  settings its participants load name `sandbox.excludedCommands`, naming the file.
- `Doc::Panel` and `PanelTask` take a `place` in place of `repo`: `Place::Workspace(id)` for a
  run recorded under the workspace, `Place::Repo(path)` for what lives in a repo, a revmux import
  or a run from before workspaces, opened from the archive. The place joins the tab's id and
  `same()`, so two projects' `review/01` stay two tabs. `PanelStats` takes `workspace` and reads
  its runs.

## 6. Processes and worktrees

- *Proposal:* **processes stay per project.** Definitions are the project's store items and a run
  belongs to its project, running on across a switch. The Processes app lists the current
  workspace's read-write projects, grouped. A read-only project's processes are neither listed nor
  started, by the owner or an agent: they run repo code that writes.
- A chat's `proc` reaches its own read-write projects' processes at their `agents` level. A
  process runs outside the fence; one an agent edited and restarted could write a read-only
  project, which decision 11 leaves out: the `start` level already means code execution as the
  owner (host §8).
- Managed worktrees stay per project (`<folder>/worktrees/<repo>/<branch>`), shown in every
  workspace that lists it; none for a read-only one.

## 7. The archive

Read-only, in a collapsed Archive section at the end of the Chats app and of the Council app,
the same in every workspace. Nothing is moved or copied.

- **Its places:** `data/archive.json`, a list of repo and folder paths. The migration writes
  every repo and folder the settings list then; a folder a workspace drops and the folders of a
  deleted workspace are added.
- **What it holds:** the past chats of those places, as today's Chats app lists them, less the
  index's entries with a workspace; the runs under those places' `<git common dir>/detached/
  panels/`; a deleted workspace's chats and runs, under its name.
- **Chats** open as `Doc::PastChat { provider, cwd, id }` (kind `chat-archive`): the transcript,
  no composer, no resume. **Runs** open with rounds, findings and outcomes, no next round and no
  Fix; a kept finder branch keeps Merge and Discard in a read-write project.
- **Clear** removes only what Detached made and can prove: a chat whose index entry has `started`
  and is not live, and a run or folder under `data/workspaces/`. Per item, and Clear all per
  section, each after a confirm with the counts. Everything else is listed with its reason and no
  Clear: "from before workspaces" (today's chats and runs) or "not started by Detached".

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

- **Hidden, never removed.** Every document tab stays in the dock's center as today; `DocTab`
  gains its workspace (set at open, saved with it), and our `skin` draws in each center tab strip
  only the tabs of the current workspace and category, selecting the category's remembered tab. A
  switch removes no panel, so no tab's `removed` hook runs: a docked terminal keeps its shell and
  its place in `Terminals`. `DockArea::set_center` is not used: it sends `on_removed` to every
  center tab. A center pane with no tab of the shown category draws its empty state. Side zones are
  unchanged, and a tab dragged to one leaves its category.
- Each category has its own tabs, active tab and preview tab: a file from the tree replaces the
  code preview, a plan from Docs the docs preview. `show_doc` opens a doc in its category and
  brings the category forward. It looks only at the current workspace's tabs, for a match and for
  a preview to reuse, so a file of a project two workspaces share gets a tab in each.
- The center shows the current workspace's tabs; a switch hides the others' and shows its own.
- ui_state: `dock.layout` keeps every tab, as today; `tabs.<workspace id>` keeps the category shown
  and each category's selected tab.
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

- **Workspaces carry over.** At boot, one `update` gives every workspace without an `id` a new one.
- **`active` stays** as it is, read and written but unused after the migration, so an older build
  still lists repos. The first active workspace becomes `workspace.current`.
- **Chats and runs are not migrated**; the archive shows them (§7).
- **The saved layout:** the center's tabs of the old `dock.layout` load into the current
  workspace. A saved doc that no longer parses (old `Doc::Chat`, `Panel`, `PanelTask`,
  `PanelStats`) is dropped by the dock's loader, not kept as a `Missing` placeholder; `Missing`
  stays for an unknown app name.
- **A stated limit:** a build older than this drops `id` and `readOnly` on its next save, and the
  index entries' `workspace`, `cwd` and `started` on its next index write. Back on this build, the
  workspace gets a new id, and its earlier chats and runs show in the archive under its name as a
  deleted workspace's. Its runs and the chats whose `started` survived stay clearable there, so the
  release notes say not to clear them; its read-only roles must be set again.

## 11. Written rules this changes

- CLAUDE.md: the write list says a read-only attachment takes only fetch, pull and the fetch of an
  MR's head; Native UI gains the tab categories.
- PLAN-FOUNDATION §3 and PLAN-NATIVE Contracts: `Workspace::switch`, `id`, `role`, `hold`;
  `dock::register_category`, `DocKind::category`; PLAN-NATIVE's File view loses Open in Zed.
- PLAN-AGENT-HOST:
  - §1's flag table (the MCP config file row goes; `Spec` gains `projects`, `fence`,
    `instructions`) and §2's "MCP servers" row (every session strict);
  - §5 (the fence beside the sandbox; a chat beside a read-only project);
  - §7 (a chat reads its workspace; `--strict-mcp-config` on every spawn);
  - §4 (the trust fingerprint and watch cover each added project);
  - §10 (run records), §11 (skills of several projects);
  - §13 (the "Extra folders" row: trusted read-write projects by `--add-dir`, the others by read
    rules; the System prompt row gains the workspace's instructions);
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
come last (W7), with every enforcement they need, so no build ships them unenforced.

### Step W0. Spike: agents across projects

On the owner's `claude` and `codex`, macOS and Linux, Windows where it runs; answers go in this
step's status.
1. `--add-dir`, under each row of §4.1's sources table: `Edit` there runs unasked in
   `acceptEdits`; what it loads: skills, agents, commands, `CLAUDE.md` with the variable, and of
   its settings `enabledPlugins` and `extraKnownMarketplaces` alone; two added dirs with one skill
   name; a mid-turn change to its skills, agents, commands or settings fires a blocked
   `ConfigChange`. `Read`, `Grep`, `Glob` allow rules on a path not added read it unasked.
2. `Edit`, `Write`, `NotebookEdit` denies hold in every mode, `bypassPermissions` included.
3. The fence: `denyWrite` wins inside `allowWrite`; `touch`, `echo >`, `git switch` and
   `git stash` in a read-only project fail; `cargo build`, `npm install` and `git commit` work in
   the cwd, in a read-write project the CLI loads nothing from, and in a linked worktree whose
   common dir is elsewhere, as a finder's is; Bash asks in `default` and runs unasked in
   `bypassPermissions`; with the network unrestricted, `git push` over ssh, `docker ps` and a
   local DB socket work from sandboxed Bash.
4. `--strict-mcp-config` with `user,project,local`, against servers in `~/.claude.json`, in user
   settings, in an accepted repo's `.mcp.json` with `enableAllProjectMcpServers`, in an enabled
   plugin (the user's and an added project's), in an agent file's inline `mcpServers` (the cwd's
   and an added project's), and claude.ai connectors, and hooks in user and project settings:
   `initialize` lists no server but Detached's, whose tools answer; both hooks run.
5. `--resume` with the same flags keeps rows 1–4.
6. How each backend takes `instructions`: Claude's appended system prompt, Codex's developer
   instructions; both survive a resume.
7. Codex: a writable root is written, a read-only project is read and not written, in
   `workspace-write` and `read-only`; a per-session override empties `mcp_servers` in the user's
   config and the project layer, or does not.

Done when every row has an answer per OS, or a fallback the owner accepts.

### Step W1. Open in Zed out

Work: §9. Tests: `tests/files.rs` asserts no `viewer.zed` in the header and no Zed button in the
conflict bar; `tests/repos.rs` the sidebar menu without the item; the action is unknown to the
palette. Done when `rg -i 'open in zed|open_in_zed|openInZed' crates docs` finds nothing outside
decisions and KEYMAP matches. Independent of the rest.

### Step W2. Third-party MCP servers off

Needs W0 rows 4 and 7. Work: §4.5 on today's chat and Council; the README and the release notes
say third-party MCP servers no longer load in Detached's agents, and skills with scripts replace
them.

Tests: on `fake_claude`, every spawn and resume carries `--strict-mcp-config`, the chat's in a
trusted repo with `user,project,local`; no `--mcp-config` and no file in `data/private/`; a
trusted repo's hook still runs; per W0 row 4, a plugin or agent file declaring a server refuses
the session and names the file, or loads with no server. Codex: off, or its override on the fake
ACP agent.
`make agent-test` gains one row: a user server in `~/.claude.json` is not listed by the real
`claude`, and the host's read tool answers.

Done when the owner's chat lists Detached's server alone and his skills still run.

### Step W3. The workspace as context

**Status (2026-11-19): built on `ws/context`, unverified: the owner ordered no test runs, so it is
compiled and clippy-clean, its tests written and not run.** Core: `id` (made once, 8 hex),
`readOnly` and unknown fields kept, `list_repos(id)`, `all_repos()` for the untrusted deny list and
Fix's folder, folder calls by id (`add_folder(None, …)` the first workspace, made as Default when
there is none), `assign_ids` run before each listing. Kernel: `Workspace::{id, name, switch, hold,
release}`, `workspace.current`, `workspace.repo.<id>`; a switch lists in the rescan and emits
`Switched` once its repos are in; a session keeps the repos of its open on its `Identity`, holds
them, and its snapshots, `proc`, `watch` and the read tool read them. UI: the top bar's switcher
(a click switches; a new workspace is switched to; rename; delete after an inline confirm), the
sidebar shows the current workspace's folders alone, no workspace headers; the Chats app lists the
current workspace's past chats. Choices: `active` stays unused; Delete is not yet refused while
something runs (W4 for terminals, W5 for chats). Owner's check: his workspaces carry over with
ids, switching shows each one's projects and restores its current repo, a chat goes on across a
switch.

Work:
1. Core: `id`, `readOnly` (stored, no role yet), the flattened map, `list_repos(ws)`, the folder
   calls by id, the boot migration (§10).
2. Kernel: `Workspace::{id, name, switch, hold, release}`, `workspace.current`, the current repo
   per workspace. A session keeps the repos it works in from its open; `begin`'s snapshots,
   `proc::act`, `inbox::watch` and the host's snapshot read them, not the current list.
3. The top bar's switcher; the sidebar shows the current workspace; rename and delete by id.

Tests: core: ids written once and stable, `active` kept, an unknown key in a workspace kept;
`list_repos(ws)`; a delete keeps the folder. Kernel: a switch re-lists and emits once, restores
that workspace's current repo; a chat woken after a switch snapshots its own repos and no other,
and its `proc` and `watch` still find them. `tests/repos.rs`: the switcher swaps the sidebar; a
rename keeps the id; a delete leaves the projects.

Done when the owner's workspaces carry over, switching shows each one's projects alone, and a
running chat goes on across a switch.

### Step W4. Tabs by category

Needs W3. Work: §8.

Tests, `tests/dock.rs`: a file and a chat open in different categories, a Markdown file under
Docs; a category shows its tabs alone, the chips their counts; each category's preview tab; the
four actions; a restart restores each workspace's categories; a workspace switch hides one's
tabs and shows the other's; an old `dock.layout`'s center loads into the current workspace; every
registered kind has a category; a switch of category keeps a file's scroll position; a docked
terminal survives a category switch and a workspace switch and back: the same shell runs, its
exit still closes its tab, and the interceptor still passes the tab chords; no switch calls a
tab's `removed` hook; a file of a project in two workspaces, opened in each, gets a tab in each and
neither takes the other's preview; a workspace's delete is refused while its docked terminal is
open, and otherwise closes its tabs, which a restart does not bring back; an old layout with
`chat`, `panel`, `panel-task` and `panel-stats` tabs loads with them dropped and no placeholder.

Done when the owner's `align-2-0` chat and `biome.json` no longer share a strip.

### Step W5. Chats in workspaces

Needs W0, W2 and W3. Work: §4.1–§4.3, both kinds of chat, `agent.chatWorkspace`; §7 for chats,
with `data/archive.json`.

Tests, on `fake_claude`: each row of §4.1's sources table, its sources and `--add-dir`s (a
workspace chat `user,project` with one per trusted read-write project in attachment order, read
rules per other project, and the instructions; a chat in a project not accepted adds none); two
workspace chats get two folders, a file one writes survives the other's creation and its own
resume, and a chat's delete removes its folder alone; a trust file appearing in a chat's folder
ends the session at the turn's end and resumes it as a project not accepted; a resume keeps
the place and the flags; a trust file of an added project changed between turns ends the session unsent, changed
mid-turn is blocked and ends it at the turn's end, and the next message resumes with that project
as read rules only until accepted; Pull All landing a new skill in an added project does the same;
an untrusted project's skills, agents and commands never reach the CLI;
snapshots of the session's read-write projects alone; the read scope is the chat's workspace; an
archived chat opens without a composer; a workspace's delete is refused while a chat's turn runs,
and otherwise ends its idle chats, tab or none, which a later message to their inbox does not
wake; a tab of a chat pinned to a project, restored at boot, comes back pinned with
`user,project,local`; a workspace chat's user skill turned off stays off and its `~/.agents`
skills reach the plugin copy; a deleted workspace's chats show in the archive; Clear and
Clear all remove only `started` chats after their confirms; an archived or resumed terminal
session and a v0.5.0 chat are listed and never cleared; an index entry's unknown field survives
a write. The Skills page lists a clashing added skill as `/<folder>:<skill>`.

Done when the owner chats across his workspace's projects, in a project with its hooks, and finds
his old chats in the archive.

### Step W6. Council per workspace

Needs W3; after the Council rename. Work: §5 but its read-only rules; §7 for runs.

Tests: records under the workspace; a run of a project in two workspaces lists in its own alone;
marks rewritten to ids by `panel::marks` and a repo trusted by its workspace's mark still trusted;
an old run opens read-only and a kept finder branch still merges; a deleted workspace's runs in
the archive, cleared after a confirm; an old run has no Clear; an import and an old run open by
`Place::Repo`, and two projects' `review/01` open two tabs; a workspace's delete is refused while
a run runs.

Done when the owner runs a review from a workspace and finds his old runs in the archive.

### Step W7. Read-only projects

Needs W0, W5, W6. Work: §3, §4.4, §5's read-only rules, §6's; attaching read-only from Add
folder… and the project row's menu; docs/TECHDEBT.md gains the Windows row: Bash in a chat with a
read-only project is not confined on native Windows; `default`/`plan` only and the ask card
mitigate (§4.4).

Tests: core: each write refused in a read-only attachment, the store's write, Share and sharing
default among them, and allowed where the same repo is read-write; the ff-only pull skips a dirty
and a diverged tree. Kernel: keep-fresh runs at a switch and every 15 minutes on GPUI's clock
against a local remote, and skips a repo a live session of another workspace holds read-write; a
project made read-only ends a live session at its turn's end, skips its end snapshot with a note,
and the resume carries its denies and the fence and no `--add-dir`; snapshots skip a read-only
project and `begin` still runs; a role change is refused while a run holds the project. On `fake_claude`: the denies
per read-only project, the pinned cwd included; a trusted read-only project gets no `--add-dir`;
the fence's write and deny lists, its unrestricted network and `failIfUnavailable`; a spec never
carries a fence and a sandbox; the chat refused on a failed probe and on `excludedCommands`; only
`default` and `plan` on Windows. The probe and untrusted participants' flags unchanged. Codex:
`agent` and full access refused, a pinned read-only project refused. Council: a read-only subject
runs untrusted with no Fix, its MR's head fetched and removed after; a trusted run beside a
read-only project carries the fence with the subject's common dir, Claude in Codex's roles, and is refused without the fence and on `excludedCommands`. `tests/repos.rs`: menus and the VCS popup lack writes, a chord's
refusal says why, the Processes app lists no read-only project. `make agent-test` gains one row:
an edit and a `git switch` into a read-only project are refused by the real `claude`, and a
`git commit`, an `npm install` and a `git push` over ssh in a read-write one go through.

Done when the owner's knowledge-base repo stays fresh with nothing but fetch and pull, and no
chat or run writes it by accident, but through a hook or, on Windows, an allowed command.

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

- **Read-only's boundary** (decision 11): an agent that means to escape can, through git config,
  hooks, settings or a process; untrusted code belongs on Council's untrusted path.
- **No third-party MCP servers** in Detached's agents: a workflow that needs one moves to a skill
  with a script, or to the owner's own terminal `claude`.
- **Hooks** of the owner's settings, of accepted projects and of the plugins they enable can
  write a read-only project; stated.
- **Windows:** Bash in a chat with a read-only project is not confined; the owner accepted it;
  TECHDEBT row from W7.
- **`--add-dir` semantics** move between CLI versions; `min_cli` pins what W0 row 1 saw.
- **Hiding tabs in our skin** while the kit's tab panel keeps them is unproven; W4 starts with
  the terminal test.

