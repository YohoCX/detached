# Detached — extensibility plan

An AI chat in the client, a look and layout the user can change, and plugins for the rest. Built in
the order that gets a usable feature first, except [PLAN-FOUNDATION.md](PLAN-FOUNDATION.md), which
goes first on purpose; each step can change the ones after it.

Decided with the owner on 2026-11-07, after a revmux plan review
(`.revmux/tasks/extensibility-ideas/`, kept locally):
- Chat comes first; the app registry and zones are built once chat and the terminal show what they must
  carry.
- No `crates/sdk`: built-ins stay on the kernel's entities and the `api.rs` contracts, and `crates/kernel` is
  internal. A registry lives in `crates/app`. The only versioned public API, when one is needed, is the plugin manifest and protocol.
- [PLAN-FOUNDATION.md](PLAN-FOUNDATION.md), agreed on 2026-11-10, changes the decisions above as its
  steps land: a kernel crate, registries, settings, updates, invalidation, the watcher.
- Detached moves off read-only one capability at a time, past the `switch`, `stash` and `pull` it already
  runs. Each new write gets its own decision and test.

**Status (2026-11-09): 0.5.0 ships steps 1–3 as below; the chat's sidecar is not in the packages. Step 0 answered on macOS, see [its answers](#spike-answers), Linux and Windows open; the owner took the sidecar. Step 1's first slice is on `main` (local): everything listed below is in. Context shows as a percent with its breakdown in a menu; the ring waits for step 2. Opening the chat starts the sidecar so it can list past conversations. Left before Done: the owner's week of real use, Linux and Windows, then the second slice. Step 2's colors and fonts are on `main` (local), waiting for the owner's look on macOS. Step 3 is on `main` (local), waiting for the owner's look on macOS: zones on the kit's dock, documents in tabs, a flat title bar and a status bar, no icon strip.**

## Steps

Each step ends green (`make check`), checked by hand on macOS. Thresholds below are proposals for the
owner to move.

### 0. Spike

Throwaway, outside `crates/`: a sidecar on `@anthropic-ai/claude-agent-sdk` that dumps every event, and
the smallest view that sends a prompt. It may reorder or cut anything below. Answer the rows in order.

| Question | If the answer is | Then |
| --- | --- | --- |
| Bundled CLI or the user's `claude` (`pathToClaudeCodeExecutable`), given that features gate on CLI version and a compiled sidecar has to embed the 322 MB binary | the bundled binary can't reuse the login, or is too big | the user's `claude`, with a minimum version check and detection from the init message's `capabilities` |
| Size on disk and idle memory of a `bun build --compile` sidecar alone, without the CLI; record the CLI's size and memory separately | over 150 MB, or over 200 MB idle | speak `stream-json` from Rust instead |
| Do turn boundaries, `SDKPermissionDeniedMessage`, `SDKStatusMessage` and subagent events carry what step 1's feed and mode indicator need | no | redesign the feed before writing it |
| Is `apiKeySource` `ANTHROPIC_API_KEY` with a key set and `none` on a claude.ai login | no | a different public build gate |
| Does killing the app or the sidecar leave processes, on macOS, Linux and Windows, and do the SDK and CLI exit when stdin closes | yes, no | the sidecar takes EOF on stdin from the host as a kill and takes its children down; on Windows a job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, which `win::Job` lacks. A process group alone gets no signal when the app is killed |
| Does a running SDK session pick up a new project hook or `apiKeyHelper` from settings, and does a `ConfigChange` hook block it | yes, yes | step 1's trust rule blocks them mid-session as written there; if it can't block, a change ends the session |
| With `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1`, can a Bash call read the CLI's env (`ps eww -ax`), and on Linux a hook or stdio MCP server, checked on a dummy key | yes | record it for the public build decision in step 1 |
| Do Gatekeeper and SmartScreen accept a binary the app downloaded | no | plugins from a folder only, which step 4 needs anyway |
| What fills the context in real sessions (with the owner's OK to read transcripts) | Bash output is not the top item | drop the output trimming idea, take what is |

The owner also asks Anthropic whether a personal install on the owner's own claude.ai login is fine; the
docs forbid third parties offering that login and don't address personal use.

Done when: each row has an answer written here, and the owner has decided go or no-go on the sidecar.

#### Spike answers

2026-11-08, macOS arm64, Claude Code 2.1.319, SDK 0.3.319, in temp repos on the owner's login. Linux and
Windows were not run.

| Question | Answer | What it changes |
| --- | --- | --- |
| Bundled CLI or the user's | The CLI is 310 MB here, 322 MB for the SDK's package. Reusing the login from the bundled binary was not tried | The user's `claude`; init reports its version and 15 `capabilities` |
| Sidecar alone | 86.3 MB on disk, 46 MB idle; the `claude` child is 284 MB idle on any route | Inside the thresholds: the sidecar stays |
| Do events carry the feed | Yes: `result` per turn, `system/init` repeated per turn, `status`, `permission_denied`, `task_*`, `background_tasks_changed`, `parent_tool_use_id` on subagent events. `getContextUsage` gives categories. A classifier block was not provoked (`git push --force` with no remote ran) | Feed as planned; the first real block checks its shape |
| `apiKeySource` | `none` on the login. `ANTHROPIC_API_KEY` was not run, no key here | Gate stands, its allow half untested |
| Orphans | Sidecar killed: `claude` and the tool die with it. Host killed, no EOF handling: sidecar and tool live on. With EOF as a kill: all gone | EOF as a kill is required, as step 1 now says |
| Hook reload | Yes: a settings file written mid-session fired its hook on later turns. A `ConfigChange` hook returning `decision: "block"` stopped it (0 fires, source `project_settings`) | The trust rule works as written |
| Scrub | Removes `AWS_SECRET_ACCESS_KEY` from a Bash call's env, leaves other variables, and the dummy is still readable through `ps eww -ax`. It **forces `default` mode**, reported only on stderr, and `allowedTools` does not keep `auto` | Scrub and `auto` can't be combined: step 1's env rule changed |
| Downloaded binary | A quarantined, ad-hoc signed sidecar is killed on launch (exit 137); with no quarantine it runs. SmartScreen not run | Unsigned plugin binaries work only if built locally or fetched by the app without the attribute |
| What fills the context | `Read` and `Bash` results: 33% and 23% of 217 M chars in 267 main sessions, 37% and 40% of 855 M in 1731 subagent files. `Edit` calls 0.4% and 0.5%, model text 3.7% and 0.3%. Cumulative characters, not one context | Target reads and Bash output; not retelling edits barely moves the total |
| Stop and mode, added | `interrupt()` returns in about 100 ms with `error_during_execution`, but the running Bash goes on in the background (`task_started`, `is_backgrounded`); `stopTask` ends it. The session takes a next turn. `setModel` to Haiku 4.5 drops to `default`, `auto` fails there and re-arms after a supported model | Step 1's stop is both calls |

### 1. Chat

`Doc::Chat` over the existing page slot (`shell.rs` `page()`), its sessions in a global as `Terminals`
holds shells, so a session survives its view. Opening a file or diff replaces the chat in the center
until it is reopened; step 3 lifts that. The sidecar is found in a folder and stays out of release
packages; one private stdio protocol between them. Sessions run in the current folder.

First slice:
- Streaming, model and effort selectors, `/` commands, `@` files, history and resume.
- Stop is `interrupt()` plus `stopTask()` for what the turn started: `interrupt` alone leaves a running
  Bash command alive in the background. Background tasks show in the feed.
- `auto` by default. A live mode indicator from `SDKStatusMessage`, models auto does not support marked,
  `auto` re-armed after a model switch, since `setModel` can silently drop a session to `default`.
- Compact action feed, with blocked actions and their reasons.
- Approval cards wired to `canUseTool`: rare, but paused `auto` reaches them.
- Context ring from `getContextUsage`, with its breakdown.
- Snapshot, one new write: the host stores the worktree as a git tree at each turn boundary (`add -A` and
  `write-tree` on a temp index seeded from a copy of the repo's, a private ref, old ones pruned). The
  per-turn diff goes through `Side::Rev`, because `rewindFiles` skips Bash and subagent edits. The runner
  strips `GIT_INDEX_FILE` and offers a 30 s kill or no cancel, so this needs its own kind. A turn boundary
  can't tell the agent from the user: edits saved elsewhere during a turn show in its diff, and the UI says so.
- Restore, a second write, to the worktree: take a fresh snapshot first so it can be undone, list the files
  it will change, and confirm when a file differs from the turn's end tree. A file the agent created is
  removed only if it is unchanged since then. A path on disk but absent from the fresh snapshot is ignored
  by now, has no undo copy, so restore skips it and says so. The UI says ignored files and submodules are
  not covered.
- Changes and the repo status refresh at each turn boundary, and in between as the watcher sees the
  agent's edits (PLAN-FOUNDATION §7); the window stays focused while an agent edits.

Rules from the first line of code:
- The sidecar gets an explicit `env` with no project secrets, and without
  `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB`, which forces `default` mode. The personal build has no key in it,
  so nothing there needs scrubbing. The host compares the reported mode with what it asked for and shows a
  difference, since the CLI reports it only on stderr.
- The sidecar takes EOF on stdin from the host as a kill and takes its children down.
- `Read(//**/.env*)` and matching Bash patterns are denied in the session's options. That stops Claude's
  file tools and the Bash file commands it recognises, not `grep -r` or a script; the docs and UI say so.
  A real limit means the Bash sandbox, a separate decision.
- Per-repo trust: SDK sessions show no trust dialog, so project settings run on start. Until the user
  accepts a list of what they would run (hooks, the `env` block, helper commands such as `apiKeyHelper`,
  skill hooks and `allowed-tools`, `.mcp.json`), `settingSources: ["user"]`. The acceptance is stored
  against a hash of those files and asked again when they change, since a pull or switch can add a hook.
  The hash does not cover the scripts those commands run, so the prompt says accepting also trusts future
  versions of them.
  A repo with an empty list loads its project settings with no prompt. Claude Code reloads settings into a
  running session, so the session also registers a `ConfigChange` hook blocking project, local and skill
  changes, and the host re-hashes at each turn boundary and after its own pull or switch, ending the
  session until the new list is accepted.
- UI text says "Claude", never "Claude Code".

Added 2026-11-09, on the owner's look: the bar under the box shows the agent, the provider or plan
(`accountInfo`) and what is left of the 5-hour and weekly limits, read on open and after each turn;
before a session runs, a `claude` with no settings, so no hooks, answers. Tool calls in a row fold
into one line, edits show the lines they add and remove, a command's output opens on a click, and
each turn ends with its time and a button that copies the answer; code blocks copy too. The last
limits and commands are kept in `ui_state` for the next open.

Second slice: subagents and tasks, MCP status, fork, open in terminal via `--resume` (without injected env),
Detached data as tools through the sidecar.

Before any public build: the key goes through `env`, and the sidecar runs only when `apiKeySource` is
`ANTHROPIC_API_KEY`. Anything else is refused, `none` included: `claude` falls back to the claude.ai login
by itself and reports it as `none`. Also decide how the key travels: in `auto` without the scrub, every
Bash call, hook and stdio MCP server sees it; with the scrub, sessions run in `default` with an allow
list, and same-user processes can still read it from the CLI. `apiKeyHelper` through the `settings`
option is untested and would change the gate's allowed value.

Done when: the owner runs real sessions in it for a week instead of the terminal, a headless test covers
a fake sidecar's stream, stop, crash and resume, tests on a real repo cover restore over a Bash edit, a
file created after the snapshot, an ignored file, a file that became ignored during the turn and a user
edit after the snapshot, and CLAUDE.md's
"read-only" line is updated.

### 2. Colors and fonts

Runtime themes from JSON with hot reload, the Fleet-like default in light and dark, the current look as a
bundled theme. Inter and JetBrains Mono bundled (closes the fonts line in docs/TECHDEBT.md).
- First decide `ThemeRegistry::watch_dir` and `ThemeConfig` from the kit against our own format; our
  tokens, syntax colors and `lane()` are not in its schema.
- `theme::sync` gets a trigger for reload. Viewer line numbers stay an exception (kit limit, TECHDEBT).
- `radius::`, `text::` and `px()` stay as they are until step 3 needs otherwise.

Done when: editing a theme file changes the running app, a broken file keeps the last good theme and
says why, and a test covers both.

Decided 2026-11-08: our own format, not the kit's `ThemeConfig` and `watch_dir`. Its schema has none
of our tokens, and its watcher only logs a broken file. A file holds what it changes over the theme it
`extends`, plus `font` and `mono_font`; see docs/THEMES.md. The folder is polled every 500 ms instead
of watched, which needs no new dependency and sees an editor's atomic save. Syntax colors stay the
kit's for now (TECHDEBT).

### 3. Dock and zones

First, a time-boxed trial of a few days: the current panes on the kit's `DockArea` with a custom renderer
(tab drag, `dump`/`load`, `PanelRegistry`, `invalid_panel`). Record the verdict here; write our own only
where it fails, with the upstream issue linked.

Then: Repos left, Changes/Project/MRs right, Log/Pipelines/Terminal bottom, documents center; flat title
bar, no icon strip, status bar at the bottom, a place for each top bar widget. This takes:
- An edit to PLAN-NATIVE's contracts: `Tool`, `Bottom`, `open_tool`, `open_bottom`, `Store.shown`.
- Several open documents, and navigation history per zone or global (decide here).
- Actions sent to the focused tab rather than the last one mounted: with two diffs, F7 steps the wrong one.
- Apps owning long-lived state outside views; tab views stay alive across a switch and a move.
- Tests in two moves: first through a fixture helper with stable `bottom.*` and `tool.*` ids, green on
  the enums; then the enums go with those tests unchanged.
- Whether a tab pins to a repo or follows the active one (decide here). Chat sessions keep their folder
  either way.

Done when: a live terminal and a running chat move between zones in a headless test without losing their
session, the layout survives a restart, and a saved panel that no longer exists shows a placeholder.

Trial, 2026-11-08, on `native/dock`: the kit's `DockArea` stays, our code is the skin (`dock/skin.rs`)
and one `Tab` entity per app (`dock/mod.rs`). Tested headless: a terminal with a live session dragged
from the bottom zone to the right keeps its session and screen, the layout is saved in `ui_state`
(`dock.layout`) and comes back on the next window, and a saved app this build lacks shows a placeholder
that saves its state back. Base draws nothing and binds no keys; drag, drop targets and focus are its.
What it costs:
- A closed bottom zone keeps a 29 px strip of its tabs (`CLOSED_BOTTOM_STRIP`), so it can't vanish. Kept
  for now: with no icon strip it is how the bottom apps open. The owner's call.
- The last tab of a zone can't be dragged out, so Repos and the main area stay where they are.
- A zone sizes from the pointer's position, so the grip keeps the grab offset itself. A side zone is
  capped only by the other side, not by a maximum of ours.
- The bottom zone spans the center, between the side zones, as in Fleet.
- Hidden views stay alive, so a tab builds its view when first on screen (else the MR list fetches for
  nobody). Project keeps its selection when closed now. The terminal starts its first shell when its
  tab is shown, through `Panel::set_active`.

Done, 2026-11-08:
- The dock is the only record of what is open: `Tool`, `Bottom`, `open_tool` and `open_bottom` are
  gone for `dock::open`, `toggle`, `close` and `is_shown` (PLAN-NATIVE's contracts changed with it).
  `store.shown()` is the document of the tab last on screen; `store.show` opens a tab or brings its
  tab forward, in whatever zone it is.
- Documents are tabs (`dock::DocTab`), saved with the layout. Files and diffs share one preview tab,
  in italics, which a double click keeps; MR pages, job logs and the chat get a tab each. A tab closes
  on its button or a middle click. With no document, the center shows the hints.
- Keys go to the tab on screen: a tab coming forward raises its actions (`actions::raise`), so with two
  diffs F7 steps the one shown.
- Navigation history stays one for the window: back and forward reopen a document wherever its tab is.
- Tabs follow the current repo, as the panes did; a document and a chat carry their own repo.
- The title bar is flat, with buttons for the chat, each zone and Settings; the status bar is at the
  bottom; the icon strip is gone. The closed bottom zone's strip of tabs stays, until the owner says.
- Tests moved to the tab ids (`tool.*`, `bottom.*`) through `on_screen` and `open_tab`, then the enums
  went. Covered headless: a running chat and a live terminal moved between zones, documents back
  after a restart, the preview tab, F7 on two diffs.
- The kit gives an empty center no tab group, so nothing can be dropped there.

Added 2026-11-09, on the owner's look:
- Repositories draws the same tab bar as every app, and shares the left zone with Chats, so it moves
  like the others. Tooltips name an app's or a button's chord; Cmd+B, Cmd+J and Cmd+Alt+B open and
  close the zones.
- A shell dragged out of the Terminal app onto the center or any tab bar becomes a tab of its own
  (`Doc::Terminal`); closing the tab ends it, and after a restart it comes back as a new shell in
  its folder. A lone document closes too, to an empty center, through `DockArea::remove_panel`: the
  kit keeps the last tab of a zone from its own close.
- Chats lists the chats open in this run, then past conversations in the workspace's folders by
  repo and folder (the sidecar's `sessions` with no folder lists every project's).
- A chat runs on a folder as well as a repo ("Chat here" on a folder): each turn snapshots every
  repo inside, and its changes and going back come per repo.
- Events, in the bottom zone, keeps every error and warning the app shows (status messages, notices,
  the chat's errors); the status bar counts the unread ones and its message goes after 10 s.
- A zone's grip is the whole gap beside it, lit by a thin line while hovered or held; a double click
  gives the zone its default size back, and a side zone dragged nearly shut closes, keeping its size.
- Any number of chats per repo or folder (`Doc::Chat { repo, n }`), each a tab named by its first
  message; + opens another. Chats ends, archives or deletes an active one. The bar under the box
  shows Claude (plan in the tooltip), the mode in a menu, the model running (the last one before a
  session says), then context and limits on the right.
- Polish before 0.5.0: the status bar shows repo and branch, Events filters errors and warnings,
  the sidebar filters repos, Settings has a section nav and a Keymap page; palettes fit the window,
  recent files persist; an empty chat lists recent conversations, chat tabs mark working or waiting,
  a system notice when a chat needs you away; Chats searches. A restore re-checks the worktree
  before it writes, and "Always allow" keeps its rule to the session.

### 4. Plugins from a folder

- Manifest with themes and TUI tabs through the terminal.
- Manifest over a JSON CLI: `source` fills a table, `actions` run commands, `confirm` before destructive
  ones.
- One honest consent, "this plugin runs code as you", shown again when its files change. No per-permission
  screen: without a sandbox it would imply a limit that isn't there.
- Terminal env opt-in per tab: `session::start` takes it as a parameter; TUI plugin tabs never get it.
- `keymap.rs` and `actions.rs` open up: `ACTIONS` stays for built-ins, a runtime table takes namespaced
  ids from manifests, unknown ids are refused softly, one path rebinds without a second keystroke
  observer. CLAUDE.md's key rule and docs/KEYMAP.md change with it.

Done when: a `local-postgres` manifest lists databases from `pg-dev` and drops one after a confirm, in a
test on a fake CLI.

### 5. DHP plugin

Private, installed from a folder. `wt` + `pg-dev` + `redis-dev` as an environment provider, which
also brings the "worktree / current folder" choice to new chat sessions: the default provider adds a new
write, `git worktree add`, and a worktree's path has to reach `discover`. Infra, Services and Board are
step 4's CLI tables in this plugin; Board stays out of the public client. The session card lists what deleting
a worktree removes, and the `.env` that `wt` copies falls under step 1's deny rule.

Done when: the owner starts a session in a new worktree with its own DB from the app and removes it again.

## Later

In docs/BACKLOG.md until something real asks for them: theme editor, icon packs, `px()` migration and
density, plugin catalog, install with account tokens, process plugins with host widgets, local env store.

Open, and not blocking step 0: whether chat goes into public releases (decides per-OS sidecar builds).
