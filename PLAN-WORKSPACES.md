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

**Status (2026-11-18): the owner's answers in; waiting for the plan review.** What the owner
decided is in the [decision](docs/decisions/2026-11-18-workspaces.md); choices still marked
*Proposal* are this plan's own, for the review.

Owner's decisions, 2026-11-18:
1. The workspace is the top-level context, and the window shows one at a time. Chats and Council
   runs belong to a workspace; a chat's agent works across the workspace's projects.
2. One project can be attached to several workspaces.
3. A project can be attached read-only: read, search, Docs, and fetch/pull to keep it fresh.
   Nothing else writes to it, enforced for agents, not only hidden in the UI.
4. Start clean: per-project chats and Council runs are not migrated; they stay readable in an
   archive, read-only. A deleted workspace's chats and runs go there too.
5. Document tabs by category, as category chips at the start of one strip.
6. A chat can be started inside one project, so that project's own context loads.
7. Open in Zed is removed everywhere.

Words: a **project** is a repo the workspace lists (today's `RepoInfo`); an **attachment** is a
project in a workspace, with a **role**, read-write or read-only. Council is today's Panels; its
code rename is separate (§12).

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
  worktree takes its repo's role; else read-write. The same repo can be read-only in one workspace
  and read-write in another.
- **Identity is `id`**, 8 random hex, made once. Chats, runs, tabs, trust marks and ui_state keys
  name the id; the name is a unique label, kept also in `data/workspaces/<id>/name`.
- **Rename** changes the name alone. **Delete** removes the settings entry and nothing else: its
  folder stays, so its chats and runs show in the archive (§7). Its projects are not touched.
- **A project in no workspace** is not listed, as today. Its old chats and runs stay in the
  archive.
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
- A switch stops nothing: other workspaces' chat turns and runs go on, their jobs in Activity under
  the workspace's name; processes and terminals belong to projects (§6).

## 3. Read-only projects

- **Detached reads everything:** tree, files, diffs, log, MRs, pipelines, find, Docs.
- **Keeps it fresh:** fetch, then `pull --ff-only` when the worktree is clean and the branch
  tracks; otherwise the pull is skipped and the project's row says why. At a switch to the
  workspace, every 15 minutes while the window shows, and in Fetch All and Pull All.
- **Allows one more fetch:** a Council run's MR head into `refs/detached/subjects/<run>` (§5).
- **Refuses every other write** of CLAUDE.md's list: switch and checkout, new and delete branch,
  stash, prune, managed worktrees and recipes, chat snapshots and restores, Council finder
  branches, pinned worktrees and Fix. Processes do not start (§6).
- **In core, not only in the UI.** Each write entry point takes the workspace id and refuses a
  read-only attachment: "`kb` is read-only in this workspace".
- **UI:** a lock on the project's row and in the file and diff headers; the branch widget shows
  the branch without its popup; write items leave menus and the VCS popup; a chord for one says
  why through Activity.
- **The user's own terminal is not limited:** what the owner types is the owner's.
- Agents: §4.4.

## 4. The workspace chat

### 4.1 Where it runs

*Proposal.* A chat for the whole workspace runs in `data/workspaces/<id>/chat/`, a folder
Detached makes and keeps empty.
- Why: the CLI keeps transcripts by cwd, so one folder per workspace lists its chats (§4.7); no
  project is primary; a rename moves nothing.
- Its cwd has no settings, so no project's `.claude/settings.json` loads there; a chat in one
  project (§4.2) is the way to have them.

### 4.2 A chat in one project

- **Pinned at creation.** The Chats app's New chat has a menu: "In the workspace" (the default)
  and each project of the workspace; a project row's menu has New chat here. `agent.chat` starts
  one in the current repo, as its KEYMAP line says today; `agent.chatWorkspace` starts a workspace
  one. The pin never changes: a resume runs where the chat started.
- **cwd is that project**, so its `CLAUDE.md` and, once its trust files are accepted, its
  `.claude/settings.json` (hooks, MCP servers, allow rules) load as in today's chat:
  `--setting-sources user,project,local` when accepted, `user` until then, with today's banner.
- **The other projects are still attached** (§4.3) with their roles; their settings do not load.
- **A read-only project can be pinned:** its settings load under its acceptance, and §4.4 denies
  every write to it, the cwd included, as an untrusted participant's cwd is today.
- The tab's title names the project; the Chats app lists it with the workspace's other chats.

### 4.3 The projects given to the agent

- `Spec` gains `dirs: Vec<Dir { path, read_only }>`, every attached project but the cwd, and
  `instructions: String`: the workspace's name, each project's path and role, and "read-only
  projects: read and search only; never write, switch, stash or commit there". `Spec.repo` stays
  the cwd. No protocol word; the backend carries the text (W0 row 7).
- **Claude:** `--add-dir <path>` per `dirs` entry, read-only ones included, so `Read`, `Grep` and
  `Glob` there do not ask; `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`, so each project's
  `CLAUDE.md` loads. W0 confirms both on `min_cli`.
- **Codex:** read-write projects as additional writable roots (W0 names the adapter's field).
  Read-only ones are not added: `workspace-write` reads the whole disk and writes its roots alone.

### 4.4 Read-only, enforced

- **Claude's file tools, every OS:** `Edit`, `Write` and `NotebookEdit` deny rules on
  `//<project>/**` and its git common dir, in `--disallowedTools`. A deny wins over `--add-dir`
  and every mode (W0 row 2).
- **Claude's Bash, macOS and Linux:** a chat with a read-only project runs in Claude's sandbox:
  `filesystem.denyWrite` on those projects and their git dirs, writes elsewhere allowed,
  `allowUnsandboxedCommands: false`, the network as today (W0 row 4 finds the settings). The
  "sandbox ready" probe must pass; when it fails, the Windows rule applies and the chat says so.
- **Claude's Bash on native Windows**, which has no sandbox: the modes that run Bash unasked
  (`bypassPermissions`, `auto`) are off in that chat, and each Bash ask's card says the shell can
  write the read-only projects. Accepted by the owner as a limit; a TECHDEBT row (W5).
- **Codex:** its asking mode's `workspace-write` holds; full access is refused in a chat with a
  read-only project.
- **Host tools:** `detached_act` has no repo write; `proc` refuses a read-only project's processes.
- **A change** of attachments or roles ends the workspace's live sessions at their turn's end; the
  next message resumes with the new flags, as a trust change does (host §4).

### 4.5 Trust and skills

- Acceptance stays per repo and provider, against its trust files' hash. An attached project
  gives a chat only its skills and `CLAUDE.md`: an added directory's settings do not load (W0
  row 3). Only the cwd's settings load, for a chat in one project.
- **Skills:** each trusted project's skill folders join the folder list; an untrusted project's
  are turned off by name, as today, and listed with why. *Proposal:* a name two projects share
  loads from the pinned project, else from the one attached first; the Skills page lists the
  other as shadowed. Codex reads the rest through `detached_read skills`, as now.
- The chat's start screen lists each untrusted project with its Trust… button, where today it has
  one banner.
- The watcher holds every attached project's trust paths while the session lives.

### 4.6 The read tool

A chat reads its own workspace, not the current one: `Workspace { id name repos current }`,
`Repo.readOnly: Boolean!`, `sessions(workspace: ID)`. A Council participant's scope is unchanged.

### 4.7 History, list, resume, delete

- The host's index (`sessions.json`) gains `workspace` and `cwd` per chat it starts.
- A workspace's chats are the transcripts of its own folder (`past(cwd)`, as a repo's today) and,
  for chats in one project, the transcripts the index names with this workspace. A project's
  other transcripts are not the workspace's: they are the archive's (§7).
- Resume passes the same `Spec`, `dirs` included; delete and the archive flag are as now. Codex:
  `session/list` narrowed to the host's index.
- `Doc::Chat { workspace, n }`; the chats global is keyed by `(workspace, n)`; the Chats app lists
  the current workspace's open and past chats.
- **Snapshots** cover the workspace's read-write projects, where `Session::repos` takes a folder's
  repos today; never a read-only one.

## 5. Council

- A run belongs to a workspace; its subject names a project's branch, MR, issue or document.
- *Proposal:* records go to `data/workspaces/<id>/council/<task>/<run>/`, not
  `<git common dir>/detached/panels/`. Why: the run is the workspace's; a project in two
  workspaces would show it in both; a read-only project's git dir takes no record. This amends
  [storage scopes](docs/decisions/2026-11-12-storage-scopes.md): run records stay local, per
  workspace. Subject refs, finder branches, pinned worktrees and Fix stay in the subject's repo.
- revmux imports (`.revmux/`) stay per project, under the project in the workspace's Council app.
- Trust marks: `panels.workspaces` by id.
- **A read-only subject** gets a review and no Fix, in every mode: participants read an export
  from raw blobs (the untrusted path's export), no pinned worktree, no finder branches; the card
  says "read-only in this workspace". Its MR's head is fetched into
  `refs/detached/subjects/<run>` and removed with the run, as for any MR. Trust marks still decide
  the sandbox and which settings load.
- `Doc::Panel`, `PanelTask` and `PanelStats` take `workspace` in place of `repo`; stats read the
  workspace's runs.

## 6. Processes and worktrees

- *Proposal:* **processes stay per project.** Definitions are the project's store items and a run
  belongs to its project, running on across a switch. The Processes app lists the current
  workspace's read-write projects, grouped, where today it lists the current repo's. A read-only
  project's processes are neither listed nor started, by the owner or an agent: they run repo code
  that writes.
- A chat's `proc` reaches its workspace's read-write projects' processes at their `agents` level.
- Managed worktrees stay per project (`<folder>/worktrees/<repo>/<branch>`), shown in every
  workspace that lists it; none for a read-only one.

## 7. The archive

Read-only, in a collapsed Archive section at the end of the Chats app and of the Council app.
Nothing is moved or copied.

- **What it holds:**
  - today's chats, past chats by repo and folder as the Chats app lists them now, less those the
    index gives a workspace;
  - today's runs, under the projects' `<git common dir>/detached/panels/`;
  - a deleted workspace's chats and runs, under its name: the folders under `data/workspaces/`
    whose id the settings no longer list, and the index's chats naming that id.
- **Chats** open as `Doc::PastChat { provider, cwd, id }` (kind `chat-archive`): the transcript,
  no composer, no resume.
- **Runs** open with rounds, findings and outcomes, no next round and no Fix. A kept finder branch
  keeps Merge and Discard in a read-write project, so none is stranded.
- **Clear:** per item, and Clear all per section, each after a confirm with the counts. A chat's
  clear is its delete (transcript and index entry); a run's removes its record. A record in a
  read-only project is not cleared: Clear all skips it and says how many stayed. A deleted
  workspace's folder goes with its last item.

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
  `git::files::open_in_zed` with `ZED_CLI` and its test.

## 10. Migration

- **Workspaces carry over.** At boot, one `update` gives every workspace without an `id` one and
  rewrites `panels.workspaces` marks from names to ids.
- **`active`** is read (`skip_serializing`) and dropped on save: the first active workspace becomes
  `workspace.current`.
- **Chats and runs are not migrated**; the archive shows them.
- **The saved layout:** the center's tabs of the old `dock.layout` load once into the current
  workspace's categories; old `Doc::Chat` and `Doc::Panel` tabs, keyed by repo, are dropped.
- **A stated limit:** a build older than this drops `id` and `readOnly` on its next save, and a
  later boot then makes new ids: that workspace's chats and runs show in the archive as a deleted
  workspace's.

## 11. Written rules this changes

- CLAUDE.md: the write list says none of it happens in a read-only attachment, but the fetch of
  an MR's head; Native UI gains the tab categories.
- PLAN-FOUNDATION §3 and PLAN-NATIVE Contracts: `Workspace::switch`, `id`, `role`;
  `dock::register_category`, `DocKind::category`; PLAN-NATIVE's File view loses Open in Zed.
- PLAN-AGENT-HOST §7 (a chat reads its workspace), §10 (run records), §11 (skills of several
  projects), §13 (extra folders are the workspace's projects).
- PLAN-PANELS §3 (read-only subjects), §14 (where runs live).
- docs/KEYMAP.md: the tab rows and `agent.chatWorkspace`; `file.openInZed` goes.
- README: workspaces and read-only projects.

## 12. Steps

Each step passes the host plan's gate: `make check`, a revmux round without blocking findings, the
owner's check on macOS. Docs of §11 change in the step that makes them true.

### Step W0. Spike: agents across projects

On the owner's `claude` and `codex`, macOS and Linux, Windows where it runs; answers go in this
step's status and W5 and W6 follow them.
1. `--add-dir`: `Read`, `Grep`, `Glob` there ask nothing; `Edit` asks in `default`, not in
   `acceptEdits`.
2. `Edit`, `Write`, `NotebookEdit` denies on an added dir and on the cwd hold in every mode,
   `bypassPermissions` included.
3. An added dir's `.claude/settings.json` does not load; its `.claude/skills` do or do not; its
   `CLAUDE.md` loads with the variable.
4. The sandbox with writes allowed but on the read-only roots: `touch`, `echo >`, `git switch` and
   `git stash` there fail; `cargo build`, `npm install` and `git commit` in a read-write project
   work; network asks reach the host or pass.
5. `--resume` with the same flags keeps rows 1–4.
6. Codex: a writable root is written, a read-only project is read and not written.
7. How each backend takes `instructions`: Claude's appended system prompt, Codex's developer
   instructions; both survive a resume.

Done when every row has an answer per OS, or a fallback the owner accepts.

### Step W1. Open in Zed out

Work: §9. Tests: `tests/files.rs` asserts no `viewer.zed` in the header and no Zed button in the
conflict bar; `tests/repos.rs` the sidebar menu without the item; the action is unknown to the
palette. Done when `rg -i zed crates` finds no Open in Zed and KEYMAP matches. Independent of the
rest.

### Step W2. The workspace as context

Work:
1. Core: `id`, `readOnly`, `list_repos(ws)`, the folder calls by id, the boot migration (§10).
2. Kernel: `Workspace::{id, name, role, switch}`, `workspace.current`, the current repo per
   workspace.
3. The top bar's switcher; the sidebar shows the current workspace; rename and delete by id.

Tests: core: ids written once and stable, `active` to current, marks from name to id, an unknown
key kept; `list_repos(ws)` and roles by folder and linked worktree; a delete keeps the folder.
Kernel: a switch re-lists and emits once, restores that workspace's current repo.
`tests/repos.rs`: the switcher swaps the sidebar; a rename keeps the id; a delete leaves the
projects.

Done when the owner's workspaces carry over and switching shows each one's projects alone.

### Step W3. Tabs by category

Needs W2. Work: §8.

Tests, `tests/dock.rs`: a file and a chat open in different categories, a Markdown file under
Docs; a category shows its tabs alone, the chips their counts; each category's preview tab; the
four actions; a restart restores each workspace's categories; a switch swaps the center; an old
`dock.layout`'s center loads into categories; every registered kind has a category; a switch of
category keeps a file's scroll position.

Done when the owner's `align-2-0` chat and `biome.json` no longer share a strip.

### Step W4. Read-only projects

Needs W2. Work: §3 and §6's read-only rules; attaching read-only from Add folder… and the project
row's menu.

Tests: core: each write refused in a read-only attachment and allowed where the same repo is
read-write; the ff-only pull skips a dirty and a diverged tree. Kernel: keep-fresh runs at a
switch and every 15 minutes on GPUI's clock against a local remote. `tests/repos.rs`: menus and
the VCS popup lack writes, a chord's refusal says why, the Processes app lists no read-only
project.

Done when the owner's knowledge-base repo stays fresh with nothing but fetch and pull.

### Step W5. The workspace chat

Needs W0, W2, W4. Work: §4 but §4.2, and §7 for chats; docs/TECHDEBT.md gains the Windows row:
Bash in a chat with a read-only project is not sandboxed on native Windows, so it can write that
project; the asking modes and the ask card mitigate (§4.4).

Tests, on `fake_claude`: the args carry `--add-dir` per project, the denies per read-only one and
the instructions; a resume repeats them; the sandbox only with a read-only project;
`bypassPermissions` refused on Windows with one; snapshots of read-write projects alone; an
untrusted project's skills off; an attachment change resumes at the turn's end; the read scope is
the chat's workspace; an archived chat opens without a composer; a deleted workspace's chats show
in the archive; Clear and Clear all after their confirms. `make agent-test` gains one row: an
edit into a read-only project is refused by the real `claude`.

Done when the owner chats across his workspace's projects and the knowledge base cannot be
written.

### Step W6. A chat in one project

Needs W5. Work: §4.2; the index's `workspace` and `cwd`; `agent.chatWorkspace`.

Tests, on `fake_claude`: a chat pinned to a project runs there with `project` settings once
accepted and `user` before, the other projects added with their roles; a pinned read-only project
denies writes to the cwd; a resume keeps the pin; the workspace lists its pinned chats and not the
project's other transcripts, which the archive shows; a trust change of the pinned project ends
the session at the turn's end.

Done when the owner's chat in a project runs that project's hooks and MCP servers and still sees
the workspace's other projects.

### Step W7. Council per workspace

Needs W2 and W4; after the Council rename. Work: §5 and §7 for runs.

Tests: records under the workspace; a run of a project in two workspaces lists in its own alone; a
read-only subject runs on an export with no worktree or branch and offers no Fix; its MR's head is
fetched into `refs/detached/subjects/<run>` and removed after; marks by id; an old run opens
read-only and a kept finder branch still merges; a deleted workspace's runs in the archive; an old
run in a read-only project is skipped by Clear all.

Done when the owner runs a review from a workspace and finds his old runs in the archive.

## 13. Parallel work

- **ui/polish** (presentation, `crates/app`) touches `dock/skin.rs`, `dock/doc.rs`, `chat/`,
  `repos/sidebar.rs`, `repos/workspaces.rs`, `notices.rs` and `files/viewer.rs`. W0 and W2's core
  and kernel parts run beside it; W1, W2's UI and W3 rebase on it after it merges.
- **The Council rename** goes before W7, so W7 writes the new names once. W3's category table
  names kinds by string and takes the new ones in the rename's commit.
- W3 and W4 can run in parallel after W2; W5 waits for W0.

## 14. Risks

- **The chat in a sandbox** may break toolchains or ask about the network; W0 row 4 decides, the
  fallback is Windows' rule on every OS.
- **`--add-dir` semantics** move between CLI versions (skills, `CLAUDE.md`); `min_cli` pins what
  W0 saw.
- **Windows:** Bash in a chat with a read-only project can write it; asks and a card mitigate,
  nothing enforces. The owner accepted it; TECHDEBT row from W5.
- **The kit's `set_center`** with kept tab entities is unproven; W3 starts with that test.
