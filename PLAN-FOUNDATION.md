# Detached — foundation plan

The layer the agent host will stand on: a kernel crate with the app's live state, one owner for
settings, registries instead of closed enums, the updater as a Settings page, and one place that
decides what is stale, fed by a file watcher.

**Status (2026-11-11): accepted by the owner on 2026-11-10 after step 0 (step 1); steps 0, 0b, 1,
2, 3 and 4 done. §1–§11 were reviewed in seven revmux rounds
(`.revmux/tasks/agent-host-architecture/`, kept locally); the last found nothing major. §12, the
execution and acceptance, is written and not yet reviewed. Step 0b is on main (7e5a833); step 0
is answered (§12), §7 rewritten for Windows and Linux, and §4's two dock tests are on main
(faf27c8); the owner's own baseline is left. Steps 2 (the kernel crate), 3 (registries) and 4
(Settings) each had a revmux round with its findings fixed, passed the owner's check on macOS on
2026-11-11 and are on main. Step 5, Updates, is next.**

Owner's decisions, 2026-11-10:
- "No hurry; we do everything right, on the right architecture." The foundation comes before the
  features; shipping a feature early is not a goal.
- The plan is split. This document is the foundation. Providers and the session cut, the gateway,
  processes and the terminal crate, messaging, findings, reviews and plugins are a second
  document, written after this one passes review; §9 lists what it will ask of this one.
- The updater moves to an Updates page in Settings; settings get a page per module.

"Right" here: each piece of state has one owner; a second copy is allowed only as a read-only
snapshot with one stated way to refresh it; dependency directions are held by the compiler where
it can and by `make check` where it cannot, and the plan says which. Behavior stays as it is
today unless a section says it changes.

## 1. Crates

```
core    blocking drivers, as today; gains `watch` and `update`              no gpui
kernel  the app's live state and its exchange; draws nothing                gpui-pre, core
app     shell, dock, ui, feature views, registries of views; the binary     kernel, core, gpui-kit
```

`app → kernel → core`. Nothing points up.

- The compiler holds: the kernel cannot import a feature, `ui::theme` or a kit component.
- The compiler does not hold "draws nothing": `gpui-pre` is GPUI itself, with `div()` and
  `Render`. A lint in `make check` holds it (clippy's disallowed lists in
  `crates/kernel/clippy.toml`); step 0 proves the lint or replaces it.
- `gpui-pre` is pinned `=0.3.11`, the version the kit pins.
- `crates/core` keeps its name and every rule written about it.
- Amends PLAN-EXTENSIBLE's "No `crates/sdk`": the kernel is internal; the only public API stays
  the plugin manifest and protocol.

## 2. What the kernel is

The owner's definition: the asynchronous point where the apps exchange state, statuses, messages
and events. In code it is a set of GPUI entities on the foreground executor:

| Exchange | In the kernel |
| --- | --- |
| State | an entity; a view or another entity observes it |
| Event | a typed event an entity emits; subscribers react |
| Status of work | a job in `Activity` |
| Message | an inbox entity; second document |

- Git and forge tooling is not the kernel. It is core's drivers, reached only through
  `blocking::run` and `stream`.
- Kernel state lives on the main thread, in one copy; a view reads it in a frame.
- Stated exception: a buffer written by a driver's own thread (a terminal's emulator) stays behind
  that entity's lock. It is the second document's, named here so the rule is not read as absolute.
- "Views read the `Store`" becomes "views read kernel entities and their own state". A view may
  keep what exists only to draw (a laid-out text, a scroll position); it keeps no state the kernel holds.
- Kernel tests run next to the code in GPUI's test context, without a window, on GPUI's clock.

## 3. Entities of the foundation

| Entity | Holds | Was |
| --- | --- | --- |
| `Workspace` | discovered repos (what core's `list_repos` returns), the current repo, the document shown | `Store.repos`, `current`, `shown` |
| `Repo`, one per repo | status, branch names, compare base, tree ref, its last outcome | five tables in `Store` keyed by a path string |
| `Resource<T>` | a loaded value, the last error, when it loaded, a loading flag, a refresh policy (§7) | `Cache<T>` (forge only) and `last` fields in views |
| `Activity` | jobs, notices, the status message, the log the Events app shows | `Store.ops`, `notices`, `message`, `log`, `unread` |
| `Settings` | a read-only snapshot of core's settings (§5) | `Store.workspaces`, `Store.file_nesting` |
| `Update` | the updater's state (§6) | the `Update` global in `app/update.rs` |

- **Open documents stay the dock's.** PLAN-EXTENSIBLE: "The dock is the only record of what is
  open." The kernel holds only the one shown, which the dock sets when a tab comes on screen, as
  it sets `Store.shown` today.
- **A job is today's `Op`**, nothing more: what `run_op` starts, visible and cancellable. One
  record per job, in `Activity`: id, title, repo, progress. The id is core's op id and cancel is
  `Ops::cancel(id)`, as now (store.rs:256, 495, 533). Several jobs can run on one repo, as now;
  `Repo` holds none of them, and "the jobs of this repo" is a read of `Activity`.
- The update's download and install are not jobs: `Update` alone holds them (§6).
- A quiet read (a status, branch names, settings) is a plain `blocking::run` with no record and
  no op id, as now.
- **The log is unchanged**: errors and warnings only, capped at 500, counted in `unread`. Jobs
  are not written to it.
- `Ops` and `RepoLocks` stay in core.
- `Doc` moves from `store.rs` to the kernel as it is: a typed enum of plain data with serde and
  `PartialEq`. `Location`, `DiffDoc` and `DocRef`, defined beside it in `store.rs`, move with it;
  `files/api.rs` keeps re-exporting them, so the Files API's paths stand.
- `ui_state` moves to the kernel unchanged (it imports only `App` and `Global`).
- `nav` moves to the kernel in step 6: it needs only `App`, `Doc`, `ui_state` and the repo list,
  so a kernel entity can record a visit.
- `actions.rs` and `keymap.rs` stay in `app`: they need a window.
- The rest of `Store`, each with its home:

| Today in `Store` | Home | Step |
| --- | --- | --- |
| `core: Arc<AppState>`, read at about 36 call sites through `Store::global(cx).read(cx).core()` | a kernel global, `kernel::core(cx)`; `Store::core()` forwards to it until `Store` goes | 2 |
| `QueryId`, `query_id()` (PLAN-NATIVE Threads: "an id from the store") | the kernel, beside `blocking` | 2 |
| `register_actions`: `panel.*`, `zone.*`, `nav.*`, and the calls to `terminal::register` and `chat::register` | `dock` registers `panel.*` and `zone.*`; `nav.*` and each feature's own go to that feature's `register(cx)` | 3 |
| `name_of` | `Workspace` | 6 |
| `load_branch_names` | `Repo` | 6 |
| `refresh_all` and its coalescing | `Workspace` in step 6, then invalidation | 6, 7 |

- `Error` is not changed by this document.

## 4. Registries

All in `app`. The goal of step 3: no file in `dock/` and not `shell.rs` imports a feature, with
every UI test unchanged and green and a `dock.layout` saved by 0.5.0 loading without a placeholder.

**An app** registers what `dock::Kind` and `shell.rs` hold for it today:

| Field | Today | Example |
| --- | --- | --- |
| `name` | `Kind::name()`, what a saved layout knows it by | `changes` |
| `id` | `Kind::id()`, the tab's element id the tests use | `tool.Changes` |
| `title`, `action` | `Kind::title()`, `Kind::action()` | `Changes`, none |
| `home` | `Kind::home()`: the zone and size when a layout does not place it | right, 300 |
| `view` | `shell::app` | builds `files::Changes` |
| `shown` | `shell::shown`, optional | Terminal starts its first shell; Events clears `unread` |

Both strings are kept, so no layout resets and no element id changes.

**A document kind** registers an object with the behavior `DocTab` now matches on by variant:
the view; the tab's id, title and badge; what to raise when it comes forward; what happens when
it becomes active and when it is removed (a terminal's tab ends its shell); whether two docs are
the same tab; whether it shares the preview tab and how it replaces its content in place (the
Viewer). `DocTab` holds the `Doc` and that object and calls it. `Doc` stays a closed enum in the
kernel; a test asserts every kind has an object; a missing one shows today's placeholder.

**The terminal's own dock code** turns around, keeping what a drop does today:
- The payload becomes `dock`'s own type carrying a `Doc`. The terminal starts the drag with a
  `Doc::Terminal` built from the session's id and folder; starting a drag changes no state.
- Both drop targets stay, the center hints and every zone's tab strip (skin.rs:68-69, 400-402).
  They call `dock::drop_doc(doc, node)`, which is `dock_terminal` without the terminal: it calls
  the document kind's `adopt` hook, then adds the `DocTab` in the zone holding `node`, else the
  center, by today's lookup. `show_doc` is not used for a drop: it has no zone.
- The terminal's `adopt` is `Terminals::undock`: the session leaves the Terminal app at the drop,
  as now, so a drag released over no target changes nothing and no second shell starts.
- `close_terminal` becomes the terminal's call to the existing `close_docs(predicate)`.
- `tests/dock.rs` drops a shell only on the center today. Two tests are written first, on
  today's code: a drop on a zone's tab strip lands in that zone, and a drag released over no
  target leaves the shell in the Terminal app.

**The shell's slots** get registries too: `shell::top_bar(slot, order, view)` for what `top_bar`
lists by hand, `shell::overlay(view)` for the palette, find and the Settings layer, and
`history::init` moves into history's own `register(cx)`.

Settings pages are §5; actions stay as they are. A feature still calls another only through its
`api.rs`. `main` lists the features, each with one `register(cx)`.

## 5. Settings

**Owner: core.** `AppState.settings` is read by drivers on worker threads (accounts, forge
project lookup, `list_repos`). Every write but one goes through `AppState::update`, which saves
the file under the lock and changes memory only when the save worked.

- The one other writer is `accounts::remove` (accounts/mod.rs:90-96): it locks the settings,
  edits them and saves by itself, memory first. Step 4 routes it through `update`. After that
  `update` is the only writer, and a test fails on any `save` outside it.
- The kernel's `Settings` is a snapshot for the main thread, never edited. `AppState` gains a
  revision bumped in `update`; after every `blocking::run` that returns, the kernel compares it
  and reloads the snapshot on a worker when it moved. Tests: the snapshot changes after an
  account is removed, and after each of `update`'s callers.
- A view changes a setting by a call to core through `blocking::run`. `Store.workspaces` and
  `Store.file_nesting`, today's copies reloaded by hand (store.rs:465-477), go.
- **Blocks without a format change.** Today's top-level keys (`workspaces`, `repoAccounts`,
  `forgeAccounts`, `mrRemotes`, `gitPath`, `fileNesting`) stay where they are: the README's
  `fileNesting` holds and no migration is needed. A new module adds one top-level key of its own.
  Core's `Settings` gains a flattened map of the keys it does not know, so no writer drops them;
  `block::<T>(name)` reads one and `set_block(name, value)` writes one through `update`.
- The legacy `folders` and `excludedRepos` are not unknown keys: `load` removes them from that
  map once it has made the Default workspace from them, as a save drops them today. Test: a file
  with them, loaded, every workspace removed, saved and loaded again, has no workspace.
- Tests: today's file loads and saves with every current key intact; an unknown key survives a
  save by each writer.
- Stated limit: a build older than this drops unknown keys on its next save. Only blocks that
  build never had are lost.
- **Pages.** `settings::register(id, title, group, view)`;
  `repos::open_settings(cx, Section)` becomes `settings::open(cx, id)`. Settings become a feature
  of their own, `settings/`, holding the layer and the registry; `repos/settings.rs` (1175 lines)
  splits by owner: Accounts, Repository and Git pages stay with `repos`, Appearance goes to `ui`,
  Keymap to the keymap's owner, Updates to the updater. Groups: the app, modules, the current
  repo. Settings stay a modal layer. No new option comes with this document.

## 6. Updates

- `core::update`: check, download to a temp file with a progress callback, verify, apply; moved
  from `app/update.rs` with its tests and local test server.
- `Update` in the kernel: idle, checking, up to date, offered, downloading with a percent,
  verifying, ready, installing, failed with its text.
- **`Update` is the one owner** of the percent and of the cancel flag, which is the updater's own
  `Cancel`, as `Update.installing` is today. Neither the download nor the install appears in
  `Activity`, so the status bar's op rows never show them; the marker below is their only place
  there, and the page's button their only cancel.
- Cancel exists only while downloading. A second download or install cannot start while one runs.
- **The swap happens at "Restart now", not before.** `replace_app` moves the running bundle into a
  temp folder that is deleted when it returns, so an app that kept running after the swap would
  run from a deleted bundle. Ready means "downloaded and verified"; the click swaps, starts the
  relaunch helper and quits, in one call with no cancel. Windows and AppImage the same way.
- A `.deb` install stays as it is: it never downloads. The page says the package manager updates
  it and offers the releases page.
- "Restart now" names the live chats and shells first (the page reads the app's `Chats` and
  `Terminals`).
- Checks: at launch and daily, and they go on after an offer; a manual check always reports; a
  failed background check is silent in the status bar and shown on the page.
- The status bar keeps a marker that opens the page and shows the percent.
- Tests: progress and a cancel in mid-download against the local server; a second click starts
  nothing; a cancel leaves no temp file; the page in a UI test. `tests/update.rs`'s
  `a_click_cancels_a_running_install` asserts today's bug and is rewritten.

## 7. Invalidation

One place in the kernel decides what is stale. Its inputs:

| Input | Marks stale |
| --- | --- |
| The watcher: a repo's git files (below) | that `Repo`'s status and branch names |
| The watcher: the current repo's worktree | its changes and tree |
| A job's end | what the job declares (a repo's status after an op) |
| Window activation | the rescan and every status, as today; forge resources older than 30 s, as `Cache` does (`FRESH`) |
| Time | a `Resource` whose policy says so |

- A `Resource<T>` has a policy: `fresh_for`, and an optional `poll` period with its condition (5 s
  while a pipeline runs; 60 s for the MR list).
- **Polling keeps today's condition.** `forge::poll` runs for as long as its view lives and ticks
  only while the window is visible (forge/mod.rs:155-176); a tab hidden behind another keeps
  polling. So: a view holds an `Interest` in a resource for as long as the view lives, and a poll
  runs while any interest is held and the window is visible. The second document's `watch` holds
  one without a view.
- The polls in `forge/badge.rs`, `pipelines.rs` and `mr_list.rs` become policies with the same
  periods; the tests that drive them with `advance_clock` stay.
- A view may call `refresh()` for its button. It does not decide what is stale.
- A view's own query (a log page, find results) is not a resource.
- `refresh_all`'s lost call (docs/TECHDEBT.md) is fixed here.

**What the watcher watches.** For a repo, its git directory's `HEAD` and `index`, and its common
directory's `refs` and `packed-refs`. For a main repo both are `.git`. A linked worktree's `.git`
is a file naming `<main>/.git/worktrees/<name>` (discovery.rs `worktree_of`): its `HEAD` and
`index` are there, and its refs are the main repo's, which may lie outside every workspace
folder. A change under a common directory marks the main repo and each listed worktree of it.

Any other repo whose `.git` is a file (a submodule listed by itself, a `--separate-git-dir`
clone) has its git directory resolved from that file's `gitdir:` line and that directory's
`commondir`, not assumed to be `.git`. A repo whose directories cannot be resolved is not
watched and refreshes on activation, as today.

The watches are on directories, never on those files: git writes each as `X.lock` renamed over
`X`, and on Linux a watch on the file dies with the old inode. So: the git directory, not
recursive, taking events named `HEAD` and `index`; the common directory, not recursive, taking
`packed-refs`; its `refs`, recursive. Events for `*.lock` are dropped, and so are reads: inotify
reports a file opened for reading, and core's own `status` opens `HEAD` (step 0). Core reads with
`GIT_OPTIONAL_LOCKS=0`, so its statuses write no `index`; a `git status` the user runs in a
terminal can, and costs one refresh.

**On Windows** the watches above and the worktree's below are replaced by one recursive watch on
each workspace folder, filtered by the same rules (step 0): a handle open inside a repo stops its
folder from being renamed ("Access is denied"), while one on the workspace folder blocks only
renaming that folder. A linked worktree's common directory outside every folder gets its own
watches as above.

**The current repo's worktree** is the only worktree watched. How differs by OS, since only
inotify pays per directory:
- **macOS: one recursive watch on the worktree's root**, nothing per directory. Windows has the
  folder's watch above.
- **Linux: a watch per directory**, from a walk that follows the repo's ignore rules, with the
  `ignore` crate core's search already uses, and never enters `.git`, so `node_modules/`,
  `target/` and the like cost nothing. A cap of 10,000 directories on one worktree. A worktree over the cap is not watched at all and refreshes on window activation, as
  every worktree does today. A directory created later gains its watch when its creation is
  seen, unless it is ignored. The walk runs on a worker, never on the thread that reads events,
  and again when the current repo or one of its ignore files changes.
- **Every OS**: an event under an ignored path or under `.git` is dropped before invalidation, so
  a build writing into `target/` refreshes nothing. Git-directory events of other repos never
  wait for a worktree's setup.
- The git-directory watches above are three per repo on macOS and Linux (the git directory, the
  common directory, `refs`): 41 for 20 repos and a linked worktree in step 0.
- Invalidation runs one status per repo at a time, and once more after it when events came in
  the meantime, so a burst cannot queue refreshes.

`core::watch` wraps `notify` (8.2.1 is already in `Cargo.lock`): debounced batches over a channel
to the kernel.

## 8. What stays where it is

- The git runner, the forge clients, the token store, trust, the sidecar protocol.
- The dock on the kit's `DockArea` with our skin; themes; the keymap; the action registry.
- The chat and the terminal: their sessions stay globals in `app` until the second document.
- The UI tests on real git and the forge fake; they are what makes each step safe.

## 9. What the second document will ask of this one

Nothing below is built here; each is a reason for a shape above.
- New kernel entities beside these: `Session`, `Terminal`, `Proc`, `Inbox`, `Findings`.
- An `Interest` in a resource with no view (`watch`).
- Settings blocks for process definitions and env profiles.
- The watch list growing by the repos and trust files of live sessions.
- One more `Doc` variant, for a plugin's document.
- A read of what is open, which the dock answers, since it owns it.

## 10. Written rules this changes

CLAUDE.md:
- the crate list at the top gains `crates/kernel`;
- "Never call core on the UI thread …: go through `blocking::run`/`stream`": the path;
- "Views read the `Store` and their own state; they do not cache what the store already holds":
  kernel entities, with §2's wording;
- "UI state that persists goes through `ui_state`": the path;
- new: the kernel draws nothing and imports no kit component; core owns `settings.json`,
  `AppState::update` is its only writer, and the kernel's snapshot is read-only;
- Tests: kernel tests next to the code, in the test context without a window;
- Parallel work: "`shell` slot constructors" becomes the registrations of §4 and §5; "stub in the
  provider's `api.rs` or the store" becomes "or the kernel".

PLAN-NATIVE:
- Layout: `store.rs`, `blocking.rs`, `cache.rs`, `ui_state.rs`, `nav.rs`, `update.rs`, a new
  `settings/`;
- Stack's Auto-update row and Threads: the new homes; "an id from the store" becomes the kernel's;
- the Store section and contract: `current()`, `status(repo)`, `run_op`, `tree_ref`, `compare`,
  `shown()`, `show(Doc)` move to `Workspace`, `Repo` and `Activity`;
- the Contracts preamble: "a stub to the provider's `api.rs` or the store";
- the Verticals table: F owns `store`, `blocking`, `cache`, `ui_state`; the kernel takes their
  place under the same owner, and V2's and V5's needs name its entities;
- the Dock contract (`dock::open(Kind, …)`, `is_shown(Kind)`) and Shell slots (`dock::Kind`,
  `shell::app`, `Doc::{…}`, `TopBar`, `overlays`): names and registrations;
- the Repos API: `open_settings(cx, Section)`;
- the Files API: `Location`, `DiffDoc`, `DocRef` are re-exported from the kernel, unchanged;
- the UI contract: `ui_state::get/set`'s path; Navigation: `nav::open`'s path; the Actions
  contract: `dispatch` reports through `Activity`, not `Store::say`;
- Fixtures: `open_app(cx)` "with the store booted";
- "Carried over unchanged: `settings.json`" holds; a line about module keys is added.

PLAN-EXTENSIBLE:
- "No `crates/sdk`: built-ins stay on `Store` and the `api.rs` contracts" (§1, §3);
- "there is no file watcher" (step 1);
- "Built in the order that gets a usable feature first": this plan is the owner's stated exception.

docs/TECHDEBT.md: the update cancel row and the `refresh_all` row close with steps 5 and 7.

## 11. Order

One step at a time, in this order. "Sees" is what the owner notices. §12 has each step's work
and what makes it done.

| # | Step | Needs | Sees |
| --- | --- | --- | --- |
| 0 | Spikes and the baseline; answers written here | — | — |
| 0b | Builds and tests in a branch's worktree: `make warm`, `make ramdisk`, the cap on test threads, lighter sandboxes | — | the gate in about two minutes |
| 1 | The owner accepts this plan | 0 | — |
| 2 | `crates/kernel`: `blocking` with `QueryId`, the core handle, `ui_state`, `Activity`, `Doc` and its data types leave `app` | 1 | nothing |
| 3 | The registries of §4; `dock/` and `shell.rs` import no feature | 2 | nothing |
| 4 | Settings: one writer, unknown keys kept, blocks, the snapshot, the `settings/` feature and its pages | 3 | settings pages by module |
| 5 | `core::update`, `Update`, the Updates page | 4 | the page with progress; the cancel bug gone |
| 6 | `Workspace` and `Repo`; `nav` moves; `Store` goes | 5 | nothing |
| 7 | `Resource<T>`, policies, interests, invalidation by job end, activation and time; `Cache` and the views' polls go | 6 | nothing |
| 8 | `core::watch`; invalidation takes its events | 7 | changes appear by themselves |

## 12. Execution and acceptance

Sizes below are counts in the code on 2026-11-10, not time.

### The gate every step passes

A step is done when all of these hold, and it merges only then:

1. It was built in its own worktree and branch, `foundation/<n>-<name>`, warmed by step 0b's
   `make warm`, as ordered commits that each pass `make check`; the branch is rebased on main.
2. CI's Check is green on macOS, Windows and Linux: `make check` and `make smoke`.
3. No UI test loses an assertion or a step. A test changes only in the names and paths the step
   renames, and in what the step's own "Tests" lists. The 8,429 lines under `crates/app/tests`
   are the proof that behavior held.
4. Every test the step lists exists; one that pins a bug fails on main.
5. A revmux round on the branch, the `claude-only` profile on Opus high, has no major finding.
6. The owner ran the step's check list on macOS, in a build on a temp config directory first and
   then on the real one.
7. The lines of §10 the step makes true are edited in the same branch, this plan's status line
   says the step closed, and the docs/TECHDEBT.md rows it fixes are removed.
8. The user sees nothing but what the step's "Sees" names.
9. The step reports step 0's numbers again. One worse by more than a tenth goes to the owner
   with the figures, who decides.

A step that cannot pass is reverted as one merge; no step leaves a half that later ones need.

### Step 0. Spikes and the baseline

Throwaway code outside `crates/`, on a branch that never merges. Linux and Windows rows run on
CI runners through a workflow that lives only on that branch.

| Question | If | Then |
| --- | --- | --- |
| Does a crate on `gpui-pre` alone build and run tests with GPUI's test context and clock | no | the kernel depends on the kit and the lint also bans its components |
| Does a clippy disallowed list in `crates/kernel/clippy.toml` fail `make check` on `div()` and on a `Render` impl | no | a test that scans the crate's source, or the rule stays written only and §1 says so |
| Can a `dock`-owned drag payload replace `terminal::Dragged` in the skin's drop targets, with the center test and §4's two new tests green | no | the skin keeps that one import and §4's goal names the exception |
| `notify` on about 20 repos' git directories, a linked worktree among them, on macOS, Linux and Windows: limits; noise from `index.lock` and editors' atomic saves; three commits in a row each seen on every OS | too many watches or too noisy | only the current repo is watched; the others refresh on activation as today |
| The current worktree, on a repo with a filled `node_modules` and `target`. Linux: the count of watches against §7's cap and the user's inotify budget; a new directory is picked up. Every OS: a build into an ignored directory refreshes nothing | over budget, or the filter cannot hold | no worktree is watched; git directories only, and worktrees refresh on activation as today |
| Windows, under live watches: rename, move and delete a folder inside the current worktree; delete and rename a watched repo's own folder; a switch that removes directories; `git gc`, a branch delete that empties a folder under `refs`, `git worktree remove` | any of them fails | that watch goes on Windows, down to no watcher there, and Windows refreshes on activation as today |

Answers, 2026-11-10, from branch `foundation/0-spikes` (`spikes/`, and the Spike workflow on the
three OSes):
1. Yes. `gpui = { package = "gpui-pre", version = "=0.3.11", default-features = false }`, with
   `test-support` in dev-dependencies, builds no kit and no platform crate; `blocking::run` and
   `stream` copied over pass their tests, and an entity's job, event and a timer on GPUI's clock
   pass in `TestAppContext`. Nothing in §1–§11 changes.
2. Yes, with `#![forbid(clippy::disallowed_methods, clippy::disallowed_types)]` at the crate's root:
   without it a module's `#[allow]` switches the list off. `div()`, `Render`, `IntoElement` and
   `Window` each fail clippy. It holds under clippy, which `make check` runs, not under a bare
   `cargo build`. `Window` is on the list, so window activation reaches the kernel from `app`.
3. Yes. `dock::DragDoc { doc, title, adopt }` with `dock::drop_doc(drag, node)`: the skin imports
   no feature, the terminal's tab drags a `DragDoc` whose `adopt` undocks its session, and the
   dock's eight tests pass, the center test and §4's two among them. Found on the way, on main
   too: the center's drop target lies under the bottom zone, so a shell released over the
   Terminal app moves to the center (docs/TECHDEBT.md); §4's "no target" test drops on the sidebar.
4. Yes on the three OSes, once read events are dropped on Linux (§7 now says so): three commits
   in a row each seen and only in their repo; a linked worktree's commit marks it and its main
   repo; core's statuses mark nothing. 41 watches for 20 repos.
5. Yes. Linux: 42 watches for a worktree of 41 directories beside 500 ignored ones, which cost no
   watch and no event; the runner allows 655,360. The cap is 10,000 directories. A file made in a
   new directory before its watch exists is missed, but the directory's own event marks the repo,
   so the refresh sees it. macOS and Windows: a build into `target/` raised 2,102 and 3,052 raw
   events, all dropped by the filter.
6. The "Then" fired for the per-repo watches and was replaced, not taken: with them, renaming a
   watched repo's folder fails on Windows with "Access is denied" (deleting it works). One
   recursive watch on the workspace folder passed every row; §7 now says so. A directory's
   modified event can come late on Windows: one refresh too many, nothing lost.

The baseline, 2026-11-10, at 445c579 on the owner's Mac (10 cores), on a temp config directory
with 20 new repos, not the usual workspace; the owner's own run replaces the last three:
- `make app` binary: 90,047,072 bytes; its build from nothing 115 s.
- `cargo check --workspace --all-targets`: cold 85 s, one line in the app 2 s.
- `make check`: its tests alone took 172 s before step 0b; the whole gate in a warmed worktree
  takes 121 s after.
- Start to the first frame and exit (`--smoke`): 0.40 s, 1.82 s on the first, cold run.
- Memory after a minute: 109 MB.
- Git processes: 60 in the first minute (startup's statuses), none in the next five idle
  minutes with the window in the background.

Also in this step, on main, since they test today's code: the two dock tests of §4 (a shell
dropped on a zone's tab strip lands in that zone; a drag released over no target leaves the shell
in the Terminal app).

The baseline, on the owner's Mac with the usual workspace, from a `make app` build: release
binary size, a cold and an incremental `cargo check`, the time of `make check`, startup to the
first frame, idle memory after a minute, and the count of git processes in five idle minutes
(above, on a temp workspace).

Done when:
- every row has its answer written under it here, with what it changed in §1–§11;
- a "Then" that fired has rewritten the sections it names, before step 1;
- the two dock tests are on main and green on the three OSes;
- the baseline numbers are written here;
- the spike branch is deleted.

### Step 0b. Builds and tests in a branch's worktree

Ordinary commits on main, merged before step 2: every later step pays this in each commit, and
several branches pay it at once.

Measured on 2026-11-10 on the owner's Mac (10 cores), in throwaway worktrees at 445c579:

| What | Time |
| --- | --- |
| A new worktree, nothing built: clippy, core's clippy, the tests' build, the smoke build | 85 + 10 + 169 + 68 s; `target/debug` grows to 11 GB |
| The same where `target` is an APFS clone of a built one (`cp -cRp`: 5 s, no disk until they differ) | clippy 9 s, the tests' build 38 s, the smoke build 17 s; only the workspace's own crates compile |
| One line changed in the app and all its test binaries rebuilt; in core; in one test file | 7 s; 6 s; 1 s |
| `cargo test --workspace` | 172 s: the eleven UI binaries 141 s and core 11 s, one binary after another |
| One UI binary (`history`, 45 tests) at 1, 3, 5 and 10 test threads | 73, 30, 25 and 26 s by the clock; 48, 61, 82 and 142 s of system CPU; 19 to 21 s of user CPU |
| Every test binary started at once | 409 s, and two tests ran into `wait_until`'s 10 s |

What the numbers say:
- Compiling is cheap once a worktree has built. The first build is what a branch pays, and a
  clone takes four fifths of it away.
- The tests are what every commit pays. They spend several times longer in the system than in
  our code: git processes, and the sandboxes' deletion, which tops the profile of the test
  process itself. Past five tests at a time nothing gets faster, whichever process they run in.
- Two gates at once on the default thread count can fail each other's tests.

Done on branch `foundation/0b-local-builds`, merged to main as 7e5a833 on 2026-11-10 without a
revmux round, at the owner's word; `make check` green there in 121 s, `make smoke` and CI green
on the three OSes:
1. `make warm` (`scripts/warm.sh`): clones `target/debug` of the worktree that built last, not
   main's, whose `target/debug` holds little because the owner builds `make app` there. It then
   deletes our crates' fingerprints so they rebuild: file times alone could take the source's
   code. Checked: a test only in the new worktree runs there and not in the source. After it,
   clippy 44 s and the tests' build 31 s, against 85 and 169.
2. `.cargo/config.toml` sets `RUST_TEST_THREADS = "5"`. `GHOSTTY_VT_DIR` stays as it is:
   shared, the release's two macOS targets would run `build.rs` at once on one directory, which
   `build.rs` says fails once; the gain was seconds, and `make warm` clones the library anyway.
3. `make ui-test t=<file>` runs one test file.
4. Two and three `make check` at once in warmed worktrees, with the cap: every test passed;
   266 and 298 s for two, 428 to 451 s for three. No lock.
5. A UI test makes about 56 git calls (2,533 over `history`'s 45 tests: `log`, `rev-parse`,
   `commit`, `add`, `remote`, `status`, 174 `clone`s). Kept, in `git::testutil`: repos without
   git's template (a third of the files; the hook helper and one LFS test create their
   directory), and `find_git` once per binary. `cargo test --workspace`: 172 to 154 s.
6. `make ramdisk` (`scripts/ramdisk.sh`, once per boot, 1 GB): `make check` and `make ui-test`
   put the sandboxes there when it is mounted. `history` 24.6 to 15.6 s, `repos` 21.3 to 13.7 s;
   `cargo test --workspace` 130 s, all 562 tests green. Deleting the sandboxes on a background
   thread instead gave nothing (24.0 s): the cost is the file system's work, not the thread.

Left: a fixture repo built once per binary and copied per test, after the RAM disk the next
place the time goes; the CI run on three OSes.

Not done, and why:
- One target or build directory for every worktree, by symlink, `CARGO_TARGET_DIR` or
  `build.build-dir`: checked, cargo gives a workspace crate the same artifact in every worktree
  and judges it fresh by file times, so a worktree whose files are older than another's last
  build runs that other branch's code. A test that exists only in branch B passed in branch A.
- sccache: it gives what the clone gives, and adds a tool and a cache to keep.
- cargo-nextest, or the eleven test files as one binary: linking all eleven takes 7 s, and
  running more tests at once is what does not help.
- Dependencies without debug info: the cold build of the tests took 185 s with it off.
- The second build of GPUI that `make smoke` causes (the tests turn on the kit's
  `test-support`, the app does not): 68 s cold, 17 s in a clone, and CI runs it.

Done when:
- in a new worktree after `make warm`, `make check` compiles for under 90 s on the owner's Mac
  (measured: 88 s, `make warm` 6 s of it);
- `cargo test`, however it is started, runs at most five tests at a time per binary (done);
- point 4's answer is written here (done: no lock);
- point 5's findings are written here, and `cargo test --workspace` takes under 130 s (done:
  130 s with the RAM disk, 154 s without);
- the 231 UI tests and core's 146 still run and pass on the three OSes (done);
- CLAUDE.md's Commands and "Parallel work" name `make warm`, `make ramdisk` and `t=` (done).

The owner checks: `make ramdisk`; `git worktree add`, `make warm`, `make check` in the new
worktree; the whole gate takes about two minutes, not five.

### Step 1. The owner accepts the plan

PLAN-NATIVE's rule: changing a contract "takes a plan edit agreed with the owner". This step is
that agreement. No rule file changes yet: each line of §10 changes in the step that makes it true.

| §10's lines | Step |
| --- | --- |
| CLAUDE.md's crate list, the `blocking` path, the `ui_state` path, "the kernel draws nothing", kernel tests; PLAN-NATIVE's Layout, Threads, the UI contract, the Actions contract, the Files API's re-export; PLAN-EXTENSIBLE's "No `crates/sdk`" | 2 |
| CLAUDE.md's Parallel work registrations; PLAN-NATIVE's Dock contract and Shell slots | 3 |
| CLAUDE.md's "core owns `settings.json`"; PLAN-NATIVE's Repos API and "Carried over unchanged" | 4 |
| PLAN-NATIVE's Auto-update row; docs/TECHDEBT.md's update row | 5 |
| CLAUDE.md's "Views read the `Store`" and "or the store"; PLAN-NATIVE's Store section and contract, Contracts preamble, Verticals table, Navigation, Fixtures; PLAN-EXTENSIBLE's "built-ins stay on `Store`" | 6 |
| docs/TECHDEBT.md's `refresh_all` row | 7 |
| PLAN-EXTENSIBLE's "there is no file watcher" | 8 |

Done when:
- the status line says the owner accepted the plan, with the date and step 0's go;
- PLAN-NATIVE's Contracts and PLAN-EXTENSIBLE's decisions each gain one line naming this plan as
  the agreed change, and PLAN-EXTENSIBLE's "usable feature first" line names it as the exception.

### Step 2. The kernel crate

Size: 56 `blocking::run` and 3 `stream` call sites, 27 `ui_state::`, 36 `.core()`, 52 `Doc::`.

Work, a commit each:
1. `crates/kernel` on what step 0 answered, its lint config, its place in `make check`.
2. `blocking` and its tests, `QueryId`, and the core handle as `kernel::core(cx)`. Call sites take
   the new path in the same commit; `Store::core()` forwards.
3. `ui_state`.
4. `Doc`, `Location`, `DiffDoc`, `DocRef`; `files/api.rs` re-exports them.
5. `Activity`: `ops`, `notices`, `message`, `log` and `unread` leave `Store` with `run_op` and
   `say`; the status bar, the notices and the Events app observe it; `actions::dispatch` reports
   through it.

Tests, in the kernel: `blocking`'s own, moved; a job is recorded and removed; a cancel reaches
the running core op and it stops; the log keeps 500 and drops the oldest; `unread` counts and
clears; a dropped `QueryId` cancels its query.

Done when:
- `cargo tree -p detached-kernel` lists no kit crate, or step 0's other answer holds;
- a `div()` added to the kernel fails `make check`, by the means step 0 chose;
- `crates/app/src/blocking.rs` and `ui_state.rs` are gone, and `Store` has none of the five fields;
- nothing in `crates/kernel` names `Store`, a feature or `ui::theme`;
- no forwarding module is left but `Store::core()`.

Done on branch `foundation/2-kernel`, 2026-11-10, not merged; `make check` and `make smoke` green:
- `Store::core()` went instead of forwarding: every caller reads `kernel::core(cx)`. The fixtures
  call `kernel::init(core, cx)` before `Store::init(cx)`.
- The kernel cannot name `Store`, so an ended op emits `OpEnded { repo }` and `Store` refreshes
  that repo's status on it, as `run_op` did itself.
- `Store`'s notice test moved to the kernel with `notify`; the op test stays in `store.rs` and
  reads the op from `Activity`.
- PLAN-NATIVE's Store paragraph and contract lose ops, notices, the message and `run_op` now, not
  in step 6: the code changed here.

The owner checks: Fetch All with its progress and a cancel; an error raises a notice and lands
in Events; the unread count rises and clears; the status message goes after 10 s; a layout and
sizes saved before the step come back.

### Step 3. Registries

Size: 96 `Kind::` in `src` and 9 in tests; six feature imports in `dock/`; `shell.rs` names eight features.

Work, a commit each:
1. The app registration of §4; `Kind` goes; `dock::open`, `toggle`, `close` and `is_shown` take a name.
2. The document object of §4, implemented by `files`, `forge`, `chat` and `terminal`; `DocTab`
   calls it.
3. The drag payload and `dock::drop_doc`; the terminal's `adopt`; `close_terminal` on `close_docs`.
4. `shell::top_bar` and `shell::overlay` registrations; `history::init` into its `register`.
5. `Store::register_actions` split: `panel.*` and `zone.*` to `dock`, the rest to their features.
6. `main` lists the features, each with one `register(cx)`.

Tests: every `Doc` kind has an object; a `dock.layout` saved by 0.5.0, kept as a fixture, loads
with every app in its place and no placeholder; a saved panel that no build has still shows the
placeholder; a stub app and a stub document kind registered by a test alone appear in their zone
and tab, with no line of `dock/` or `shell.rs` naming them.

Done when:
- `grep -rnE "crate::(chat|files|forge|history|repos|search|terminal|events)" crates/app/src/dock crates/app/src/shell.rs`
  prints nothing, or only the one line step 0 allowed;
- `dock::Kind` and its five `match` tables are gone;
- every element id the tests use (`tool.Changes`, `bottom.Log`, `doc.mr.12`, …) is unchanged;
- step 0's two dock tests and the center one are green.

Done on branch `foundation/3-registries`, 2026-11-10, on top of `foundation/2-kernel`, not
merged; `make check` (128 s) and `make smoke` green:
- `main` and the UI fixtures call `detached_app::register(cx)` in `lib.rs`, which lists the
  features, each with one `register(cx)`; the fixtures need the same list, so it is not in `main`.
  A zone lists its apps in that order.
- An app is `dock::AppSpec`, added by `dock::register_app`, which also registers its `panel.*`
  action; the terminal and the MR chip no longer register their own. `nav.*` went to
  `nav::register`: nav is no feature, and in step 6 its registration stays in `app`.
- `Doc::kind()` and `Doc::KINDS` in the kernel name each variant for `dock::register_doc`. What to
  raise and what happens when a tab becomes active are one hook, `activated`. A kind with no
  object: its saved tab shows the placeholder, and showing or dropping such a doc opens nothing.
- The drag payload is `dock::DragDoc { doc, title }`: the preview keeps the shell's title.
- The Settings layer moved from the title bar's left slot to the overlays; it is drawn deferred,
  so nothing on screen moves.
- The 0.5.0 layout (`crates/app/tests/fixtures/dock-0.5.0.json`) was saved by a throwaway test
  on this branch's base, whose dock code is 0.5.0's but for import paths, in a temp sandbox: the
  Terminal app moved to the right zone, Project shown there, a kept diff and a chat in the center.
  It loads with no placeholder and saves back unchanged. "A saved panel that no build has shows the
  placeholder" was already a test (`tests/dock.rs`).

The owner checks: each app by its button and its chord; a tab dragged between zones; a shell
dragged to the center and to a zone, and a drag let go over nothing; an MR tab's title and a
chat tab's badge; closing a shell's tab ends the shell; Events clears its count when shown; the
layout after a restart; a layout from before the step.

### Step 4. Settings

Size: `repos/settings.rs` is 1,175 lines; 4 `open_settings` call sites in `src` and 6 in tests;
13 `Section::` and 5.

Work, a commit each:
1. Core: `accounts::remove` through `update`; `Settings::save` private to its module, so the
   compiler holds "one writer".
2. Core: the map of unknown keys, the legacy keys dropped after their migration, `block` and
   `set_block`, the revision.
3. Kernel: the `Settings` snapshot and its refresh after a `blocking::run` when the revision moved.
4. `settings/`: the layer, the navigation by group, `settings::register` and `settings::open`.
5. The pages move to their owners; `Section` and `repos::open_settings` go.
6. `Store.workspaces` and `Store.file_nesting` go; their readers take the snapshot.

Tests: §5's five (a save outside `update` does not compile; the snapshot after an account is
removed and after each caller of `update`; today's file round-trips with every key; an unknown
key survives each writer; the legacy keys do not bring a workspace back); a failed save leaves
memory and the snapshot as they were; each page opens by its id; the pages list by group.

Done when:
- `grep -rn "\.save(" crates/core/src` finds the one call in `update`, tests aside;
- `enum Section` is gone and `repos/` holds no settings layer;
- a `settings.json` written by 0.5.0 loads, and after any change in Settings differs from it only
  in the changed key;
- `Store` has neither copy.

Done on branch `foundation/4-settings`, 2026-11-11, on top of `foundation/3-registries`, not
merged; `make check` and `make smoke` green:
- `AppState.settings` is private to core as well as `Settings::save`, so nothing outside core
  writes memory either; a `compile_fail` doctest on `Settings` pins the save.
- A page is added with `settings::register_page(cx, id, title, group, view)`, not
  `settings::register`: since step 3 that name is each feature's own `register(cx)`. The settings
  feature's `register` adds the layer and the pages of `ui` (Appearance) and `keymap` (Keymap),
  which have none. There is no Updates page: today's Settings has no updater section; it comes in step 5.
- Ids and groups: `accounts`, `git`, `appearance`, `keymap` in the app's group, `repository` in the
  current repo's; Modules is empty. The nav lists them in that order under a heading per group, so
  Git moved above Repository.
- Every page is made when Settings opens and kept while it is open, as the one dialog was. The
  three pages of `repos` share one state, so an account added shows in the repo's pickers at once.
- `set_block` refuses a key the build knows and the legacy ones.
- The 0.5.0 `settings.json` (`crates/core/tests/fixtures/settings-0.5.0.json`, every key filled)
  was saved by a throwaway test on this branch's base. It loads and saves byte for byte, and each
  writer changes only its own keys, with and without a block it does not know.
- The kernel's test boot waits for the snapshot's first load: a reload still running when a test
  ended aborted the kernel's test binary once.
- `Store::load_repos` no longer waits for a settings load before `refresh_all`, which made
  `refresh_all`'s lost call (docs/TECHDEBT.md) fail `the_empty_state_adds_folders_and_creates_default`
  under load; it is fixed here, `again` cleared before the listing starts, not in step 7. Step 7's
  test for it is still to write.
- The repos pages keep reading `settings.json` values from their own load (their `Data`), which
  reloads with each call that changes one; only the store's two copies were in this step.

The owner checks: every page; an account added and removed, with the repo's account pickers
right at once; `fileNesting` edited by hand and kept; an unknown key added by hand, kept after a
change in Settings; a removed repo restored; Cmd+, and each "Open Settings" link in the MR and
pipeline views land on the right page.

### Step 5. Updates

Size: `app/update.rs` is 538 lines, most of them tests; `tests/update.rs` 51.

Work, a commit each:
1. `core::update`: the move, the download to a temp file with a progress callback.
2. Kernel: `Update`, its states, the launch and daily checks that go on after an offer.
3. The Updates page; the status bar's marker in place of `update::item`.
4. "Restart now": the warning, then the swap, the relaunch helper and the quit as one call.

Tests: the percent only rises, against a server that sends a length; a server that sends none
shows bytes without a percent; a cancel in mid-download returns to "offered", leaves no temp
file and lets a new download start; a click while one runs starts nothing; a bad signature ends
in "failed" with its text and no file; a check after an offer finds a still newer release; a
failed background check leaves the status bar silent and shows on the page; a manual check
reports "up to date" and an error; a `.deb` never downloads; a build that is not an install says
updates are off; the page in a UI test, state by state.

Done when:
- `cx.quit()` is reachable only from "Restart now";
- `Activity` never holds the download or the install;
- docs/TECHDEBT.md's update row is removed, and `tests/update.rs` no longer asserts it;
- the owner updated a packaged macOS build to the next release through the page, with a chat and
  a shell open: warned of both, restarted into the new version, no temp folder left beside the app.

Not covered, and it stays in docs/TECHDEBT.md: the install on Windows and from an AppImage has
never been run by hand.

Done on branch `foundation/5-updates`, 2026-11-11, not merged; `make check` and `make smoke` green:
- `core::update::download` writes to a temp file in a directory it is given and verifies the file
  with minisign's stream verifier; it returns a `TempPath`, so an error, a cancel or a drop leaves
  no file. `Install::apply` takes the file's path. The local test server and keys are
  `update::testutil`, behind core's `testutil` feature, for core's, the kernel's and the UI tests.
- `kernel::update::Update` holds `state` (§6's nine) and `release` apart: a failed check keeps the
  offered release. A background check and a manual one are the same call; a failure is "silent in
  the status bar" because the marker shows only a release, and the page shows the error.
- What tests replace is `Update.feed`: the base URL, the key, the running version and the download
  directory (the system temp directory in the app). `kernel::init` makes `Update` with
  `Install::current()`; `main` calls `kernel::update::start` for the launch and daily checks, not
  under `--smoke`.
- No check runs from the download to the restart, so a ready package stays until "Restart now".
- A cancel sets the flag and the state stays "downloading" until the download's thread sees it at
  its next read (a stalled read waits for its 30 s timeout), then "offered": nothing new starts
  beside it, which was the bug.
- The page is `settings/updates.rs` in the App group after Keymap, registered by the settings
  feature as Appearance and Keymap are: its owner, the kernel, draws nothing, and `app/update.rs`
  is gone. The marker is the status bar's own.
- "Live" chats are the open ones with something in them, named by their tab's title
  (`chat::live`); shells are every shell, in the Terminal app or a tab of its own
  (`terminal::live`). With none, "Restart now" restarts at once.
- docs/TECHDEBT.md had no update cancel row; `tests/update.rs` alone asserted the bug, and is
  rewritten. The row of the updater's Unix-only tests is removed: they moved to core and are listed
  in core's Unix-only row; the Windows and AppImage row stays.

The owner checks: in a `make smoke`-style build nothing changes but the Updates page under App,
which says updates are off; in a packaged build, the marker on an offer opens the page, Download
shows the percent and Cancel brings the offer back with no `.detached-package*` file left in the
temp directory; "Restart now" with a chat and a shell open names both, "Not now" keeps them, and
"Restart anyway" restarts into the new version with no `.detached-update*` folder beside the app.

### Step 6. `Workspace` and `Repo`

Size: 174 `Store::global` call sites; 14 `nav::`.

Work:
1. `Repo` and `Workspace` in the kernel, filled from the same core calls as `Store`.
2. One commit per feature, each moving its readers: `repos`, `files`, `forge`, `history`,
   `search`, `chat`, `terminal`, then the shell, the dock and the status bar.
3. `nav` to the kernel.
4. `store.rs` and `Store::core()` go; the fixtures' `open_app` boots the kernel.

Tests, in the kernel: a status change of one repo notifies that repo's observers and no other
repo's; a repo appearing on disk gains its entity and one removed loses it; switching the
current repo emits once; `nav`'s own, moved.

Done when:
- `store.rs` is gone and nothing under `crates/app` names `Store`;
- no table keyed by a repo's path is left outside `Repo`;
- a fetch's progress line re-renders the status bar and not the sidebar, shown by the test above.

Done on branch `foundation/6-workspace-repo`, 2026-11-11, on top of `foundation/5-updates`, not
merged; `make check` and `make smoke` green:
- While the features moved, `Store` was a copy of the kernel: its writes went to the kernel and it
  copied `Workspace` and every `Repo` back on each change, so each feature's commit moved alone. The
  writers of the compare base, the tree ref and the outcome moved in the first commit, since the
  copy would have overwritten theirs; so did the shell's `Shown`, which through the copy came after
  the chat list's deferred focus.
- `Workspace` keeps the list and one `Repo` per listed repo in the list's order, no map by path. It
  notifies for its list, the current repo and the shown doc; a repo notifies its own observers, and
  `Workspace` emits `Changed(path)` for it and `Status(path)` after a status refresh, for views that
  follow a repo by path (a file, a diff, the Log). `Workspace::observe(cx, f)` calls `f` on both,
  for views without a window; views that need the window subscribe themselves.
- A repo that leaves the list loses its entity, and with it its compare base, tree ref and branch
  names, which `Store` kept; a status of a repo not listed is not kept, its `Status` still emitted.
- `Repo::set_outcome` fades a success itself; the repos feature's fade counter went. Left in their
  features as views' memory, not repo state: the branch popup's recent branches, the tree's
  expanded folders, the pipelines' seen jobs, and `cache.rs` until step 7.
- `set_current` notifies even for the repo already current, as `Store`'s did: File History sets the
  Log's filters and relies on it. The test: a switch notifies `Workspace`'s observers once and no
  repo's.
- GPUI redraws the whole window when any entity drawn in it notifies, so the progress test is on
  notifications: a progress line notifies `Activity`, which the status bar observes, and neither
  `Workspace` nor a `Repo`, which the sidebar observes; the op's end notifies its repo alone.
- `nav` is `kernel::nav`; `app/nav.rs` only registers `nav.back` and `nav.forward`, since actions
  need a window. `kernel::init` makes `Workspace` and starts its first rescan; the kernel's test
  boot waits for it.

The owner checks: the repo switcher and workspaces on and off; fetch and pull of one repo and of
all; statuses, ahead and behind, the branch widget; Compare with a base; the tree at a ref; back,
forward and recent files; a repo added and removed on disk.

### Step 7. Resources and invalidation

Size: 25 `Cache` uses; polls in three forge views; refresh asked from 7 places outside the store.

Work, a commit each:
1. `Resource<T>`, its policy, `Interest`, and invalidation by a job's end, activation and time.
2. Forge: the MR list, the MR, pipelines and the badge on resources; their polls as policies;
   `cache.rs` goes.
3. The changes list and the tree as resources of their repo.
4. `refresh_all`'s coalescing into invalidation, with its lost call fixed.
5. The seven refresh calls go; a view keeps only `refresh()` behind its button.

Tests, on GPUI's clock: a pipeline polls every 5 s only while it runs, an interest is held and
the window shows; the MR list every 60 s; activation skips a resource fresher than 30 s and a
button never skips; a dropped interest stops its poll; a failed refresh keeps the last value
and the error beside it; a repo added while the list loads is not missed, failing on main; the
current repo's MRs load in the kernel with no forge view ever built.

Done when:
- `cache.rs` and `forge::poll` are gone;
- no view starts a status or list refresh but through `refresh()`;
- the forge and history tests that drive `advance_clock` pass with their steps unchanged;
- docs/TECHDEBT.md's `refresh_all` row is removed.

Done on branch `foundation/7-resources`, 2026-11-11, on top of `foundation/6-workspace-repo`, not
merged; `make check` and `make smoke` green:
- `kernel::resource`: `Resource<T>` (value, error, `at`, a loading flag), `Policy { fresh_for, poll }`
  and `Interest`, which holds it while it lives (`Interest::view` also redraws its view). Resources
  live for the session under a key (`resource::get`), as `Cache` kept its values. Taking an interest
  loads the resource, as a view's mount did; activation reloads a held one older than `fresh_for`;
  a resource reloads on nothing while no interest holds it. A refresh drops a load still running,
  so a stale result never lands; a poll skips a tick while one runs. A poll's condition sees a value
  not loaded yet, so the MR list polls after a failed first load, as its view did.
- `kernel::invalidate` is the one place: `status(repo)` loads a repo's status once at a time and once
  more after a load that calls came in during (§7's rule, now for every caller), its task ending
  with the load that saw the call; an op's end calls it. `rescan` and `listed(list)` replace
  `refresh_all` and `load_repos`, whose `Option` had no `None` caller left. `activated` and
  `set_visible` come from the shell, since the kernel cannot name a window.
- What a status load marks: a resource declares `On::Status(repo)` (the changes in Compare and the
  working tree's listing) or `On::Head(repo)` (a repo's pipelines, which the Pipelines view reloaded
  by itself when the head moved).
- The forge's resources are in `kernel::forge` with `setup_problem` and `active`, so an interest
  without a view can hold them. "The MR" is the chip's MR of the branch: the MR page has no cache,
  poll or activation today and keeps loading itself, a view's own query.
- The pipelines' poll condition is on their list (a pipeline pending or running). The view also
  polled while an expanded job ran in a finished pipeline; a retried job sets its pipeline running
  again on both forges, and the view refreshes after its own actions.
- The changes in Compare and the tree are resources per repo and base, and per repo and ref, made
  by `files` (the tree's value is its built `Dir`, built on the worker as before). A view keeps the
  resource it showed before for the same repo on screen until the new one loads, as it kept its last
  result. It shows no value loaded before it took its interest (`Interest::loaded`): nothing kept
  that one fresh, so a repo returned to says "Comparing…" until it compares again, as before.
- The lost call of `refresh_all` was fixed in step 4, so a test of it passes on the base. What still
  failed there: a listing that read the settings before a folder was added landed after the add's
  own list and dropped the new repo until the next pass. `invalidate::listed` drops such a listing.
  The test (`crates/kernel/tests/rescan.rs`) holds the listing open by filling core's pool, all
  threads but one (`blocking::THREADS` is public for it), so it has a binary of its own; it failed
  on the base three times out of three.
- The refresh calls outside the kernel: the shell's activation is `invalidate::activated`; the
  sidebar's button `invalidate::rescan`; Changes' button `invalidate::status`; a settings change
  `invalidate::listed`. A checkout, a branch operation, a chat's restore and a turn's end are not
  jobs, so no job ends for them: they declare the repo's status stale with `invalidate::status`,
  and the invalidation runs the load. `Workspace::refresh` is the kernel's own.
- The kernel's tests take the forge fake and core's `e2e` as dev-dependencies, for the MRs loaded
  with no view.

The owner checks: the MR list and an MR page; a running pipeline in the badge and in Pipelines,
with a tab hidden behind another still fresh when it comes back; a job's live log; the window
left and returned to; the network off, with the old data and the error together.

### Step 8. The watcher

Work, a commit each:
1. `core::watch`: the git and common directories by §7, with a worktree's and a `.git` file's
   resolved; the worktree watch by OS; the filters.
2. Kernel: the watch list follows `Workspace` (a repo added, removed, the current one switched);
   its batches feed invalidation, one status at a time per repo.

Tests, on the three OSes, waiting on state: three commits made outside in a row each arrive; a
commit inside a linked worktree marks that worktree, and a ref moved in the main repo marks both;
`packed-refs` rewritten by `git gc`; a repo whose `.git` is a file elsewhere; `index.lock` alone
marks nothing; a write under an ignored directory marks nothing; a file saved in the current
repo shows in Changes with the window never left; a branch made outside shows in the branch
list; a repo over the cap is not watched and still refreshes on activation; step 0's Windows
renames, moves and deletes, kept as tests.

Done when:
- in five idle minutes on the owner's workspace the app starts no git process;
- a full build in the current repo starts none either;
- on Linux the watch count for the owner's workspace is reported and under the cap;
- where step 0 chose a fallback, that OS or that watch is off and its tests say so.

Done on branch `foundation/8-watcher`, 2026-11-11, not merged; `make check` and `make smoke` green:
- `core::watch::Watcher::new(on_batch)` and `set(repos, current)`; a batch is the events within
  100 ms of its first, named as the repos they mark and those whose refs moved. An event that says
  the OS dropped some marks every repo. The watches are diffed on each `set`, so an unchanged list
  touches nothing. `*.lock` is dropped under `refs` alone (`HEAD`, `index` and `packed-refs` are
  matched by name), so a tracked `Cargo.lock` marks its repo.
- The ignore rules of the filter: each directory's `.gitignore`, read when an event first passes the
  directory and forgotten when a `.gitignore` changes, over the global excludes and `info/exclude`,
  which each `set` reads again; the deepest rule for a path decides. A directory's own modified event is dropped: Windows raises
  it beside its child's, so an ignored file made in the root would mark the repo. Linux's walk is
  core's `files::walker`, the tree's. A new directory or a changed `.gitignore` runs the walk again
  on a thread of its own, which watches what it finds and drops what went; at most one more waits.
  inotify drops the watch of a directory deleted or moved away, so the watcher forgets it and its
  children, and one made again at that path is new to the walk.
- `kernel::invalidate::watch` starts it and `main` calls it, `--smoke` too; `kernel::init` does not,
  so the tests that count a repo's notifications see no refresh they did not cause, and a test that
  wants the watcher starts it. It sets the watcher on the blocking pool, one `set` at a time, again
  when `Workspace` changed meanwhile. Activation sets it again, which brings back a git directory's
  watch the OS dropped (a repo cloned again, a worktree removed and added) and reads the excludes
  again. A batch calls `invalidate::status` per repo, and a repo whose
  refs moved reloads its branch names if a view loaded them (they load on demand).
- "Over the cap" exists on Linux alone, so its test runs there, with the cap at 50 under test: the
  worktree gets no watch and a commit still marks the repo. Activation's rescan, which refreshes it,
  is unchanged. Step 0's Windows rows run as one test on the three OSes. Nothing is `#[cfg]` off.
- Measured on a temp config directory, 20 new repos and a logging git shim, the `--profile app`
  binary, the window left as it opened: 120 git calls in the first minute, none in the next five
  idle minutes; 2,000 files written under an ignored `target/` of the current repo, none; 20 writes
  to tracked and new files there, one status (3 calls). The first minute's 120 are two statuses per
  repo, the start's rescan and activation's, as many with this branch's first commit, which starts
  no watcher; step 0 measured 60 while the lost call of `refresh_all` dropped the second. That
  figure goes to the owner.
- docs/TECHDEBT.md: a watch the OS refuses (inotify's limit) is skipped without a word, and on
  Windows a lost folder watch or an overflow is not seen.

The owner checks: an agent's edits appear in Changes during its turn; a commit from the terminal
updates the sidebar and the branch widget; a switch made outside; a folder renamed and a repo
deleted while the app runs. The Log does not follow by itself: it is a view's own query.

### The foundation is done when

- steps 0 to 8 passed their gates;
- `store.rs`, `cache.rs`, `app/update.rs`, `dock::Kind` and `Section` no longer exist;
- the kernel has no kit crate under it, and `dock/` and `shell.rs` import no feature;
- `settings.json` has one writer, and no state lives in two places but the settings snapshot;
- docs/TECHDEBT.md has lost its update and `refresh_all` rows;
- CLAUDE.md, PLAN-NATIVE and PLAN-EXTENSIBLE say what the code does (§10);
- the numbers of step 0 are no worse by more than a tenth, or the owner accepted the ones that are;
- the owner worked a week on a build with all of it and did not go back to 0.5.0.

Then the second document is written against the code as it stands.
