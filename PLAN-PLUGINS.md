---
covers:
  - crates/core/src/plugins.rs
  - crates/core/src/bin/fake_pg.rs
  - crates/kernel/src/plugins.rs
  - crates/app/src/plugins
  - crates/app/src/actions.rs
  - crates/app/src/keymap.rs
---
# Detached — plugins plan

Plugins from a folder: a manifest that adds themes, tabs over a JSON CLI with actions, TUI tabs
through the terminal and a worktree recipe, each install global or a workspace's own.

**Status (2026-11-23): settled draft, nothing built. The draft's five questions and the review's
findings are decided below by the owner's delegation; the plan waits for his acceptance.**

Decisions, 2026-11-23:
1. **Install copies the folder.** One model for a folder and a later catalog install; the hash
   stands for files the app holds, and a `git pull` or a build in the source cannot turn a plugin
   off mid-work.
2. **Scope is picked at install, not in the manifest.** The author cannot know the user's
   workspaces, and Settings split by scope (the Threads direction) has "Add a plugin…" inside each
   scope.
3. **Every plugin command, a TUI included, gets the env a defined process gets,** the login env,
   without `DETACHED_*`. The consent already says "anything your account can"; an allowlist would
   imply a limit that is not there, and a CLI must answer as it does in the owner's terminal.
4. **No plugins shared through a repo.** A pull would bring code that runs as you; a team shares
   the folder and each member installs it.
5. **Agents get nothing of plugins.** Reading a table runs its `source`: a process start at an
   agent's request, not a read. Asked again once the DHP Services table is in use.
6. **No rebind.** The owner's "one path" is `keymap::bind`, which runs once, at launch, as today;
   a chord for a plugin id binds there whether or not its plugin is loaded. GPUI cannot unbind one
   key, and binding the kit's keys again after a clear is unproven.
7. **No watcher.** Core re-checks the hash before every spawn, so the consent does not depend on an
   event arriving.
8. **The DHP environment is a managed worktree (H6) whose recipe the plugin brings,** not a second
   worktree path through `wt new`: hooks stay off and the write list stays as it is (§12, P3).
9. **P0 is core alone**, and proves that nothing of a plugin runs without the owner's yes to its
   current files.

From the owner (docs/BACKLOG.md's rows "Plugins from a folder" and "DHP plugin", taken from
PLAN-EXTENSIBLE's steps 4 and 5 by the [cleanup decision](docs/decisions/2026-11-17-cleanup-before-0.6.0.md)):
1. Each plugin is global (all workspaces) or a workspace's own, installed and turned on per scope.
2. One consent, "this plugin runs code as you", shown again when its files change. No
   per-permission screen: without a sandbox it would imply a limit that is not there.
3. A plugin's TUI tab never gets the terminal's env (the owner socket, `DETACHED_SOCKET`).
4. `ACTIONS` stays for built-ins; a runtime table takes namespaced ids from manifests, unknown ids
   are refused softly, one path rebinds.
5. Done when a `local-postgres` manifest lists databases from `pg-dev` and drops one after a
   confirm, in a test on a fake CLI.
6. The DHP plugin is private, installed from a folder; Board stays out of the public client.

"Plugin" here never means the per-session Claude plugin folder of PLAN-AGENT-HOST §11
(`cache/plugins/<id>/`, the `detached:` skills); the code calls ours `core::plugins` and theirs
stays inside `core::claude`.

## 1. What a plugin is

A folder with a `plugin.toml` at its root and the files it names. Detached reads the manifest,
registers what it contributes and runs the commands it lists, as the user. Nothing in it is
compiled into or loaded by the app: no dylib, no script engine.

- **Code or not.** A plugin that lists no command (themes only) runs nothing and turns on without
  the consent; one that lists any command needs it (§5).
- **Core runs only what the manifest lists:** a source, an action, a TUI, the recipe's two commands.
  No call takes an argv from the app.

## 2. The manifest

TOML, as process definitions and the worktree recipe are; a theme stays the JSON of docs/THEMES.md.

```toml
detached    = 1                  # the manifest's schema; a newer one is refused: "needs a newer Detached"
id          = "local-postgres"   # [a-z0-9-]{1,40}, also its folder's name
name        = "Local Postgres"
description = "Databases of pg-dev."
version     = "0.1.0"
themes      = ["themes/dusk.json"]

[[tabs]]                      # a table over a JSON CLI
id      = "databases"
title   = "Databases"
source  = ["pg-dev", "list", "--json"]       # prints a JSON array of objects
columns = [["name", "Database"], ["owner", "User"], ["size", "Size"]]

[[tabs.actions]]
id      = "drop"
title   = "Drop"
run     = ["pg-dev", "drop", "--", "{name}"]  # {key}: that field of the selected row
confirm = "Drop {name}? Its data is gone."    # present: a confirm before it runs

[[tabs]]                      # a TUI through the terminal
id    = "psql"
title = "psql"
tui   = ["psql"]
```

- **Unknown keys are refused** (`deny_unknown_fields`): a typo is an error, and a new key comes
  with `detached = 2`. Tab and action ids are `[a-z0-9-]{1,40}`, unique in their plugin and tab.
- **A tab is a table or a TUI:** exactly one of `source` and `tui`.
- **Commands are argv, never a shell line.** The program is:
  - a bare name, found on the login PATH (`core::env::which`) in an absolute entry only, so a file
    of that name in the command's folder is never run;
  - or a path with a `/`, relative to the plugin's folder: no `..`, not absolute, and the file is
    there at install.
- **`{key}` is replaced in an action's `run` and `confirm` only;** a `source` and a `tui` are
  literal, so `docker ps --format {{json .}}` stays as written. In `run`:
  - never in the program;
  - once: a value that holds `{other}` is not expanded again;
  - inside its argument, so a value with spaces or `;` stays one argument;
  - a string, number or boolean as text; a missing key, `null`, an array or an object refuses the
    action, naming the key;
  - **no option from a row:** an argument that starts with a placeholder and whose value starts
    with `-` refuses the action unless a literal `--` comes before it. `--name={name}` is safe as
    written.
- **The table:** `source` runs in the tab's folder (§7) and prints an array of objects, 8 MB at
  most. A cell is one line of plain text: a string as is, anything else as compact JSON. An action
  whose `run` names no `{key}` is a tab action, in the tab's toolbar; others act on the selected
  row. The confirm shows the manifest's text and, under it, the argv that will run. After an action
  the source runs again.
- **No default chords** in a manifest; the owner binds them in `keymap.json` (§7).

## 3. Where plugins live

| Scope | Folder | Its entry in `settings.json` |
| --- | --- | --- |
| global | `config/plugins/<id>/` | `plugins.global.<id>` |
| a workspace's own | `data/workspaces/<ws id>/plugins/<id>/` | `plugins.workspaces.<ws id>.<id>` |

- Global plugins are personal assets, so the config tier, beside `themes/`
  ([storage scopes](docs/decisions/2026-11-12-storage-scopes.md)). A workspace is not a repo, so
  its plugins go in the workspace's own folder (PLAN-WORKSPACES §1), with its chats.
- **The `plugins` block** is core's, through `change_block`:

  ```json
  "plugins": {
    "global":     {"local-postgres": {"on": true, "hash": "…", "source": "/Users/me/dev/local-postgres"}},
    "workspaces": {"3f9a2c1e": {"dhp": {"on": true, "hash": "…", "source": "/Users/me/dev/dhp-plugin"}}}
  }
  ```

  `hash` is the accepted one, absent until the first consent.
- **One id in both scopes:** in that workspace its own install is the one that loads, on or off,
  and the global one is shadowed there, as a store item is `overridden`; the page says so. A chord
  bound to `plugin.<id>.…` follows whichever is loaded.
- **On and off belong to the install.** A global plugin is on or off everywhere; a workspace has
  no switch for it.
- **A deleted workspace's** entries are ignored and leave at the block's next write; its folder
  stays for the archive and loads nothing.
- **A folder dropped in by hand** is listed, off, at the next reload, until it is turned on.

## 4. Core

`core::plugins`, blocking, called through `blocking::run`:

```rust
pub enum Scope { Global, Workspace(String) }
pub enum State { Off, On, Consent, Broken(String) }   // Consent: on, its files not the accepted
pub struct Plugin {
    pub manifest: Manifest,   // Broken: only the id, the folder's name
    pub scope: Scope,
    pub dir: PathBuf,
    pub hash: String,
    pub state: State,
    pub shadowed: bool,
}
pub type Row = serde_json::Map<String, serde_json::Value>;
pub enum Cmd<'a> { Source(&'a str), Action(&'a str, &'a str, Option<&'a Row>), Tui(&'a str) }

pub fn list(state: &AppState, ws: Option<&str>) -> Result<Vec<Plugin>>;
pub fn install(state: &AppState, from: &Path, scope: &Scope) -> Result<Plugin>;   // a copy, off
pub fn accept(state: &AppState, p: &Plugin) -> Result<()>;   // refuses a folder that is not p.hash
pub fn set_on(state: &AppState, p: &Plugin, on: bool) -> Result<()>;
pub fn remove(state: &AppState, p: &Plugin) -> Result<()>;
pub fn spec(state: &AppState, p: &Plugin, cmd: Cmd, cwd: &Path) -> Result<proc::Spec>;
pub fn run(spec: &proc::Spec, cancel: &Cancel) -> Result<Output>;   // stdout, stderr's tail, exit
```

- **`spec` is the only way to a plugin's command.** It reads the folder again, refuses unless the
  plugin is on and the folder hashes to the accepted hash, and takes the argv from the manifest it
  just read, never from `p`. Then §2's program and `{key}` rules and §6's env. The terminal starts
  a TUI from the same `Spec` (P2).
- **`run`** spawns through `proc::Proc` (its own process group, a job on Windows), closes stdin,
  honors the cancel and ends a command whose stdout passes 8 MB.
- **Install** reads `from`, canonicalized, as `store::read` does: a link that leaves the folder
  and a special file are refused, a root `.git` is left out, and more than 8 MB is refused, here
  and at every later read. It validates the manifest and publishes the copy atomically with its
  file modes, off, with `source` recorded. An id already in that scope is refused in P0; P1 makes
  it Update: the copy is replaced, the entry stays, and a changed hash asks the consent again.
- **Turn off** keeps the hash: on again with the same files asks nothing.
- **Remove** turns it off, deletes the copy and its entry; `source` is not touched.
- **Broken** (a manifest that does not parse, a folder not named by its id, a program or theme file
  missing, a link out) is listed with why and registers nothing.

## 5. Consent

- **One dialog** for a plugin that lists a command, at Turn on and at Review after its files
  changed: "Local Postgres (`local-postgres`, global) runs code as you. It can read, change and
  send anything your account can." Below it every argv of the manifest, element by element, as
  facts, not switches. Allow or Cancel.
- **The hash** is the store's (`store::read`, `store::hash`) over the plugin's folder: file paths
  and bytes, a link inside by its target text
  ([decision](docs/decisions/2026-11-16-skills-with-links.md)).
- **Allow accepts the hash the dialog was built from;** files that moved since refuse it.
- **Files change** (an edit in the folder, an Update): the next `spec` refuses, the plugin lists as
  `Consent`, and its tabs show "Local Postgres changed" with Review. A TUI already running goes on.
- **What it does not cover,** said in the dialog's last line: programs on PATH it calls (`pg-dev`,
  `psql`) are the user's own and change without a new consent.
- **What it is not:** a defense against a process already running as the user, which can write the
  folder and `settings.json` alike. It catches a change the owner did not review.

## 6. Env

A source, an action, a TUI and a recipe command get the same env: what `processes::start` gives a
defined process (the login env, core's PATH and variables), with every `DETACHED_*` name removed;
for a TUI also `TERM` and `COLORTERM`, as the terminal sets them.

- **The owner socket** (the owner's rule 3): no `DETACHED_SOCKET`, and a plugin's PTY never joins
  `owner::Tabs`, which is what the socket checks; the variable is only its address.
- **No env-store profile** in these steps.
- **The command's folder may be a repo the owner does not trust,** or a read-only one. A tool that
  reads config from its folder reads the repo's, as it does in his terminal.

## 7. Contributions and the registries

`kernel::plugins::get(cx, ws)` is a `Resource<Vec<Plugin>>`: the global plugins and the current
workspace's, reloaded after each job of §4, on `Workspace::Switched` and when the Plugins page
opens. The app's `plugins/` feature has one `register(cx)` and follows that resource; a plugin
registers only while it is on.

| Contribution | Registers into | Id |
| --- | --- | --- |
| a theme | the theme loader's list, beside `config/themes/*.json`; Appearance shows it with the plugin's name | the theme's `name` |
| a tab | one document kind, `plugin` (`dock::register_doc`), category `code` | `Doc::Plugin` |
| opening a tab | the runtime action table | `plugin.<plugin>.<tab>` |
| a tab action | the same table, raised while its tab is active (`actions::raise`) | `plugin.<plugin>.<tab>.<action>` |
| the Plugins page | `settings::register_page`, group App | `plugins` |

- **The document.** The kernel's `Doc` gains its one variant for a plugin (PLAN-FOUNDATION §9):
  `Doc::Plugin { plugin: String, tab: String, cwd: String }`, kind `plugin`. `cwd` is the current
  repo when the tab opens, else the workspace's first folder, else the home folder; every command
  of the tab runs there. A saved tab after a restart runs its source again, or starts its TUI anew,
  like a docked terminal; when its plugin is off, changed or gone the tab says so and runs nothing.
  A table's rows are a kernel `Resource` keyed by plugin, tab and `cwd`; Refresh calls `refresh()`.
- **No app (zone panel) and no shell slot from a plugin:** a tab opens in the center and can be
  dragged to a zone like any tab.
- **Actions.** `ACTIONS` stays the built-ins' append-only list. The runtime table holds the loaded
  plugins' ids and titles under a `Plugins` group; `actions::find` and `all` read both, so the
  palette and the Keymap page list them. A runtime id is interned (one leaked `&'static str` per
  distinct id), so `Action`, `Run` and the keymap's `Bound` keep their types.
- **Keys.** `keymap::bindings` takes an override whose id is in `ACTIONS` or has the shape
  `plugin.<id>.<tab>[.<action>]`, and `bind` binds it at launch with the others. Pressing one whose
  plugin is not loaded says so in the status bar, as an unregistered built-in does. Overrides are
  tried after the defaults, so a built-in keeps its chord until its own override frees it. An edit
  to `keymap.json` takes a restart, as today.
- **Workspace plugins** register while their workspace is current; a switch drops their
  `Registration`s. Their tabs belong to that workspace already (W4).
- **Themes load from global installs only:** the theme is one app setting. A workspace install's
  themes are not listed, and its row says so. On a name the user's `themes/` wins, as it does over
  a bundled theme today, then the bundled themes, then plugins by id.

## 8. The Plugins page

Settings › App › Plugins: one list view per scope, shown as two sections, **Global** and **This
workspace** (its name). When Settings split by scope (the Threads direction), each section moves
under its scope and the workspace one shows the global rows greyed, "From Global".

- A row: name, description, version, what it adds (2 themes, 1 table, 1 TUI), its state, a switch,
  and a menu: Update from folder, Open folder, Remove. A Broken row says why, a `Consent` row has
  Review, a shadowed row says by what.
- **Add a plugin…** in each section: a folder picker, the install into that section's scope, then
  the consent when it lists a command.
- Remove asks first and says the copy is deleted; a plugin with no `source` loses its only copy.
- The page reloads its own copy in the job that writes it, as every Settings page does.

## 9. Windows and WSL

- A bare name is an `.exe` on PATH (`env::which` adds the suffix, not `PATHEXT`); a path in the
  plugin's folder names its `.exe`. A `.cmd` or `.bat` is refused: cmd's quoting is not argv's.
- No WSL place for plugins: a command is a native process. A manifest that names `wsl.exe` passes
  its arguments through `wsl.exe`'s own quoting, which Detached does not vouch for.
- TUI tabs run on ConPTY through `portable-pty`, as the terminal does.
- The store makes no link on Windows, so installing a plugin with a link inside is refused there
  with why, as a skill's copy is.
- Folders: `%APPDATA%\com.w-devs.detached\plugins\` and
  `%LOCALAPPDATA%\com.w-devs.detached\workspaces\<id>\plugins\`.
- `fake_pg` is a Rust binary, so the tests run on the three OSes; one that cannot is `#[cfg(unix)]`
  and a TECHDEBT row.

## 10. Written rules this changes

Each in the step that makes it true:
- CLAUDE.md: `crates/core`'s list gains `plugins` (P0); Keys: "every built-in action has an id in
  `actions.rs`; a plugin's are `plugin.<plugin>.…` in the runtime table, with no default chord"
  (P1). The write list stays: a plugin's command is code the user runs, like his terminal, and P3's
  worktree is a managed one.
- docs/KEYMAP.md: a paragraph on plugin ids and their overrides (P1).
- docs/THEMES.md: themes from global plugins, and which wins on a name (P1).
- PLAN-FOUNDATION §9: the plugin's `Doc` variant is this plan's (P1).
- docs/BACKLOG.md: "Plugins from a folder" and "DHP plugin" leave for this plan when it is
  accepted; it gains "Plugins: an env-store profile, scripts and `.cmd` on Windows".
- README, when P1 ships: plugins, the consent, where they live.
- docs/decisions/2026-11-23-plugins.md when the owner accepts: the Decisions above.

## 11. Order

| # | Step | Needs | Sees |
| --- | --- | --- | --- |
| P0 | `core::plugins`: manifest, scopes, install, consent, `spec` and `run`, on `fake_pg` | — | nothing |
| P1 | `kernel::plugins`, the Plugins page, a CLI table tab, runtime actions, themes | P0 | plugins with tables and themes |
| P2 | TUI tabs | P1 | a TUI in a tab |
| P3 | A plugin's worktree recipe, a new chat in a new worktree | P1, PLAN-WORKSPACES W5 | a chat in a new worktree with its own DB |

P2 and P3 do not need each other; P3 goes first if W5 is in.

## 12. Steps

Each step passes the host plan's gate: `make check`, a revmux round (claude-only, opus high)
without blocking findings, the owner's check on macOS. A change outside the feature's own files
(`actions.rs`, `keymap.rs`, the kernel's `Doc`, `terminal::session`, the theme loader,
`git::worktree`) is its own commit with its reason.

### Step P0. Core

Work, in `crates/core` alone: `plugins.rs` (§2–§6) with `list`, `install`, `accept`, `set_on`,
`remove`, `spec`, `run` and the `plugins` block; `bin/fake_pg.rs`, a scripted `pg-dev` that keeps
its databases in `dbs.json` in its folder: `list --json`, `drop <name>`, `args …` (prints its argv
as JSON), `env` (the names in its env), `sleep`; `testutil` builds it once, as `fake_claude`; the
`local-postgres` folder in `crates/core/tests/fixtures/plugins/`. No kernel, no app, no Update, no
TUI, no recipe.

Tests, next to the code, on a `Sandbox`:
1. **Manifest:** the fixture parses whole; refused with why: an unknown key, `detached = 2`, a bad
   or duplicate id, a tab with both or neither of `source` and `tui`, a `{key}` as a program, an
   absolute or `..` program path, a program file that is missing.
2. **Install:** copies without the root `.git`, keeps the executable bit (Unix), stays off, records
   `source`; a link out of the folder, a special file (Unix), an id already there and a folder over
   8 MB are refused and leave nothing behind.
3. **Scopes:** one id installed globally and in a workspace: `list` marks the global one shadowed
   there and not in another workspace, which never lists the first one's plugin; an entry of a
   workspace that is gone is ignored and leaves at the next write.
4. **Consent:** a themes-only plugin turns on without `accept`; one with a command is `Consent`
   until accepted, and `spec` refuses; an edit to any file after the accept refuses `spec` again,
   and `accept` with the stale `Plugin` refuses; off and on with the same files asks nothing;
   `spec` on an off plugin refuses.
5. **Arguments,** read back from `fake_pg args`: `{name}` as `a b; rm -rf /` is one argument;
   `--all` refuses where its argument starts with the placeholder, and passes after `--` and inside
   `--name={name}`; a value holding `{other}` is not expanded; a missing key, `null`, an array and
   an object refuse, naming the key.
6. **Programs:** a bare name is found on the sandbox's PATH; with `.` on PATH, a `fake_pg` in the
   command's folder is not run; on Windows a `.cmd` is refused.
7. **Env:** `fake_pg env` sees a variable of the login env and neither `DETACHED_SOCKET` nor
   `DETACHED_CONFIG_DIR` planted in the parent.
8. **Run:** `list --json` returns the fake's databases; a failing command returns its exit and
   stderr's tail; a cancel ends `fake_pg sleep`; stdout past 8 MB ends the command with an error.
9. **Remove:** the copy and the entry are gone, `source` is untouched.

Done when those nine groups pass in Check on macOS, Linux and Windows (a `win/plugins-p0` push runs
the Windows job), `make check` is green, and the diff touches only `crates/core` and CLAUDE.md's
crate list. That proves the three things every later step stands on: no command of a plugin runs
unless its current files are the accepted ones, a row's value cannot become a program, a second
argument or an option on any OS, and a workspace sees its own plugins and the global ones alone.

### Step P1. The page, a CLI table, themes

Work: Update in `install`; `kernel::plugins` and the rows' resource; the Plugins page (§8) with the
consent dialog; `Doc::Plugin` and the `plugin` kind; the table view with its toolbar, row actions,
confirm and Refresh; the runtime action table and plugin ids in `keymap::bindings`; themes into the
loader; KEYMAP.md, THEMES.md, CLAUDE.md, README.

Tests, `crates/app/tests/plugins.rs` with `fake_pg` first on the sandbox's PATH as `pg-dev`:
install `local-postgres` from its folder, consent, open `plugin.local-postgres.databases` from the
palette, see the fake's databases, Drop one, Cancel the confirm and nothing changes, confirm and it
is gone from the table and from `dbs.json`; an edit to the copy shows Review and refuses Drop until
allowed; Update from a changed source asks the consent again; Turn off leaves the tab saying so; a
workspace plugin's actions leave the palette on a switch and its install shadows the global one
there; a `keymap.json` chord for a plugin id opens its tab, and says "not available" while the
plugin is off; a theme from a global plugin is listed in Appearance and applied, one from a
workspace install is not. Kernel: the list reloads after each job and on a switch.

Done when a `local-postgres` manifest lists databases from `pg-dev` and drops one after a confirm,
in a test on a fake CLI (the owner's rule 5), and a plugin's theme applies.

The owner checks: his `local-postgres` on his `pg-dev`, a chord of his own for it, Remove.

### Step P2. TUI tabs

Work: `terminal::session::start` takes a `proc::Spec` in place of the shell, and such a session
does not join `owner::Tabs`; the `plugin` kind's TUI tab; restore after a restart; `fake_pg tui`
(a prompt that echoes lines).

Tests: a TUI tab runs `fake_pg tui`, takes typed input and shows the echo; its env has no
`DETACHED_SOCKET`, and a process in it that connects to the owner socket by its path is refused;
closing the tab ends the command, and Turn off ends it too; a restart starts it anew in its `cwd`;
a changed plugin leaves a running TUI alone and refuses a new one.

Done when `psql` from `local-postgres` runs in a tab on macOS, and the tests pass on the three OSes.

### Step P3. The DHP plugin

The plugin itself is the owner's, private, out of this repo: its Infra, Services and Board tables
(P1) and the recipe below over `pg-dev` and `redis-dev`. H6 already makes a worktree as `wt new`
does, with hooks off, and runs a recipe's install and teardown; this step lets a plugin bring the
recipe and a new chat start in such a worktree. H6's open item, the DHP recipe against `wt new`,
closes here.

```toml
[worktree]                # the recipe for repos that have no worktree.toml of their own
install  = ["bin/up"]     # in the new worktree
teardown = ["bin/down"]   # in it, before it is removed
```

- **Which recipe:** the repo's own `worktree.toml`, else the workspace's plugin's, else a global
  plugin's, the first by id. `git::worktree::create` and `remove` run a plugin's two commands
  through `spec` (`Cmd` gains `Install` and `Teardown`), so a plugin that is off or not consented
  refuses the create by name, as an unaccepted recipe does.
- **New chat's menu** (PLAN-WORKSPACES §4.1) gains "In a new worktree…" on a read-write project:
  it asks for a branch, creates the worktree and pins the chat to it.
- **The session card** of a chat in a managed worktree lists what Remove takes: the folder, the
  teardown's argv; the branch stays. Deleting the chat offers it, after a confirm.

Tests, on a fixture plugin whose install is `fake_pg create` and teardown `fake_pg drop`: a chat
started through "In a new worktree…" runs in the new worktree, with its database in `dbs.json`;
the card lists what goes; Remove drops the database and the worktree and keeps the branch; a
repo's own recipe wins over the plugin's; a plugin not consented refuses the create; a read-only
project is not offered one.

Done when the owner starts a session in a new worktree with its own DB from the app and removes it
again.

### Later

Not in these steps; each has its BACKLOG row: the catalog (the adapters catalog's fetch, verify and
extract code with its own key,
[decision](docs/decisions/2026-11-17-catalog-signed-in-ci.md); it installs into the same folders
and asks the same consent), install with account tokens, process plugins with host widgets
(`detached = 2`), an env-store profile per plugin, scripts and `.cmd` on Windows.

## 13. Risks

- **No sandbox.** A consented plugin is code as the user; the consent says so and nothing more is
  promised. The hash covers the folder, not the programs on PATH it calls.
- **A table's data is not the owner's.** Rows come from a CLI's output, which may name what others
  made (a branch, a database). §2's rules keep a value one argument and never an option; what the
  tool does with that argument is the tool's.
- **JSON CLIs drift.** A `source` that changes its output shows the error in the tab, never a
  crash.
- **Leaked ids.** An interned runtime id is never freed: a few dozen short strings per run.
