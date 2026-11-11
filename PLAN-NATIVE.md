# Detached — native client plan

The same client without a browser: a Rust UI on GPUI (via gpui-kit) instead of Tauri, Svelte,
Monaco and a web view. This file is how it gets built: decisions, stack, layout, design, the
parallel verticals with their contracts, and the steps with their Done-when.

Decided with the owner on 2026-11-04:
- Go native, no browser. The Tauri client's black screen on Windows is not fixed; the native client
  replaces it.
- Mermaid renders in Rust; a block that fails shows its source.
- Plan first, reviewed with revmux; the work runs as parallel verticals after a shared foundation.

**Status (2026-11-09): steps 0–2 (F, V1–V6) done: the native client ships alone from 0.4.0 on the real config directory, the Tauri client and docs/native are removed. The update install is checked by hand from 0.4.0 to the next release (docs/TECHDEBT.md). Step 3, the terminal (V7), shipped in 0.4.1; a shell can move to a tab of its own in the center since 0.5.0; libghostty-vt is its only emulator on every OS since 2026-11-09.**

The parity bar was the Tauri client as of v0.3.0 plus the strip's Log/Pipelines buttons. What it
did was surveyed in docs/native/: shell.md (window, top bar, sidebar, workspaces, branches, palette,
settings, keymap, design tokens), files.md (viewer, diff, tree, Changes, find, markdown), history.md
(log, graph, MRs, pipelines, job logs, the fake forge), core.md (the backend's Tauri boundary) and
gpui-kit.md (what the kit can and cannot do). Every section had an owner in
[Parity owners](#parity-owners), who ticked it off when its behavior was ported, ported differently
(said how, in the commit), or dropped with the owner's OK. The survey went with the Tauri client in
V6; the section references below (shell.md §5, ...) point into it in git history.

## Why

- No WebKit/WebView2 processes: memory falls from the web view's 100–200 MB to the app's own, and
  startup is near instant.
- Large logs, diffs and files render on the GPU straight from Rust data: no JSON over IPC, no DOM.
- Linux without webkit2gtk, the slowest of the three web views and most of the 89 MB AppImage.
- Keys reach the app first: no capture-phase guards against Monaco and text fields.
- Room for a real terminal (step 3).

## Scope

In: everything in the parity docs, on macOS, Windows and Linux; auto-update; a CI that builds,
tests and smoke-runs all three OSes; a terminal after parity.

Out: commit, staging, push (still milestone 3), anything in [docs/BACKLOG.md](docs/BACKLOG.md),
new features during the port beyond the terminal. Behavior changes only where the platform forces
them; those are listed under Design.

## Stack

| Part | Choice |
| --- | --- |
| UI | `gpui-kit` **=0.9.0** (Apache-2.0), which pins GPUI as `gpui-pre` =0.3.11 and re-exports it with `gpui-base` and `gpui-component`. Exact pins, upgraded only on purpose (CLAUDE.md) |
| File view | the kit's `EditorState`, read-only, Tree-sitter highlighting, built-in find |
| Diff view | the kit's `Diff` (virtualized, split or unified, annotations under lines, gutter markers, next/previous change), fed a full-context unified patch built in core |
| Markdown | the kit's `TextView` with our link, image and code-block hooks |
| Mermaid | `mermaid-rs-renderer` to SVG, drawn with `img()`; on error the source block |
| Lists, tree | GPUI `uniform_list`/`list`, the kit's `Tree`; the commit graph on `canvas` |
| Async | core stays blocking; a small pool of std threads runs it (see Threads) |
| Packaging | `cargo-packager` 0.11.8 (`.app`, NSIS, deb, AppImage), configured in `crates/app/Cargo.toml` and run by `scripts/package.sh`, which signs the `.app` itself; its updater and signer are not used. A `v*` tag has `release.yml` build them on the three OSes, sign each with minisign, write `latest.json` (`scripts/latest-json.sh`) and sign it too, and publish them with `install.sh`. Looked at in V6: `gpui-updater-release` (Sparkle, macOS-centric) and `cargo-bundle` (no NSIS or AppImage) |
| Auto-update | ours: `core::update` checks, downloads to a temp file with its progress, verifies and installs; the kernel's `Update` holds its state, the percent and the cancel; the Updates page in Settings shows it and installs on "Restart now", which names the live chats and shells first. `latest.json` in the latest GitHub release (`{version, platforms: {"<os>-<arch>": {url, signature}}}`), the file and each package checked with `minisign-verify` against the key compiled in, so its version and URLs are the release job's; checked at launch and daily when run from a package, an offer or not |
| Terminal | step 3: libghostty-vt 1.0 (MIT, C ABI, linked statically) for the emulation, `portable-pty` for the PTY (ConPTY on Windows) |
| Tests | `cargo test` for core and for ported logic; the kit's headless UI tests (`#[gpui_kit::test]`) against temp repos and a fake forge, on macOS, Windows and Linux runners |

Not used: upstream `gpui` 0.2.2 from crates.io (a year old and type-incompatible with gpui-pre),
Zed's `terminal`/`terminal_view` (they need Zed's workspace).

Platform floors: macOS 15+ (the kit's stated minimum), Windows 10+, Linux with Vulkan or GL and
xdg-desktop-portal. v0.3.0, the last Tauri release, stays downloadable for older macOS.

## Layout

```
Cargo.toml                 workspace (resolver 3, edition 2024), [workspace.package], shared pins
crates/core/               package detached-core: the backend from src-tauri/src minus the commands
  src/paths.rs             config_dir(), DETACHED_CONFIG_DIR, the workbench rename
  src/api.rs               one function per command, taking op/query ids where today's commands do
  src/selftest.rs          --self-test <report>, unchanged (unix)
  src/update.rs            latest.json, the download to a temp file, the signature, the install per OS
  src/watch.rs             the watcher: repos' git directories and the current worktree, in batches
crates/forge-fake/         package detached-forge-fake: e2e/forge.ts in Rust
crates/kernel/             package detached-kernel: the app's live state on gpui-pre alone, drawing
                           nothing (PLAN-FOUNDATION)
  src/blocking.rs          runs core on worker threads, back on the UI thread with the result; QueryId
  src/ui_state.rs          ui.json
  src/activity.rs          ops, notices, the status message, the log the Events app shows
  src/doc.rs               Doc, Location, DiffDoc, DocRef
  src/workspace.rs         Workspace: the listed repos, a Repo each, the current one, the shown doc
  src/repo.rs              Repo: a repo's status, branch names, compare base, tree ref, last outcome
  src/nav.rs               back/forward and recent files
  src/settings.rs          the read-only snapshot of core's settings
  src/update.rs            Update: the updater's state, its checks, the download and the restart
  src/resource.rs          Resource: a loaded value, its error and age, its policy; Interest
  src/invalidate.rs        what is stale: a job's end or a write, window activation, time, the
                           watcher; the rescan
  src/forge.rs             the forge's resources: MRs, the branch's MR, pipelines
crates/app/                package detached-app, binary detached
  icons/                   the app icons
  src/main.rs              args (--self-test, --smoke), window, menus, boot
  src/util.rs              helpers several verticals use: path names, change letters, times, fuzzy
  src/keymap.rs            defaults, keymap.json, contexts, Double Shift
  src/actions.rs           action ids, registration, run
  src/nav.rs               registers nav.back and nav.forward
  src/ui/                  theme tokens, icons, panel, icon button, segmented control, row, grip,
                           the parts of a Settings page, the Appearance page
  src/settings/            the Settings layer and its pages' registry, the Updates page; api.rs is
                           the contract
  src/shell.rs             title bar slots, each app's view, the status bar
  src/dock/                the zones on the kit's dock: app and document tabs, the saved layout
  src/status_bar.rs, notices.rs
  src/repos/               V1: sidebar, workspaces, repo switcher, branches, sync, and the Accounts,
                           Repository and Git pages of Settings;
                           api.rs is the contract
  src/files/               V2: viewer, diff, tree, changes, markdown, mermaid; api.rs is the contract
  src/search/              V3: palette, find in files
  src/history/             V4: log, graph, commit details; api.rs is the contract
  src/forge/               V5: MR list and page, threads, merge widget, MR chip, pipelines, job log
  tests/                   UI tests, one file per vertical, shared fixtures in tests/common
```

The binary `detached` is the shipped `Detached.app/Contents/MacOS/detached`, which `install.sh`
quits before replacing. The version comes from the root `Cargo.toml`; Cargo's `target/` is at the
root.

## Carried over unchanged

- Config directory `dev.wdevs.detached` with the same path per OS as Tauri computes it
  (docs/native/core.md §2), `settings.json`, `accounts.json`, `keymap.json` with the same action ids
  and chord syntax, tokens under service `workbench`.
- `settings.json` keeps its top-level keys; a module adds one of its own, and a save keeps the keys
  a build does not know. A build from before PLAN-FOUNDATION's step 4 drops them on its next save.
- macOS: bundle id `dev.wdevs.detached`, `Detached.app`, executable `detached`, signed with the
  "Detached Dev" identity, so Keychain entries stay readable without prompts. `install.sh` keeps its
  URL and behavior.
- The look: the tokens in `src/app.css` (shell.md §5) become the GPUI theme, light and dark following
  the system. The Monaco theme colors (files.md §2) become editor and diff colors.
- UI state that lived in localStorage moves to `ui.json` in the config directory, same key names
  and ranges (`sidebar.collapsed`, `size.*`, `tree.nesting`, `viewer.sideBySide`, `viewer.mdPreview`).
  Values are not migrated from the web view's storage: first launch starts from defaults.

## Design

**Core split.** `src-tauri/src` moves to `crates/core` as one crate (its module cycles stay inside
it). The three files that mix commands with `impl AppState` are split: the impls go to core, the
`#[tauri::command]` wrappers stay in `src-tauri`, keeping command and argument names so `src/` keeps
working. `open_folder` becomes `core::os::open_path` on the `open` crate; config dir and boot move
from `lib.rs` to `paths.rs` and `AppState::boot()`. During the move:
- the process-wide statics (`quick::CACHE`, forge `DONE`, `LIMITED`, `ETAGS`, accounts `FILE_LOCK`)
  move into `AppState`, so tests and fixtures stop sharing them;
- `Ops::cancel` before `start` is no longer lost: ids are registered when handed out;
- `Env` gains extra environment variables, so the test sandbox sets `HOME`, `GIT_CONFIG_GLOBAL` and
  `GIT_CONFIG_NOSYSTEM` without a shell wrapper; `testutil` becomes public behind a `testutil`
  feature for the UI tests;
- the `e2e` feature (plain http to localhost counts as https) stays off by default and is never a
  normal dependency of `crates/app`; only test builds turn it on.

**Config directory.** `paths::config_dir()` honors `DETACHED_CONFIG_DIR`, which `make smoke` and CI
set to a temp directory. While both clients existed, `make app` pointed it at a copy,
`dev.wdevs.detached.native`; since V6 the native client, `make app` included, uses the real one.

**Threads.** Core calls block, some for minutes (a fetch waits up to 120 s of silence, holding the
repo lock). They never run on GPUI's executors: `kernel::blocking::run(cx, f)` sends `f` to a pool of std
threads (16, enough for fetch all's parallelism) and resumes on the UI thread with the result.
Streams (log rows, find matches, progress lines) come back over a channel drained by a foreground
task that batches per frame. Each op or query has an id from the kernel (`Activity::run_op`'s op id,
`blocking::QueryId`); dropping the view or starting a new query cancels the old id.

**Workspace and Repo.** The kernel's `Workspace` holds the repos the active workspaces list, the
current repo and the document of the tab last on screen (`shown: Option<Doc>`); each listed repo
has a `Repo` entity with its status, its last sync's outcome, the tree ref (`tree_ref`) and Compare
base (`compare`) other verticals read, and its branch names for the ref inputs (`branch_names`,
loaded on demand). A repo's change notifies its own entity; `Workspace` emits `Changed(path)` for
it, `Status(path)` after a status refresh and `Shown` after `show`. Views observe what they read.
Workspaces are the kernel's `Settings`; ops, notices and the status message its `Activity`.
`kernel::invalidate` decides what is stale: window activation rescans with every status older
than 30 s and reloads a resource older than its policy allows; an op's end, a write (`invalidate::status`) and a Refresh
button load a repo's status, one load at a time per repo; a settings change's list goes through
`invalidate::listed`. `invalidate::watch`, which `main` starts, sets `core::watch` to the listed
repos and the current one: a change in a repo's git files loads its status, and its branch names
when loaded and its refs moved; a change in the current worktree outside what git ignores loads its
status. A `Resource` (the forge's lists, the changes in Compare, the tree) loads
while a view or a watch holds an `Interest` in it, and polls by its policy while the window shows.

**Keymap.** `keymap.rs` keeps the action ids of docs/KEYMAP.md, parses the same chord strings and
the `keymap.json` overrides into `KeyBinding::load` (an override that does not parse unbinds the
action, as now; parsing never panics). Bindings carry contexts so text fields and the editor keep
their own keys. Double Shift comes from `observe_keystrokes`: two lone Shift releases within 300 ms.
Off macOS, Cmd is Ctrl and the VCS popup is Alt+`, as today. Native menus (macOS) list the actions
with their current chords and are rebuilt when the keymap changes.

**Window.** macOS: transparent title bar, traffic lights at (16, 23), the title bar as the drag
region. Windows and Linux: the kit's title bar with window controls at its right. Minimum 900×600,
default 1400×900. Which zone holds which app and the open documents are the dock's (`dock/`), saved
in `dock.layout` (PLAN-EXTENSIBLE step 3).

**Self-test and smoke.** `--self-test <report>` keeps today's meaning: the backend check from
`core::selftest` (login-shell PATH, node and git-lfs, a checkout with a node hook), run in `main`
before GPUI starts any thread since it sets environment variables; Unix only. `--smoke` is new:
boot, open the window, render one frame, exit 0; CI runs it on all three OSes.

**File view.** Read-only `EditorState`. Language from the file name, then the extension, mapped to
the kit's names by our table (the kit matches names, not extensions); Dockerfile gets a registered
`tree-sitter-containerfile` grammar. Files in languages the kit lacks show as plain text (Monaco
colored more languages: a known loss). Placeholders, the conflict bar, Open in Zed, go to line (our
overlay), reload in place on status change, and reveal-and-focus on open, as in files.md §1.1.

**Diff view.** Core gets `file_patch(repo, path, orig_path, left, right)`: it reads both sides the
way `file_diff` does (filters, LFS, symlinks, conflicts), diffs the texts in Rust (`imara-diff`,
histogram) and returns a unified patch with the whole file as context, so the kit's per-hunk
highlighting sees the whole file. The view turns the kit's context folding off (`context_lines`
none), so the whole file shows, as Monaco does; equal sides (an unmodified file, a pure rename, a
mode-only change) show through `DiffFile::unchanged`. Side-by-side and unified (persisted),
F7/Shift+F7, the cursor on the first change of a fresh diff, Cmd+D's three cases, placeholders for
non-text sides. MR line comments are `Diff` annotations; the "+" comes from
`on_add_annotation`/gutter markers, limited by `commentable` as today.

**Markdown.** `TextView` with: code blocks highlighted by the same grammars; `mermaid` blocks
through a block hook that renders SVG off the UI thread and shows the source plus the error on
failure; repo images through an image source that calls `read_image` (same rules: relative paths
only, 5 MiB, the listed formats); `on_link_click` for external links (system browser), `#anchor`
(GitHub slug, `reveal_range` to the heading), relative links (open in the app). Source/Preview as
today. `TextView` has no `breaks` option, so MR text (which today renders single newlines as line
breaks) is preprocessed: a lone newline outside code becomes a hard break before parsing.

**Log graph.** Lanes from `core::git::graph` as now; rows are fixed height in a `uniform_list`, each
painting its lanes with `PathBuilder` on a canvas. Batches of 500 rows stream in as today.

**Tests.**
- Core keeps its `cargo test` suite. About 55 of its tests are Unix by construction (`#!/bin/sh`
  helpers, `PermissionsExt`, `ps -o pgid`): step 0 marks them `#[cfg(unix)]` and lists them in
  docs/TECHDEBT.md, so Windows CI runs the rest green and the gap is visible.
- The 12 bun unit test files (`src/lib/*.test.ts`) become Rust unit tests next to the ported logic,
  each case kept: actions, cache, fuzzy, keymap, nav → F, plus the helper cases of files (`letter`,
  `statusOf`) and log (`when`); sync → V1; files, markdown and mr's `otherLine`, sanitizer and
  links → V2; log → V4; ansi, the rest of mr, pipelines → V5.
- Playwright scenarios become UI tests under `crates/app/tests`: real views in a headless window,
  real git in a temp sandbox, `crates/forge-fake` on a local port, driven with `click`, `press`,
  `input` and checked through element snapshots. Scenarios map to the vertical owning the
  behavior, not to the spec file:

  | Scenario | Owner |
  | --- | --- |
  | frame: Cmd+9 and the strip open and switch the bottom panel | F (with stub tabs) |
  | frame: forge repos listed with the seeded upstreams | V1 |
  | sidebar, sync, branches; workspaces: several on, rename, turn off, delete | V1 |
  | workspaces: nesting and its switch; changes; markdown | V2 |
  | find; palette (its Cmd+, opens Settings) | V3 |
  | mr; pipelines (their Open Settings opens Settings) | V5 |

  The log has no spec today; V4 writes tests for history.md §1 and §6.5.
- Waiting: core runs on real threads, which GPUI's deterministic test executor does not see. The
  fixtures provide `settle(cx)` and `wait_until(cx, predicate)`: they allow parking, run the
  executor until parked and poll the predicate on real time up to 10 s. Tests wait on state through
  these, never on sleeps.
- Pixel checks are not used; the smoke run has a screenshot artifact on macOS and Linux.

**Measurements (step 0 go/no-go).** On the owner's Mac with a workspace of about 20 repos: release
binary size, cold and incremental build times, idle memory, startup to first frame, scrolling a
5,000-line diff, opening a 2,000-line markdown file. Targets: idle memory under 120 MB, startup
under 500 ms, no visible stutter in the diff; anything worse is reported with the numbers and the
owner decides.

## Verticals

Step 0 (foundation, one owner) lands first; then V1–V6 run in parallel, each in its own worktree
and branch (`native/v1-repos`, …), each owning its directory under `crates/app/src` and its test
file. V7 (terminal) starts after parity.

| Vertical | Owns | Needs from others |
| --- | --- | --- |
| F foundation | workspace, `crates/core`, `crates/kernel` (in place of `store`, `blocking`, `nav` and `ui_state`), `crates/forge-fake` control and project routes, `main`, `util`, `keymap`, `actions`, `ui/` (including the context-menu helper), `shell`, `status_bar`, `notices`, the first version of `src/files/api.rs`, `src/repos/api.rs` and `src/history/api.rs` (contracts and stubs), test fixtures, Makefile, CI | — |
| V1 repos | `src/repos/` | `Workspace`, `Repo`, shell slots, forge-fake `/user` and `/version` for account verify |
| V2 files | `src/files/` | `Repo` (`tree_ref`, `compare`), nav, `core::api::file_patch` |
| V3 search | `src/search/` | actions, nav, `Repo` (`tree_ref`), `files::open`, `repos::checkout`, `settings::open` |
| V4 history | `src/history/` | shell's bottom slot, `files::show_diff`, `repos::checkout`, `Workspace::shown()` |
| V5 forge | `src/forge/`, forge-fake MR, pipeline and job routes | top-bar slot, `files::DiffView` with annotations, `files::Markdown`, `settings::open`, `kernel::forge`'s resources, `Workspace::show`, `nav::open` with `DocRef`, `history::CommitDetails` |
| V6 ship | `core::update`, `kernel::update`, `settings/updates.rs`, packaging, `.github/workflows/`, `scripts/`, `install.sh`, the Tauri client's removal | everything, for the release |
| V7 terminal | `src/terminal/` | shell's bottom slot |

### Contracts

Fixed by F before the verticals start. Changing an existing signature or meaning takes a plan
edit agreed with the owner. [PLAN-FOUNDATION.md](PLAN-FOUNDATION.md) is such an edit, agreed on
2026-11-10: its steps change the contracts below, each in the step that makes the change true.
Adding is lighter: a vertical that finds a missing piece (a field, a
variant, a function) adds it with a stub to the provider's `api.rs` or the kernel in its own small
commit on main, with the matching line in this section, without changing what is there; the
provider then fills it in.

- **Workspace and Repo:** fields and events above; `Workspace::current()`, `Repo::get(cx, repo)`
  with its `status()`, `tree_ref` and `compare`, `Activity::run_op(...)`, `Workspace::shown()`,
  `Workspace::show(Doc)`, which opens the doc in a tab or brings its tab forward.
- **Dock:** `dock::open(name, window, cx)`, which shows the app of that name in its zone (never
  toggles; an app's `panel.*` action toggles it with `dock::toggle`), for Compare with Base (V1
  opens `changes`), file history (V4 opens `log`) and Run Pipeline and the badge (V5 open
  `pipelines`); `dock::is_shown(name)`; `dock::close_docs(pick)` closes the document tabs it picks.
- **Threads:** `kernel::blocking::run`, `kernel::blocking::stream`, the op and query id rules.
- **Core:** `api::file_patch` returns `FilePatch::Patch` (the whole file as one hunk),
  `Unchanged(text)` for equal sides, or `Sides(FileDiff)` when the sides are not two texts.
- **Actions:** `actions::register(cx, id, title, group, handler) -> Registration`. The latest
  registration of an id wins; dropping it restores the previous one (how the MR page's diff takes
  F7, Shift+F7 and Cmd+L from the main viewer while mounted); `actions::raise` puts a tab's back on
  top when it comes on screen. `actions::run(window, cx, id) -> bool` says
  whether a handler ran; `actions::dispatch` runs one or says through `Activity` that it is not
  available. A vertical adds its ids to the list in `actions.rs` and default chords to
  `keymap.rs` (append-only). Each action's handler is registered by the vertical that owns its
  behavior; step 0 adds an owner column to docs/KEYMAP.md's table, covering every action of today's
  `registerActions` too: `panel.*` and `nav.*` → F; `vcs.update`, `vcs.fetch`, `vcs.fetchAll`,
  `vcs.pullAll`, `vcs.checkout`, `vcs.newBranch`, `vcs.deleteBranch`, `vcs.prune`, `app.settings`
  → V1; the palette modes (`search.everywhere`, `file.goto`, `action.find`, `file.recent`,
  `vcs.popup`) and `find.inFiles` → V3; viewer and diff actions → V2; `vcs.fileHistory` → V4;
  `mr.*`, `pipeline.*`, `panel.mergeRequests` → V5.
- **Shell slots:** each feature's `register(cx)`, listed in `lib.rs`'s `register`, which `main`
  and the fixtures call before the window opens. It adds its apps with
  `dock::register_app(cx, AppSpec { name, id, title, action, home, view, shown })`: `name` is what a
  saved layout knows it by, `id` the tab's element id, `action` the `panel.*` action the dock
  registers to toggle it, `home` its zone and size when a layout does not place it, and a zone
  lists its apps in the list's order. It gives each `Doc::kind()` it shows (`file`, `diff`, `mr`,
  `job`, `chat`, `terminal`; `Doc` stays a closed enum) an object of `dock::DocKind` with
  `dock::register_doc`: the tab's view, id, title and badge, what it does when it comes forward or
  closes, which docs share a tab, the preview tab, and how it takes a dropped doc (`adopt`); a doc
  dragged out of an app's own list is a `dock::DragDoc` carrying the `Doc`. It adds title-bar views
  with `shell::top_bar(cx, slot, order, view)`: `TopBar::Left` (V1: workspaces, repo switcher,
  branch widget), `TopBar::Center` (V5: MR chip, pipeline badge), `TopBar::Right` (V3: search pill;
  V1: Fetch All, Pull All); and `shell::overlay(cx, view)`, modals drawn over the window and under
  the notices, inside the shell's key context so chords still reach the app while one is open
  (Settings, which draws its own layer over the top bar's popups; V3: palette, find). `dock/` and
  `shell.rs` name no feature.
- **Cross-vertical calls** live in an `api.rs` in the providing vertical's directory. F writes the
  first version with the types and stub bodies; the provider implements the bodies without changing
  signatures; callers build against the stubs from day one.
- **Repos API** (`src/repos/api.rs`): `repos::checkout(cx, repo, CheckoutTarget)` (core's type;
  smart checkout with its dialogs).
- **Settings API** (`src/settings/api.rs`): `settings::open(cx, id)` opens Settings at a page, or
  shows it when open (`accounts`, `repository`, `git`, `appearance`, `keymap`, `updates`);
  `settings::register_page(cx, id, title, group, view)` adds one, listed by
  `Group::{App, Modules, Repo}` in the order registered.
- **Files API** (`src/files/api.rs`):
  - `Location { repo, rev: Option<String>, path, line: Option<u32>, doc: Option<DocRef> }` with
    `DocRef::{Mr { repo, number }, Job { repo, project, id, name }}`, so MR pages and job logs are
    in back/forward as today;
    `DiffDoc { repo, path, orig_path, left: Side, right: Side }` (`Side` from core); the three are
    re-exported from `kernel::doc`, unchanged;
  - `files::open(cx, Location)`; `files::show_diff(cx, DiffDoc, focus: bool)`, where `focus` moves
    focus into the diff once it loads (double-click or Enter) and `false` leaves it in the caller's
    list (a click or arrow keys stepping through files);
  - `DiffView::new(doc, DiffOptions { side_by_side: Option<bool>, annotations, on_add, commentable }, cx)`;
    `DiffView::set_annotations(annotations, cx)` replaces the views under lines (MR threads, the
    draft) without reloading the diff;
  - `Markdown::new(text, Links, breaks: bool, cx)`, `Links::Repo { repo, rev, path }` or
    `Links::Forge { project_url, base_url, gitlab: bool }` (GitLab resolves `/uploads/…` under the
    project and shows its upload images as links, history.md §2.7).
- **History API** (`src/history/api.rs`): `CommitDetails::new(repo, shas, cx)`, the view the log's
  side panel and the MR page's Commits tab both show; `CommitDetails::worktree(repo, sha, cx)`, one
  commit against the working tree, for the log's Compare with Local.
- **Helpers** (`util.rs`, F): `base_name`, `dir_name`, `folder_name`, `in_folder`, the change
  `letter` and `status_of`, `when` (commit and pipeline times), `fuzzy::filter`/`segments`; the
  context-menu helper in `ui/`. A vertical's own helpers stay in its directory.
- **Navigation:** `kernel::nav::open(cx, Location)` records and shows any location (file, MR, job),
  back/forward, recent files (files only) with today's caps (50, 100).
- **UI:** `ui` primitives and theme tokens; a vertical needing a new primitive adds it to `ui/` in
  its own commit. `kernel::ui_state::get/set` with the vertical's key prefix.
- **Fixtures:** `Sandbox` repos, `settle`/`wait_until`, the forge fake, `open_app(cx)` returning a
  test window with the kernel booted; `open_app_with(cx, layout)` for e2e's sandbox (`ws/`, `solo`)
  and `open_forge(cx)` for its `forge/` repos on a seeded fake, with the `Fixture` paths
  (`sb.repo("group/api")`, ...) and `sb.commit_files`.
- **Forge fake:** F ports the control routes (`/health`, `/reset` with the seed of projects, bare
  repos and MR head SHAs, `/__requests`, `/__writes`, `/__delay`), auth, pagination, `/user`,
  `/version` and project lookup (GitLab by path or id, GitHub repo), and the per-project branch
  SHAs reset takes from the MRs (`State::branches`). V5 adds the MR, pipeline, job and GraphQL
  routes of history.md §6.2. Tests drive it through `Forge::reset`, `requests`, `writes`,
  `append(kind, path, job, text, status)` (as `/__append`) and `delay(ms)` (as `/__delay`).

### Parity owners

| Section | Owner |
| --- | --- |
| shell.md §0, §1.2, §1.11–1.14, §2.1 (store state, ops, notices, refresh), §2.2, §2.3, §2.5–2.8, §3 (keys of F), §5 | F |
| shell.md §1.1 (top bar shell) | F; its widgets per the slot list |
| shell.md §1.3–1.7, §1.9, §1.10, §2.1 (the rest: checkout and its notices, recent branches, sync, prune, add folders, `settingsCall`, and the handlers per the actions' owner column), §2.4, §3 (sidebar keys), §4 | V1 |
| shell.md §1.8; files.md §1.4, §1.6, §4 | V3 |
| files.md §0, §1.7 | F (kernel, nav, actions, viewer lifetime), V2 (viewer) |
| files.md §1.1–1.3, §1.5, §2, §3, §5, §6 | V2 |
| history.md §0 | F (panel toggles, strip), V5 (MR chip, badge) |
| history.md §1 | V4 |
| history.md §2 (§2.7's rendering and link rules: V2), §3, §5, §6.2 | V5 |
| history.md §4 | each key's owner (V4 log sizes, V5 MR sizes and cache) |
| core.md (all) | F |
| test-coverage sections (shell §6, files §7, history §6) | each scenario's owner per Tests |
| gpui-kit.md | reference, no ticks |

A section split between owners lists the first owner's part; the rest belongs to the second.

### Merging

One vertical branch merges at a time: rebased on main, `make check` green on all three OSes, its
parity sections ticked, its revmux round clean of blocking findings, the owner's manual check on
macOS. Verticals merge in slices (a vertical can land half its views), not as one big diff.

## Steps

Each step ends green (`make check`), checked by hand on macOS, and committed.

0. **Foundation.** Done when:
   - the workspace builds; `crates/core` holds the backend and the Tauri client runs on it; core
     tests pass on macOS and Linux, and on Windows except the listed `#[cfg(unix)]` ones;
     `make e2e` still passes;
   - the Makefile works from the root: `make check` runs `cargo fmt --all --check`,
     `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (core, ported
     unit tests, UI tests) and, until V6, `bun run check` and `bun run test`; new targets
     `make app` (the native client, release-like opt levels, `DETACHED_CONFIG_DIR` set), `make
     ui-test` and `make smoke`;
   - `release.yml`, `scripts/e2e-app.sh`, README and the root `.gitignore` follow the root
     `target/`; a manual run of the release workflow still builds the Tauri client;
   - `crates/app` opens the window on all three OSes with the theme, strip, top bar slots, stub
     tool, main and bottom views, resizable panels with sizes in `ui.json`, status bar and notices;
     the keymap loads `keymap.json`; docs/KEYMAP.md has the owner column; `--smoke` and `--self-test` work as described;
   - `src/files/api.rs`, `src/repos/api.rs` and `src/history/api.rs` with stubs, `util.rs`, the
     forge fake's F routes, the fixtures and `core::api::file_patch` exist and are tested;
   - a spike shows a real file in the editor and a real diff of a 5,000-line file in the diff view
     with one annotation under a line;
   - CI (`.github/workflows/check.yml`) runs `make check` and `make smoke` on macOS, Windows and
     Linux on every push;
   - the measurements above are reported to the owner, who decides go or no-go.
1. **V1–V5 in parallel.** Each vertical is done when its parity sections are ticked, its scenarios
   and ported unit tests pass, and the owner has used it on real repos. First slices: V2's diff
   view and V1's sidebar, since the others build on them.
2. **V6 ship.** Done when:
   - a tag builds signed macOS, Windows and Linux packages in CI and publishes them with
     `latest.json` and signatures;
   - the app offers an update and installs it on each OS (checked from the previous native release);
   - the native binary is renamed `detached`, so the bundle's `Contents/MacOS/detached` and
     `install.sh` keep working; `scripts/e2e-app.sh` builds and checks the native bundle;
   - the native client uses the real config directory; the README and docs describe it;
   - `src-tauri`, `src/`, `e2e/`, `build.ts`, Bun, Playwright and the bun steps of `make check` are
     gone, `docs/native/` too.
3. **V7 terminal.** Tabs in the bottom panel, cwd in the current repo, TUIs work (vim, htop,
   lazygit), keys go to the terminal except a short list (Double Shift, Cmd+1, Cmd+9, Cmd+,);
   memory per terminal by session id (macOS `phys_footprint`, Linux `Pss`, Windows a job object),
   shown on its tab; scrollback 1,000 lines by default.

   How (2026-11-09):
   - libghostty-vt 1.0 does the emulation, through a hand-written FFI in `terminal/ghostty.rs`
     (all of the `unsafe`); `portable-pty` (WezTerm's) runs the shell on a PTY, ConPTY on Windows.
     Per shell, one thread reads the PTY into the emulator, one writes the keys and the
     emulator's replies, one waits for the exit. The UI reads the screen under the emulator's
     lock once a frame. Sessions live in a global, so they outlive the bottom view when the panel
     switches to Log or closes.
   - The library: `crates/app/build.rs` links it statically. macOS takes the release's
     XCFramework; Linux and Windows build it with Zig 0.17 from the release's source and
     dependency tarballs, offline (`zig build --system <deps> -Doptimize=ReleaseFast
     -Dcpu=baseline -Dstrip=true`), MSVC ABI on Windows. Every download is pinned by its SHA-256
     from the release's minisign-signed SHA256SUMS. `GHOSTTY_VT_DIR` keeps the built library out
     of the target directory; CI caches it there and installs Zig with `mlugg/setup-zig`.
   - Each row is styled texts in the mono font with a run per style, cut into pieces at fixed
     columns (wide and non-ASCII cells alone, Nerd Font icons from a bundled Symbols Nerd Font
     Mono); theme `ansi` colors, the cursor and the selection are runs too. Selection is
     libghostty-vt's gesture (word on double click, line on triple), copy its formatter. The
     view's size sets the grid's columns and rows.
   - Keys: an app-level keystroke interceptor writes every keystroke to the focused terminal
     before the app's bindings, except the chords of `panel.project`, `panel.log`, `app.settings`
     and `panel.terminal` (with `keymap.json` overrides); Double Shift still works. On macOS, Cmd
     chords the app does not bind go on to the system (Cmd+Q, Cmd+H); Cmd+C and Cmd+V copy and
     paste, Ctrl+Shift+C and Ctrl+Shift+V elsewhere.
   - `panel.terminal` (Alt+F12) toggles the panel; the shell's exit closes its tab; a new tab
     starts in the current repo, or home without one.
   - Memory: `core::os::Session` from the shell's pid sums its session's processes every 2 s
     (macOS `phys_footprint` of the pids with that sid, Linux `Pss` from `smaps_rollup`, Windows
     the private bytes of a job object's processes).

## Risks

- **Kit churn and bus factor.** Weekly releases, nearly all breaking; one person owns every crate,
  and `gpui-pre` is an unofficial weekly snapshot of Zed main. Exact pins, upgrades as their own
  commits; the kit is Apache-2.0, so a fork is the fallback if it stalls.
- **Kit bugs on our paths.** Long markdown is slow (#3695), Tree-sitter injections cap at 512 per
  document (#3702), markdown image and `<pre>` layout bugs (#3706–#3711), Windows splitter lag
  (#2488), dialog focus (#3760). Each vertical checks the open issues for its components and works
  around them in our code, not by patching the kit.
- **Diff from a patch.** `Diff` takes a patch and highlights per hunk; one whole-file hunk fixes the
  highlighting but makes huge files one hunk. Measured in step 0.
- **Linux renderer.** gpui-pre uses crates.io wgpu without Zed's Vulkan fix; Linux needs a Vulkan
  or GL driver. The smoke run on CI uses mesa.
- **macOS 15 floor.** Owners of older Macs stay on v0.3.0.
- **Build cost.** 835 locked packages, an 86 s cold check, 1 GB of target after a check; debug
  builds need opt-level 3 for the kit's crates to be usable.
- **Windows coverage.** No pixel tests there, and about 55 core tests stay Unix-only until ported
  (docs/TECHDEBT.md).
