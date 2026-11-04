# Detached — native client plan

The same client without a browser: a Rust UI on GPUI (via gpui-kit) instead of Tauri, Svelte,
Monaco and a web view. This file is how it gets built: decisions, stack, layout, design, the
parallel verticals with their contracts, and the steps with their Done-when.

Decided with the owner on 2026-11-04:
- Go native, no browser. The Tauri client's black screen on Windows is not fixed; the native client
  replaces it.
- Mermaid renders in Rust; a block that fails shows its source.
- Plan first, reviewed with revmux; the work runs as parallel verticals after a shared foundation.

**Status (2026-11-04): plan reviewed (revmux, 5 rounds, 28 findings fixed). Next: step 0 (foundation).**

The parity bar is the Tauri client as of v0.3.0 plus the strip's Log/Pipelines buttons. What it
does is surveyed in [docs/native/](docs/native/): [shell.md](docs/native/shell.md) (window, top bar,
sidebar, workspaces, branches, palette, settings, keymap, design tokens),
[files.md](docs/native/files.md) (viewer, diff, tree, Changes, find, markdown),
[history.md](docs/native/history.md) (log, graph, MRs, pipelines, job logs, the fake forge),
[core.md](docs/native/core.md) (the backend's Tauri boundary) and
[gpui-kit.md](docs/native/gpui-kit.md) (what the kit can and cannot do). Every section has an owner
in [Parity owners](#parity-owners); the owner ticks it off when its behavior is ported, ported
differently (say how, in the commit), or dropped with the owner's OK. These files go when the Tauri
client goes.

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
| Packaging | decided in V6: `cargo-packager` (`.app`, NSIS, deb, AppImage) unless something better is current then |
| Auto-update | ours: `latest.json` in the latest GitHub release, minisign signature per file |
| Terminal | step 3: a port of `zortax/gpui-terminal` (MIT/Apache-2.0, `alacritty_terminal`) onto gpui-pre |
| Tests | `cargo test` for core and for ported logic; the kit's headless UI tests (`#[gpui_kit::test]`) against temp repos and a fake forge, on macOS, Windows and Linux runners |

Not used: upstream `gpui` 0.2.2 from crates.io (a year old and type-incompatible with gpui-pre),
Zed's `terminal`/`terminal_view` (they need Zed's workspace).

Platform floors: macOS 15+ (the kit's stated minimum), Windows 10+, Linux with Vulkan or GL and
xdg-desktop-portal. The Tauri client runs on older macOS; v0.3.0 stays downloadable for it.

## Layout

```
Cargo.toml                 workspace (resolver 3, edition 2024), [workspace.package], shared pins
crates/core/               package detached-core: the backend from src-tauri/src minus the commands
  src/paths.rs             config_dir(), DETACHED_CONFIG_DIR, the workbench rename
  src/api.rs               one function per command, taking op/query ids where today's commands do
  src/selftest.rs          --self-test <report>, unchanged (unix)
  src/bin/e2e-bridge.rs    for the Tauri Playwright suite until V6 removes it
crates/forge-fake/         package detached-forge-fake: e2e/forge.ts in Rust
crates/app/                package detached-app, binary detached-native (detached from V6 on)
  src/main.rs              args (--self-test, --smoke), window, menus, boot
  src/store.rs             app-wide state entity
  src/blocking.rs          runs core on worker threads, back on the UI thread with the result
  src/cache.rs             last-value cache with the 30 s focus-refresh skip
  src/util.rs              helpers several verticals use: path names, change letters, times, fuzzy
  src/keymap.rs            defaults, keymap.json, contexts, Double Shift
  src/actions.rs           action ids, registration, run
  src/nav.rs               back/forward and recent files
  src/ui_state.rs          ui.json
  src/ui/                  theme tokens, icons, panel, icon button, segmented control, row, grip
  src/shell.rs             strip, top bar slots, sidebar, tool, main and bottom slots, sizes
  src/status_bar.rs, notices.rs
  src/repos/               V1: sidebar, workspaces, repo switcher, branches, sync, settings, accounts;
                           api.rs is the contract
  src/files/               V2: viewer, diff, tree, changes, markdown, mermaid; api.rs is the contract
  src/search/              V3: palette, find in files
  src/history/             V4: log, graph, commit details; api.rs is the contract
  src/forge/               V5: MR list and page, threads, merge widget, MR chip, pipelines, job log
  src/update.rs            V6
  tests/                   UI tests, one file per vertical, shared fixtures in tests/common
src-tauri/, src/, e2e/     the Tauri client on top of crates/core until V6 removes it
```

The Tauri package keeps its name `detached` and its binary, so `Detached.app/Contents/MacOS/detached`
(which `install.sh` quits before replacing) stays put until V6. Cargo's `target/` moves to the root.

## Carried over unchanged

- Config directory `dev.wdevs.detached` with the same path per OS as Tauri computes it
  (docs/native/core.md §2), `settings.json`, `accounts.json`, `keymap.json` with the same action ids
  and chord syntax, tokens under service `workbench`.
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
  normal dependency of `crates/app`; only test builds turn it on. `e2e-bridge` compiles to an empty
  `main` off Unix, so workspace clippy passes on Windows.

**Two clients, one machine.** While both exist, `paths::config_dir()` honors `DETACHED_CONFIG_DIR`.
`make app` runs the native client with it set to a sibling `dev.wdevs.detached.native`, copied once
from the real directory on first run, so the owner can use both clients without them writing the
same files. Tokens stay shared (same Keychain service, same signing identity, same account ids):
removing or re-saving an account in one client changes its token for the other too, which is
accepted. The shipped native client (V6) uses the real directory.

**Threads.** Core calls block, some for minutes (a fetch waits up to 120 s of silence, holding the
repo lock). They never run on GPUI's executors: `blocking::run(cx, f)` sends `f` to a pool of std
threads (16, enough for fetch all's parallelism) and resumes on the UI thread with the result.
Streams (log rows, find matches, progress lines) come back over a channel drained by a foreground
task that batches per frame. Each op or query has an id from the store; dropping the view or
starting a new query cancels the old id.

**Store.** One GPUI entity holds what `src/lib/app.svelte.ts` holds today: repos, workspaces,
current repo, statuses, outcomes, ops, notices, the status message, what the main area shows
(`shown: Option<Doc>`), which tool and bottom panel are open (`tool`, `bottom`), the per-repo
session maps other verticals read (tree ref `refs`, Compare base `compare`) and branch names per
repo for the ref inputs (`branch_names`, loaded on demand). Views observe it and read what they
need. Refresh on window activation, status after each op, the same coalescing as `refreshAll`.

**Keymap.** `keymap.rs` keeps the action ids of docs/KEYMAP.md, parses the same chord strings and
the `keymap.json` overrides into `KeyBinding::load` (an override that does not parse unbinds the
action, as now; parsing never panics). Bindings carry contexts so text fields and the editor keep
their own keys. Double Shift comes from `observe_keystrokes`: two lone Shift releases within 300 ms.
Off macOS, Cmd is Ctrl and the VCS popup is Alt+`, as today. Native menus (macOS) list the actions
with their current chords and are rebuilt when the keymap changes.

**Window.** macOS: transparent title bar, traffic lights at (24, 35), the top bar as the drag
region. Windows and Linux: the kit's title bar with window controls at the right of the top bar.
Minimum 900×600, default 1400×900. The Viewer stays alive under MR and job pages, as today
(files.md §0).

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
  | find; palette (its Cmd+, opens V1's Settings) | V3 |
  | mr; pipelines (their Open Settings opens V1's Settings) | V5 |

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
| F foundation | workspace, `crates/core`, `crates/forge-fake` control and project routes, `main`, `store`, `blocking`, `cache`, `util`, `keymap`, `actions`, `nav`, `ui_state`, `ui/` (including the context-menu helper), `shell`, `status_bar`, `notices`, the first version of `src/files/api.rs`, `src/repos/api.rs` and `src/history/api.rs` (contracts and stubs), test fixtures, Makefile, CI | — |
| V1 repos | `src/repos/` | store, shell slots, forge-fake `/user` and `/version` for account verify |
| V2 files | `src/files/` | store (`refs`, `compare`), nav, `core::api::file_patch` |
| V3 search | `src/search/` | actions, nav, store (`refs`), `files::open`, `repos::checkout`, `repos::open_settings` |
| V4 history | `src/history/` | shell's bottom slot, `files::show_diff`, `repos::checkout`, `store.shown()` |
| V5 forge | `src/forge/`, forge-fake MR, pipeline and job routes | top-bar slot, `files::DiffView` with annotations, `files::Markdown`, `repos::open_settings`, `cache`, `store.show`, `nav::open` with `DocRef`, `history::CommitDetails` |
| V6 ship | `src/update.rs`, packaging, `.github/workflows/`, `scripts/`, `install.sh`, the Tauri client's removal | everything, for the release |
| V7 terminal | `src/terminal/` | shell's bottom slot |

### Contracts

Fixed by F before the verticals start. Changing an existing signature or meaning takes a plan
edit agreed with the owner. Adding is lighter: a vertical that finds a missing piece (a field, a
variant, a function) adds it with a stub to the provider's `api.rs` or the store in its own small
commit on main, with the matching line in this section, without changing what is there; the
provider then fills it in.

- **Store:** fields and events above; `store.current()`, `store.status(repo)`, `store.run_op(...)`,
  `store.tree_ref(repo)`, `store.compare(repo)`, `store.shown()`, `store.show(Doc)`, and
  `store.open_tool(Tool)`, `store.open_bottom(Bottom)`, which open that panel (never toggle; the
  `panel.*` actions toggle), for Compare with Base (V1 opens Changes), file history (V4 opens Log)
  and Run Pipeline and the badge (V5 open Pipelines).
- **Threads:** `blocking::run`, `blocking::stream`, the op and query id rules.
- **Actions:** `actions::register(id, title, group, handler) -> Registration`. The latest
  registration of an id wins; dropping it restores the previous one (how the MR page's diff takes
  F7, Shift+F7 and Cmd+L from the main viewer while mounted). `actions::run(id) -> bool` says
  whether a handler ran. A vertical adds its ids to the list in `actions.rs` and default chords to
  `keymap.rs` (append-only). Each action's handler is registered by the vertical that owns its
  behavior; step 0 adds an owner column to docs/KEYMAP.md's table, covering every action of today's
  `registerActions` too: `panel.*` and `nav.*` → F; `vcs.update`, `vcs.fetch`, `vcs.fetchAll`,
  `vcs.pullAll`, `vcs.checkout`, `vcs.newBranch`, `vcs.deleteBranch`, `vcs.prune`, `app.settings`
  → V1; the palette modes (`search.everywhere`, `file.goto`, `action.find`, `file.recent`,
  `vcs.popup`) and `find.inFiles` → V3; viewer and diff actions → V2; `vcs.fileHistory` → V4;
  `mr.*`, `pipeline.*`, `panel.mergeRequests` → V5.
- **Shell slots:** `Tool::{Changes, Files, Mrs}`, `Bottom::{Log, Pipelines, Terminal}`,
  `Doc::{File, Diff, Mr, Job}`, and the top bar's `TopBar::Left` (V1: workspaces, repo switcher,
  branch widget), `TopBar::Center` (V5: MR chip, pipeline badge), `TopBar::Right` (V3: search pill;
  V1: Fetch All, Pull All). A vertical provides each of its views through one constructor; F renders
  stubs until then.
- **Cross-vertical calls** live in an `api.rs` in the providing vertical's directory. F writes the
  first version with the types and stub bodies; the provider implements the bodies without changing
  signatures; callers build against the stubs from day one.
- **Repos API** (`src/repos/api.rs`): `repos::checkout(cx, repo, CheckoutTarget)` (core's type;
  smart checkout with its dialogs), `repos::open_settings(cx, Section)` with
  `Section::{Accounts, Repos, Git}`.
- **Files API** (`src/files/api.rs`):
  - `Location { repo, rev: Option<String>, path, line: Option<u32>, doc: Option<DocRef> }` with
    `DocRef::{Mr { repo, number }, Job { repo, project, id, name }}`, so MR pages and job logs are
    in back/forward as today;
    `DiffDoc { repo, path, orig_path, left: Side, right: Side }` (`Side` from core);
  - `files::open(cx, Location)`; `files::show_diff(cx, DiffDoc, focus: bool)`, where `focus` moves
    focus into the diff once it loads (double-click or Enter) and `false` leaves it in the caller's
    list (a click or arrow keys stepping through files);
  - `DiffView::new(doc, DiffOptions { side_by_side: Option<bool>, annotations, on_add, commentable }, cx)`;
  - `Markdown::new(text, Links, breaks: bool, cx)`, `Links::Repo { repo, rev, path }` or
    `Links::Forge { project_url, base_url, gitlab: bool }` (GitLab resolves `/uploads/…` under the
    project and shows its upload images as links, history.md §2.7).
- **History API** (`src/history/api.rs`): `CommitDetails::new(repo, shas, cx)`, the view the log's
  side panel and the MR page's Commits tab both show.
- **Helpers** (`util.rs`, F): `base_name`, `dir_name`, `folder_name`, `in_folder`, the change
  `letter` and `status_of`, `when` (commit and pipeline times), `fuzzy::filter`/`segments`; the
  context-menu helper in `ui/`. A vertical's own helpers stay in its directory.
- **Navigation:** `nav::open(Location)` records and shows any location (file, MR, job),
  back/forward, recent files (files only) with today's caps (50, 100).
- **UI:** `ui` primitives and theme tokens; a vertical needing a new primitive adds it to `ui/` in
  its own commit. `ui_state::get/set` with the vertical's key prefix.
- **Fixtures:** `Sandbox` repos, `settle`/`wait_until`, the forge fake, `open_app(cx)` returning a
  test window with the store booted.
- **Forge fake:** F ports the control routes (`/health`, `/reset` with the seed of projects, bare
  repos and MR head SHAs, `/__requests`, `/__writes`, `/__delay`), auth, pagination, `/user`,
  `/version` and project lookup (GitLab by path or id, GitHub repo). V5 adds the MR, pipeline, job
  and GraphQL routes of history.md §6.2.

### Parity owners

| Section | Owner |
| --- | --- |
| shell.md §0, §1.2, §1.11–1.14, §2.1 (store state, ops, notices, refresh), §2.2, §2.3, §2.5–2.8, §3 (keys of F), §5 | F |
| shell.md §1.1 (top bar shell) | F; its widgets per the slot list |
| shell.md §1.3–1.7, §1.9, §1.10, §2.1 (the rest: checkout and its notices, recent branches, sync, prune, add folders, `settingsCall`, and the handlers per the actions' owner column), §2.4, §3 (sidebar keys), §4 | V1 |
| shell.md §1.8; files.md §1.4, §1.6, §4 | V3 |
| files.md §0, §1.7 | F (store, nav, actions, viewer lifetime), V2 (viewer) |
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
