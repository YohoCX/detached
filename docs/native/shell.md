<!-- Generated from the Tauri client on 2026-11-04 by read-only survey agents; file:line refers to that code. -->
# Parity inventory: app shell and repo management UI (Detached, Svelte 5 frontend)

I read every file in scope in full. To check rules that the UI relies on, I also read `src/lib/files.ts` (`folderName`, `inFolder`), `src/lib/fuzzy.ts`, `e2e/fixture.ts`, `e2e/shim.ts`, `src-tauri/tauri.conf.json`, `src-tauri/src/settings.rs` and `src-tauri/src/git/discovery.rs`. Paths below are relative to the repo root.

---

## 0. Bootstrap and global document behavior

### `src/main.ts`
- **Text fields (main.ts:5-11):** a `focusin` listener on `document` sets `autocorrect=off`, `autocapitalize=off`, `autocomplete=off` and `spellcheck=false` on every `input` and `textarea` when it gets focus. The reason given: fields hold paths, refs and queries, so there should be no macOS autocorrect bubble or capitalization.
- **Context menu (main.ts:14-16):** the web view's own context menu (Reload, Inspect Element) is suppressed everywhere except inside `input`, `textarea` and `.monaco-editor`.
- **Mount (main.ts:18):** mounts `App` into `#app`. `index.html` loads `/src/app.css` and `/src/main.ts`.

### Window chrome (`src-tauri/tauri.conf.json`)
- Window "Detached", 1400×900, minimum 900×600, resizable.
- `titleBarStyle: "Overlay"`, `hiddenTitle: true`, `trafficLightPosition: {x: 24, y: 35}`.
- The capabilities allow `core:window:allow-start-dragging`, `allow-internal-toggle-maximize`, `dialog:allow-open`, and opener URLs (http/https).
- **macOS:** the top bar gets `padding-left: 90px` so the traffic lights have room (TopBar.svelte:19, :67).
- **Drag regions:** `data-tauri-drag-region` is set on the TopBar `<header>` (TopBar.svelte:19), on the StatusBar root and on its `.fill` spacer (StatusBar.svelte:6, :12). The comment at TopBar.svelte:18 explains that only elements carrying the attribute drag the window, so empty space gets it too. Double-click to maximize comes from Tauri's internal toggle-maximize.

### `src/App.svelte`: shell layout and startup
**Startup (App.svelte:23-32), on mount:**
1. `loadRepos()`.
2. `api.keymapOverrides().then(setOverrides, () => {})`. Errors are ignored.
3. `registerActions()`.
4. `install(run)`, the keymap's capture-phase listener.
5. On unmount: unregister the actions and uninstall the keymap.

**Refresh on focus (App.svelte:40):** `<svelte:window onfocus={refreshAll} />`. Every window focus rescans the folders and every repo's status.

**Layout (App.svelte:42-81, styles :83-96):**
- `.app` is a column with `padding: 10px` and `gap: 10px`, filling the window. It holds the `TopBar`, then `.body` (a row with `gap: 10px`).
- **Left strip** (`nav.strip panel`): 48px wide, `padding: 8px 0`, `gap: 6px`. Its buttons are 34×34 with radius 10. The active state `.on` uses `background: var(--selected)`, `color: var(--fg)` and `box-shadow: 0 0 18px var(--selected)` (a glow). Buttons, top to bottom:
  1. **Repositories** (icon `repo`, title "Repositories"): toggles `app.sidebar`. `.on` while the sidebar is shown. (:46)
  2. **Project** (icon `tree`, title `Project ⌘1`): toggles `app.tool` between `'files'` and null. (:47)
  3. **Changes** (icon `changes`, title "Changes"): toggles `'changes'`. (:48)
  4. **Merge Requests** (icon `mr`): toggles `'mrs'`. (:49)
  5. **Git Log** (icon `log`, class `push`, so `margin-top: auto` pins it and the buttons below to the bottom; title `Git Log ⌘9`): `run('panel.log')`. `.on` when `app.bottom === 'log'`. (:50)
  6. **Pipelines** (icon `pipeline`): `run('panel.pipelines')`. `.on` when `bottom === 'pipelines'`. (:51)
  7. **Settings** (icon `gear`, title `Settings ⌘,`): sets `app.settings = true`. It has no `.on` state. (:52)
  - The tool buttons are mutually exclusive: one `app.tool`, and clicking the active one closes it (`tool()` at :36).
- **Sidebar pane** (`aside.repos panel`): shown only if `app.sidebar`. Width `reposWidth` defaults to 280. A `Grip key="repos" edge="right"` resizes it within 200–600. (:54-56)
- **`.work`** is a column holding `.top` (a row) and an optional `BottomPanel` (when `app.bottom` is set). (:57-75)
  - **Tool pane** (`aside.tool panel`): shown when `app.tool` is set. Width `toolWidth` defaults to 300. A `Grip key="tool" edge="right"` resizes it within 220–800. It shows `FileTree`, `MrList` or `Changes` for `app.current`. (:59-66)
  - **`<main>`:** radius 14, `background: var(--code)`, `1px solid var(--line)`, `box-shadow: var(--panel-shadow)`, `overflow: hidden`. The `Viewer` always stays mounted in `.doc`. It is hidden with `display: none` when `view.shown.kind` is `'mr'` or `'job'`, so its editors survive. `MrPage` or `JobLog` render on top of it in that case. (:67-72, :95)
- **Overlays after `.app`:** `<Notices/>` always; `<Palette/>` if `app.palette`; `<Settings/>` if `app.settings`; `<FindInFiles/>` if `view.find`. (:78-81)
- `aside` panes are `position: relative; flex: none; overflow: hidden` (:92).

**Initial UI state (`app.svelte.ts:20-36`), none of it persisted:**
- `sidebar: true`
- `tool: 'changes'`, so the Changes panel is open at startup
- `bottom: null`
- `palette: null`, `paletteAll: false`
- `branches: null`, `settings: false`
- `current: null`, which becomes the first repo after listing

---

## 1. Components

### 1.1 TopBar (`src/components/TopBar.svelte`)
A `header.panel`, height 46px, `padding: 0 8px`, `gap: 6px` (:66). `.mac` adds `padding-left: 90px`. From left to right:

1. **Workspaces widget** (:20-28). Rendered only when `app.workspaces.length > 0`.
   - Button `.widget`, title "Workspaces".
   - Label: the names of the active workspaces joined with `", "`, or `"No workspace"`. A `down` chevron (12px) follows.
   - Click toggles the `Workspaces` popup, anchored under the widget.
2. **Repo widget** (:29-35). Button `.widget.repo`, title "Switch repository".
   - Styled like an input: `background: var(--input)`, `1px solid var(--line)`, `max-width: 320px`.
   - Disabled when there are no repos.
   - Label: the bold `nameOf(app.current)`, or `"No repository"`, ellipsized.
   - Click toggles `RepoSwitcher`.
3. **Branch widget** (:36-45). Title "Branches". Disabled when `!app.current`.
   - Content: an accent-colored `branch` icon (14px), then the branch name. A detached HEAD shows `` `${head.slice(0,8)} (detached)` `` (or `? (detached)`). Before any status has loaded it shows `—`. Muted `↑N` and `↓N` follow when nonzero, then a chevron.
   - Click: `app.branches = app.branches ? null : 'checkout'`.
   - The popup renders inside `{#key app.branches}`, so a change of intent (checkout, new, delete) remounts it.
4. **`<MrChip repo>`** and **`<PipelineBadge repo>`**. These are outside this area.
5. **`<StatusBar/>`**. It sits inside the top bar, between the badges and the search pill, and takes `flex: 1`. There is no bottom status bar.
6. **Search pill** (:49-51). 240×30, radius 999, `background: var(--hover)`, muted text.
   - Contents: a `search` icon, the text "Search everywhere", and a `<kbd>` showing `chordFor('search.everywhere')`: `⇧⇧` on macOS, `Shift Shift` elsewhere.
   - Click: `run('search.everywhere')`.
7. **Fetch All and Pull All** (:52-62). `.btn.sync`, 30px high. Pull All is `.primary` (dark fill).
   - Titles: "Fetch All Repositories" and "Pull All Repositories".
   - Disabled while any `app.sync` is running or when there are no repos.
   - While a bulk sync of that kind runs, the label is replaced by `done/total` in tabular numerals.
   - Click: `syncRepos(kind)`.

Other styles: `.widget` is 32px high, `padding: 0 10px`, radius 10, `gap: 7px`, with a `var(--hover)` background on hover. `kbd` is 11px with a `var(--line)` border and radius 6.

### 1.2 StatusBar (`src/components/StatusBar.svelte`)
Lives inside the TopBar. Font 12px, `var(--muted)`, `gap: 12px`.

- **Message** (:7-11). `app.message` is a single slot that is never auto-cleared.
  - Shown as a button: ellipsized, `max-width: 50%`, title holds the full text, red (`.err`) if `error`.
  - **Clicking it clears the message.**
- **Spacer:** a `.fill` that also acts as a drag region (:12).
- **Running ops** (:13-20), one chip per `app.ops` entry:
  - CSS spinner: 10px, 1.5px `var(--accent)` border with a transparent right side, 0.8s linear rotation.
  - Bold label (e.g. "Fetch", "Pull", "Prune", "Delete origin/x"), then the repo name.
  - The latest progress line, muted and ellipsized. The chip's title is that progress line.
  - A × button titled "Cancel" that calls `cancelOp(op.id)`, i.e. `api.cancelOp`. Errors go to the message slot.
  - Chip `max-width: 320px`.

### 1.3 Sidebar (`src/components/Sidebar.svelte`) [ported]

**Header** (:56-62): height 44, `padding: 0 8px 0 16px`.
- Title "Repositories", 14px, weight 800.
- `+` button titled "Add folder" → `addFolders()`. Shown only when there are no workspaces yet, or at least one is active.
- Refresh button titled "Refresh" → `refreshAll`.

**Body states:**
- **Rows exist:** a `VirtualList` with a fixed **40px row height for every row kind** (:64).
- **Workspaces exist but none is active:** "No workspace is on. Turn one on in the top bar." (:141-142)
- **Otherwise:** "No repositories yet." and a primary button "Add folder…" → `addFolders()` (:143-147). The e2e fixture uses this button.

**Row model** (:24-50, the non-obvious rules):
- Rows are built per **active workspace**, in settings order.
- **Workspace header rows** appear only when **more than one** workspace is active (:30). The comment says headers only tell several workspaces apart. A collapsed workspace (key `workspace:<name>`) hides everything under it.
- Each workspace's folders are **deduped after stripping trailing slashes** (:34).
- **Repos per folder:** `app.repos` filtered by `inFolder(path, folder)`, a path-prefix test (`path === folder || path.startsWith(folder + '/')`, files.ts:30). It does not use `RepoInfo.folder`, because a repo listed once can sit in folders of several workspaces. **Worktrees are left out of the folder list when their main repo (`worktreeOf`) is listed anywhere** (:36).
- **Solo folder:** when a folder holds exactly one repo and that repo's path equals the folder, there is no folder header. Removing that repo removes the folder (:37-40).
- A collapsed folder header (key: the folder path) hides its repos.
- **Under each repo**, its worktrees follow (all `app.repos` with `worktreeOf === repo.path`), unless the repo's path is in `collapsed`. Worktree rows have `worktree: true`, `solo: false` (:41-46).
- **A repo in two active workspaces appears under each of them.** The backend lists it once, but the sidebar walks each workspace's folders (e2e: `repoRow('lib')` count is 2).
- **Order** comes from the backend: discovery sorts by path, walks to depth 3, skips dot-directories and `node_modules`, stops at a `.git`, and skips excluded paths (discovery.rs:6-64). `list_repos` dedupes by path across active workspaces; the first workspace wins (settings.rs:100-110).
- **Repo name** is the path below its folder (e.g. `group/api`), or the directory name when the repo is the folder itself (discovery.rs:31-39). Worktree rows show `folderName(path)` instead, the last path segment (Sidebar.svelte:114).

**Collapse persistence** (:17-22): a `SvelteSet` loaded from localStorage `sidebar.collapsed` (a JSON array) and saved on every toggle. It mixes three kinds of key: `workspace:<name>`, folder paths, and repo paths (for worktree collapse).

**Workspace header row** (:66-76):
- `.folder.workspace`: 12px, `var(--fg)`, uppercase, letter-spacing 0.06em, weight 700, aligned to the bottom (`align-items: flex-end`, `padding-bottom: 6px`).
- Chevron rotated 90° when open, with a 0.1s transition.
- **Click** toggles collapse.
- **Right-click** opens `workspaceMenu` (§1.4).
- **Hover-only** `+` button titled `Add folder to <name>` → `addFolders(name)`. It stops propagation.

**Folder row** (:77-87):
- `.folder`: 11px, muted, uppercase, 0.06em letter-spacing, weight 700, bottom-aligned. Title is the full folder path. Chevron, `folder` icon, label `folderName(folder)`.
- **Click** toggles collapse.
- **Right-click** opens `folderMenu(workspace, folder, collapsed, toggle)`.
- **Hover-only** `−` button titled "Remove folder" → `removeFolder`. **There is no confirmation.**

**Repo row** (:88-137):
- **Classes:** `.repo` has `padding: 3px 10px 3px 28px`, radius 10, `margin: 0 6px` and a hover background. `.worktree` uses `padding-left: 44px`. `.selected` uses `var(--selected)` plus an inset 1px `var(--selected-ring)`.
- **Tooltip:** the repo path, plus `\n<last commit subject>` when known.
- **Click** sets `app.current`.
- **Right-click** selects the repo first, then opens `repoMenu(workspace, path, solo)`.
- **Line 1** (13px, weight 600, 17px high):
  - If the repo has worktrees: a chevron toggle button, absolutely placed at `left: 8px`. Title `Show|Hide N worktree(s)`. It toggles the repo-path key and stops propagation.
  - Name (ellipsis, flex 1).
  - In-progress badge: `MERGING`, `REBASING`, `CHERRY-PICKING`, `REVERTING` or `BISECTING`. 10px bold, `background: var(--warning)`, `color: var(--accent-fg)`, radius 4 (:52, :115).
  - Dirty count: the number of changed files, bold 11px in `var(--warning)`, titled "Changed files".
  - Hover-only × button titled "Remove from list". For a solo repo it calls `removeFolder`, otherwise `excludeRepo`. No confirmation.
  - Status dot (7px, glowing `box-shadow 0 0 8px currentColor`), colored by `toneOf`.
- **Line 2** (12px, muted):
  - If the status call failed: the error string, red, ellipsized (`.err`).
  - Otherwise: a 12px branch icon; the branch or `<sha8> (detached)` (`max-width: 75%`); `↑N` and `↓N` in `var(--fg)`; then, if a sync outcome exists, `.outcome`, right-aligned (`margin-left: auto`), red when it failed, titled with its text.

**`toneOf`** (app.svelte.ts:49-55), checked in order:
1. Status is an error string → `error`.
2. No status yet → no dot.
3. Changes or an operation in progress → `warning`.
4. Ahead or behind → `accent`.
5. On a branch without an upstream → `deleted` (gray).
6. Otherwise → `success`.

A detached HEAD with no changes and no ahead/behind therefore shows `success`.

**Hover buttons:** the `.hover` class is `visibility: hidden` and becomes visible on `.repo:hover` or `.folder:hover`. These buttons are 18px (:175-176).

Text cannot be selected anywhere (global `user-select: none`), so double-clicking empty space selects nothing.

### 1.4 Native context menus (`src/lib/menu.ts`) [ported differently: OS-drawn menus on macOS and Windows, kit-drawn on Linux]
Built with Tauri `Menu.new` and `PredefinedMenuItem` separators, then `.popup()` (:10-19). Items carry an `enabled` flag.

**Repo menu** (`repoMenu`, :23-46):

| Item | Action |
| --- | --- |
| Fetch | `syncOne('fetch', repo)` |
| Pull | `syncOne('pull', repo)` |
| — | separator |
| Compare with Base Branch | Sets `app.current = repo` and `app.tool = 'changes'`, then `compare.set(repo, compare.get(repo) ?? '')`. `''` means the default branch; an existing base is kept. |
| Branches… | Sets `app.current` and `app.branches = 'checkout'` |
| New Branch… | Sets `app.current` and `app.branches = 'new'` |
| — | separator |
| `Open in Finder` / `Open in Explorer` / `Open in File Manager` | `api.openFolder(repo)`. The name depends on the platform (platform.ts:5). |
| Open in Zed | `api.openInZed(repo, '', null)` |
| Copy Path | `navigator.clipboard.writeText`, then the status message `Copied <path>` |
| — | separator |
| Remove from List | Solo repo: `removeFolder(workspace, repo)`. Otherwise: `excludeRepo(workspace, repo)`. |

**Folder menu** (`folderMenu`, :48-60). The repo set is `app.repos` filtered by `inFolder`, which includes worktrees.

| Item | Action |
| --- | --- |
| Fetch All in Folder | `syncRepos('fetch', repos)`. Disabled while `app.sync` is running. |
| Pull All in Folder | `syncRepos('pull', repos)`. Disabled while `app.sync` is running. |
| — | separator |
| Expand / Collapse | The label depends on the current state; calls the toggle. |
| `Open in <file manager>` | `api.openFolder(folder)` |
| Copy Path | Same as the repo menu |
| — | separator |
| Remove Folder | `removeFolder(workspace, folder)` |

**Workspace menu** (`workspaceMenu`, :62-66):

| Item | Action |
| --- | --- |
| Add Folder… | `addFolders(workspace)` |
| Turn Off | `settingsCall(() => api.setWorkspaceActive(workspace, false))` |

Errors from `openFolder` and `openInZed` in these menus are not caught (no `say`).

### 1.5 Workspaces popup (`src/components/Workspaces.svelte`) [ported]

**Container:**
- A transparent full-screen `.backdrop` (z-index 10). Clicking it closes the popup.
- `.popup.glass`, `role=dialog`, `aria-label="Workspaces"`. Absolutely placed under the widget: `top: 100%`, `margin-top: 8px`, width 320, `padding: 8px 0`, z-index 11 (:61-62).
- **Escape** (window keydown) closes it unless a rename is in progress (:26).

**One row per workspace**, in settings order (:31-56):
- `.item`, `role=checkbox`, `aria-checked={active}`, `tabindex=0`. Height 32, `margin: 0 8px`, radius 9, hover background.
- **Tooltip:** the workspace's folders joined by newlines.
- **Click, Space or Enter** toggles the workspace via `setWorkspaceActive(name, !active)`. Several workspaces can be on at once.
- **Content:** an accent check icon when active; the name (weight 600); muted `N folder` or `N folders`.
- **Hover-only buttons:**
  - Edit icon titled `Rename <name>`: switches the row to an inline input.
  - × titled `Delete <name>`: `removeWorkspace`. **No confirmation.**

**Rename input** (:32-38):
- `aria-label="Workspace name"`, autofocused, prefilled with the current name.
- **Enter or blur** commits through `rename()`. It saves only when the trimmed value is non-empty and differs from the old name, by calling `saveWorkspace(renamed, from)`.
- **Escape** cancels (`editing = null`).
- Because blur also commits, a blur that fires when the input is removed would also trigger `rename`. That edge depends on the browser engine.

**"New workspace" input** at the bottom (:57): **Enter** creates the workspace via `saveWorkspace(created, null)` when the trimmed text is non-empty, then clears the field.

**Backend rules** (settings.rs:163-191):
- Names are trimmed.
- An empty name fails with "A workspace needs a name".
- A duplicate fails with `Workspace <name> already exists`.
- A new workspace starts **active** and empty.
- Rename fails if the target name exists.
- Errors surface through `settingsCall` → `say(err, true)` in the status message.

### 1.6 RepoSwitcher (`src/components/RepoSwitcher.svelte`) [ported]

**Container:** transparent backdrop (z 10, click closes); `.popup.glass`, `role=dialog`, `aria-label="Switch repository"`. Width 420, under the repo widget, `margin-top: 8px` (:58-59).

**Search input:** placeholder "Switch repository", 36px high, 14px font. **Autofocused** via an effect (:14).

**List** (`role=listbox`, `max-height: 420px`, scrolls):
- Contains **all `app.repos`** as one flat list, worktrees included.
- Filtered with `fuzzy.filter` on `name`: a case-insensitive subsequence match where whitespace in the query is ignored. With a query, results are sorted by score (tighter and earlier matches first). Without one, the original order is kept (fuzzy.ts:6-23).

**Selection:**
- **Initial selection is the current repo's index** in the unfiltered list (:9).
- Typing resets the selection to 0 (:35).

**Keys on the input** (:21-29):
- **↑ / ↓:** move the selection, clamped (no wrap), and scroll the row into view (`block: 'nearest'`).
- **Enter:** choose.
- **Escape:** close.
- All of these `preventDefault`.

**Mouse:** `mousemove` selects a row **only if `movementX` or `movementY` is nonzero**, so content scrolling under a resting pointer doesn't steal the selection. Click chooses.

**Row** (`role=option`, `aria-selected`, title is the repo path; 32px high, radius 9; selected uses the selected background and ring):
- Accent check icon when the repo is current.
- Name with the fuzzy match characters highlighted (`.hit`).
- Muted `folderName(folder)` **unless `folder === path`**, i.e. no folder label for a solo repo.
- Branch column (muted, `max-width: 45%`): the error string; or the branch (or the bare word `detached`) followed by ` ↑N` / ` ↓N`.
- Tone dot.

**Empty:** "No repositories match."

**Choosing** sets `app.current` and closes.

### 1.7 Branches popup (`src/components/Branches.svelte`) [ported]
Opened from the Branch widget (`checkout`), the `vcs.checkout`, `vcs.newBranch` and `vcs.deleteBranch` actions, and the repo menu's "Branches…" / "New Branch…".

**Captured at mount** (:24-26), not reactive: `repo = app.current`, `intent = app.branches`, `head = current branch or 'HEAD'`.

**Container:**
- Transparent backdrop (z 10, click closes).
- `.popup.glass`, `role=dialog`, `tabindex=-1`, width 440, `margin-top: 8px`, z-index 11, `outline: none`.
- **All keys are handled on the popup**, which has a single `onkeydown` (:179-204).
- **Focus** (:69-73): on every change of sub-view, the sub-view's `focus` element gets focus if it is connected, otherwise the popup itself.

**States:**
- No repo: "Select a repository first."
- Load error: the error in red.
- Not yet loaded: "Loading…".
- Empty after filtering: "No branches match."

**Load** (:43-51): `api.listBranches(repo)` once at mount, and again after any action that returns to the list.

**List view** (`sub.kind === 'list'`):
- **Search input:** placeholder "Search branches", or "Delete branch…" when the intent is delete.
- **Item order** (:53-65):
  1. When there is no query **and** the intent isn't delete, two fixed actions: **"New Branch…"** (plus icon) and **"Prune Remote Branches"** (minus icon).
  2. Section **RECENT**, only without a query: the repo's in-memory recent branches (most recent first) that exist locally and are **not the current branch**.
  3. Section **LOCAL**: local branches fuzzy-matched on `name`.
  4. Section **REMOTE**: remote branches fuzzy-matched on `remote/name`.
  - Section headers are 11px uppercase muted text and are skipped by selection. Empty sections are omitted.
- **Virtualized**, row height 28. The container is `min(items × 28, 448)` px high.
- **Selection:**
  - After every change of the list (typing, pasting, branches loaded) it resets to the first non-header item (:82-93).
  - ↑ / ↓ move to the next non-header item without wrapping and call `list.reveal(i)` (:75-81).
- **Enter before the branches have loaded** sets `enterPending`. Once they load, the first match is chosen automatically. Any typing (`oninput`) cancels the pending Enter (:84-93, :187, :218).
- **Enter or click on an item** (`choose`, :116-126):
  - "New Branch…" → the New view, starting at `HEAD`, labelled with the current branch.
  - "Prune Remote Branches" → closes the popup and runs `prune(repo)`.
  - **Delete intent:** a local branch → `del(name)`; a remote branch → the deleteRemote confirmation. The current branch isn't special-cased here, so the backend decides whether it can be deleted.
  - **Current local branch** → opens its action menu instead of checking it out.
  - Anything else → `doCheckout`: closes the popup and calls `checkout(repo, {kind: local|remote, …})`.
- **ArrowRight** on a branch row opens its action menu.
- A hover/selected-only chevron button titled **"Actions"** opens the menu. It is visible only on the selected row (`.item.selected .more`).
- **Local row content:** branch icon (accent when current); the name with highlights (bold accent when current); a `gone` badge (warning outline, titled "Upstream branch is gone"); `↑N`, `↓N`; the upstream name, muted, `max-width: 140px`.
- **Remote row content:** branch icon and `remote/name`.

**Menu view** (:159-174, :260-269):
- **Title row:** a `‹` back button, the branch's full name, and muted mono `sha8 subject`.
- **Items, in order:**
  1. "Checkout" (not for the current local branch)
  2. `New Branch from '<full>'…`
  3. For a remote branch: "Delete on Remote…" (the list ends here)
  4. For a local branch: "Set Tracking Branch…"
  5. "Unset Tracking Branch" (only if the branch has an upstream)
  6. "Delete" (not for the current branch)
- **Keys:** ↑ / ↓ clamp, Enter runs the item, ArrowLeft goes back to the list.
- **Mouse:** mousemove (with real movement) selects; click runs.

**New view** (:270-277):
- Title `New branch from <label>`.
- "Branch name" input; checkbox "Checkout branch", **checked by default**; "Create" (primary) button, disabled while busy or when the name is empty. Enter also creates.
- `create`: `api.createBranch(repo, name, start, false)`, then `say("Created branch <name>")`, then `checkout(repo, {local, name})` if the checkbox is ticked, then the popup closes.
- For `intent === 'new'`, the popup **opens directly in this view** (:33).

**Track view** (:278-289):
- Title `Tracking branch for <branch>`.
- "Remote branch" search, fuzzy over `remote/name`, limited to 200 results. Typing resets the selection.
- ↑ / ↓ clamp. Enter or click → `setUpstream(repo, branch, remote/name)`.

**Not-merged view** (:290-304):
- Title `<name> is not fully merged`.
- Text: `These commits are only on <name>; deleting it loses them:`.
- A bordered, scrollable list of `sha8 subject author`.
- Buttons: "Cancel" (back to the list) and "Delete anyway" (`.btn.danger`, focused, disabled while busy) → `del(name, force = true)`.

**Delete-on-remote view** (:305-312):
- Title `Delete <name> on <remote>?`.
- "Cancel" or "Delete on remote" (danger, focused).
- On confirm: the popup closes, then `runOp(repo, "Delete <remote>/<name>", …api.deleteRemoteBranch)`. It streams progress and can be cancelled. Success: `say("Deleted <r>/<n> on the remote")`. Failure: an error notice `Delete of <r>/<n> failed`.

**`act` wrapper** (:100-114):
- Sets `busy`. The function can return the next sub-view (e.g. notMerged), or close the popup, or go back to the list and reload.
- Errors produce an error notice "Branch operation failed" with the error as body.
- Always ends with `refresh(repo)`.
- A successful delete says `Deleted branch <name>`.

**Escape** (:181-184):
- In the list view, or when the intent is new: closes the popup.
- In any other sub-view: back to the list.

### 1.8 Palette (`src/components/Palette.svelte`)

**Modes and titles** (:16-22):

| Mode | Title |
| --- | --- |
| everywhere | Search Everywhere |
| file | Go to File |
| action | Find Action |
| vcs | VCS Operations |
| recent | Recent Files |

**Container:**
- `.backdrop.scrim`, which tints the screen (z 20). Clicking it closes the palette.
- `.palette.glass`: fixed at `top: 12%`, centered horizontally, `width: min(680px, 90vw)`, radius 18, z-index 21 (:164-165).

**Head:** the mode title (14px, weight 800). In file and everywhere modes, a checkbox "All repositories" bound to `app.paletteAll`. That value is global session state and is not persisted.

**Query input:** 40px high, radius 12, 14px font.
- Placeholder: `Files in <repo>` (or `Files in no repository`) in file mode when not searching all repos; otherwise "Type to search".
- **On every mode change** (:87-91): the selection resets to 0 and the input gets focus. **The query is kept**, because the palette stays mounted when another palette chord is pressed while it is open.

**Re-pressing Go to File:** `openPalette` (app.svelte.ts:252-255) **toggles `paletteAll`** when Go to File is requested while it is already open; any other request switches the mode.

**File search** (:35-55):
- Runs only in file and everywhere modes, with a non-empty query and at least one repo.
- `api.quickOpen(repos, ref, query, limit)`. The limit is **50** in file mode and **10** in everywhere mode.
- `repos` is all repo paths when `paletteAll`, otherwise `[current]`.
- `ref`: when searching all repos, null (the working trees). For one repo, `refOf(current)`, the ref that repo's file tree is browsing.
- Stale responses are dropped with a sequence counter. Errors give an empty list.

**Branches** (everywhere mode only, :57-59): `api.listBranches(current)` is loaded once per mount.

**Entries** (:68-85):

| Mode | Entries |
| --- | --- |
| file | With a query: the quickOpen results. Without one: recent files. |
| recent | `nav.recent` (up to 50, deduped) fuzzy-filtered on the path |
| action | Every action, fuzzy-matched on its title |
| vcs | Actions in the `vcs` group |
| everywhere | Empty without a query. Otherwise files (up to 10), then branches (local and `remote/name`, fuzzy, top 10), then actions (top 10). |

**Rows** (30px high, radius 9):
- **File:** `tree` icon; the basename with highlights (bold); the directory, muted, with highlights; detail `<repo name>` plus ` @ <ref>` when a ref is set. The `split()` function maps highlight indices between the basename and the directory (:112-119).
- **Branch:** branch icon, the highlighted name, muted detail "Checkout".
- **Action:** a 14px spacer, the highlighted title (**muted when the action has no registered `run`**), and the chord in muted mono.

**Empty state:** "Nothing found". In everywhere mode with no query it reads "Search files, branches and actions" instead.

**Keys on the input** (:102-110):
- **Escape:** close.
- **Enter:** choose the selected entry.
- **↑ / ↓:** **wrap around** (modulo the entry count) and scroll the row into view.

**Mouse:** mousemove with real movement selects; click chooses.

**Choose** (:93-100), which always closes the palette first:
- File → `open({...ref, line: null})`, which records navigation history.
- Action → `run(id)`.
- Branch → `checkout(current, local|remote)`.

### 1.9 Settings dialog (`src/components/Settings.svelte`)

**Container:**
- `.backdrop.scrim` (z 30). Clicking it closes the dialog.
- `.dialog.glass`, `role=dialog`, `aria-label="Settings"`: fixed at `top: 8vh`, centered, `width: min(720px, 92vw)`, `max-height: 84vh`, radius 20, z-index 31.
- **Text in the dialog can be selected** (`user-select: text`).
- **Escape** (window keydown) closes it.
- **Header:** "Settings" (15px, weight 800) and a × button titled "Close". A `1px var(--line)` border sits below the header.
- **Body:** scrolls, `padding: 18px 20px`, a grid with `gap: 22px`. Section headings (`h3`) are 11px uppercase muted text with 0.06em letter-spacing.

**Load** (:27-39): on mount, `reload()` runs:
1. `getSettings`, which supplies the excluded repos across workspaces and `gitPath`.
2. `listAccounts`.
3. `repoAccounts(current)`.
4. `forgeHosts(current)`.

`envInfo` is also called. Every error goes to the status message via `say`.

**Sections:**

1. **Accounts** (:77-80). Renders `<Accounts/>`. The section has `onfocusin` → `reload()`, so focusing anything inside it re-reads the accounts for the selects below.

2. **Account for \<repo\>** (current repo only, :82-100). One row per https host:
   - The host (mono), its remotes (muted, joined with ", "), and a `<select>`.
   - The empty option reads "Choose an account" when `ambiguous`, otherwise "Automatic".
   - The options are the accounts whose `baseUrl` **contains** the host. Each shows `label || username`.
   - Change → `setRepoAccount(repo, id || null)`, then reload.
   - With no https hosts: "No https remotes; git uses your own credentials (ssh agent, keychain)."

3. **Merge requests for \<repo\>** (when there is a current repo and `forgeHosts` returned, :102-130):
   - **Remote select.** The empty option reads `Automatic (<mrRemote>)`, or "Choose a remote" when there is no automatic remote. The options are every host's remotes. Change → `setMrRemote`.
   - **One select per host**, listing all accounts. The empty option reads "Choose an account" when ambiguous, `Automatic (<label|username>)` when an automatic account matched, otherwise "No account fits". Change → `setForgeAccount(repo, host, id || null)`.
   - With no hosts: "No remote is on GitLab or GitHub."

4. **Removed repositories** (:132-143):
   - Lists every `excludedRepos` entry across workspaces as a mono path.
   - The workspace name is shown only when there is more than one workspace.
   - "Restore" → `includeRepo`, then `loadRepos`, then reload.
   - Empty: "None."

5. **Git** (:145-152):
   - The muted mono line `<gitVersion> · <gitPath>`.
   - Input with placeholder "git binary (empty: from your shell PATH)".
   - "Apply" → `setGitPath(trimmed value || null)`, which returns a new EnvInfo.

### 1.10 Accounts (`src/components/Accounts.svelte`, embedded in Settings)

**Header:** h2 "Accounts" (so the section shows a heading twice) and an "Add account" button, disabled while the form is open.

**Errors:** an inline red `<p>` (`attempt()` clears it each time).

**One row per account** (`.row`: `var(--hover)` background, `1px var(--line)` border, radius 12):
- Bold `label || baseUrl`.
- Muted `<Provider name> · <baseUrl> · <username>`.
- Check state: "Checking…", or green `Token ok: <user>`, or red `Check failed (<status>): <error>` (the status part only when there is one).
- **Buttons:**
  - "Check token" → `verifyAccount`. Disabled while checking. A rejected promise is turned into a failed result.
  - "Edit". Disabled while the form is open.
  - "Remove" is **two-step**: it switches to "Confirm remove" (danger) plus "Cancel".

**Empty** (hidden while the form is open): "No accounts. Add one to fetch and pull over https with a token."

**Form** (:103-149):
- **Provider select:**

  | Provider | Base URL set on change |
  | --- | --- |
  | GitHub | `https://github.com` |
  | GitHub Enterprise | empty |
  | GitLab.com | `https://gitlab.com` |
  | GitLab self-hosted | empty |

- **Base URL:** required. **Disabled when the provider has a fixed base.** Placeholder `https://git.example.com/gitlab`.
- **Username:** required. Placeholder "Your login" for the GitHub providers, "oauth2" for the GitLab ones.
- **Label:** placeholder "Work GitLab".
- **Token:** a password field. Placeholder "Leave empty to keep the stored token" when editing, "Personal access token" when new.
- **Save** (submit): the token is trimmed, or null when empty; `saveAccount`; the form closes; the account's previous check result is cleared; the list reloads; **if a token was given, the check runs automatically.**
- **Cancel** closes the form.
- A new account defaults to GitHub with `https://github.com`.

### 1.11 Notices (`src/components/Notices.svelte`)

**Stack:** fixed at `right: 16px; bottom: 34px`, width 380, `gap: 8px`, z-index 30. Newest notices are at the bottom (pushed in order).

**Each notice** (`.notice.glass.<level>`, `padding: 10px 12px 10px 14px`):
- **Title row** (bold): a leading 8px glowing dot colored by level (info = `--success`, warn = `--warning`, error = `--error`), the title, and a × button titled "Close" → `dismiss`.
- **Body:** a `<pre>` with `white-space: pre-wrap`, the UI font at 12px, `max-height: 200px` with scrolling. **Its text can be selected.**
- **Conflict file list**, when `repo` and `files` are set: rows of the mono path plus an "Open in Zed" button → `api.openInZed(repo, file, null)`. Errors go to `say`. The list is capped at 200px and scrolls.

**Lifetime** (app.svelte.ts:67-71): **info notices auto-dismiss after 6s**; warn and error notices stay until closed.

### 1.12 Grip (`src/components/Grip.svelte`)
- Positioned absolutely on one edge of a `position: relative` pane: `left`, `right` or `top`.
- **Size handling:**
  - The initial size is `Number(localStorage['size.<key>']) || default`, so 0 or NaN fall back to the default (:14).
  - Pointerdown captures the pointer. Pointermove sets `size = round(clamp(min, max, from + sign·Δ))`, where `sign` is +1 for `right` and −1 for `left` and `top`.
  - On `lostpointercapture` the size is saved to `size.<key>` (:18-27).
- **Visuals** (:34-39): z-index 3; hover background `var(--selected-ring)`; 4px thick.
  - `left`/`right` grips have `col-resize`, are inset 14px from top and bottom, and round their inner corners.
  - `top` grips have `row-resize` and are inset 14px from left and right.

### 1.13 VirtualList (`src/components/VirtualList.svelte`)
- Fixed row height (default 24); rows are absolutely positioned.
- Overscan of 5 rows above and below.
- The viewport height defaults to 400 until it is measured (`bind:clientHeight`). `contain: strict`.
- Exported `reveal(i)` scrolls the minimum amount needed to bring row `i` into view.
- Snippet `row(item, index)`.

### 1.14 Icon (`src/components/Icon.svelte`)
- SVG with `viewBox 0 0 16 16`, no fill, `stroke: currentColor`, `stroke-width: 1.3`, round caps and joins, `aria-hidden`. Default size 16.
- Names: `branch`, `fetch`, `pull`, `refresh`, `plus`, `minus`, `close`, `folder`, `repo`, `tree`, `changes`, `mr`, `chevron` (points right), `down`, `check`, `up`, `next`, `zed`, `search`, `edit`, `log`, `pipeline`, `gear`. Each is a single path `d` (:2-26).

---

## 2. State and logic modules

### 2.1 `src/lib/app.svelte.ts`

**Shared stores:**
- `statuses`: `SvelteMap<repo, RepoStatus | errorString>`.
- `outcomes`: `SvelteMap<repo, Outcome>`.
- `recentBranches`: a plain `Map<repo, string[]>`, **in memory and not persisted** (:39-41).

**`run(id)`** (:58-60): if no handler is registered, it says `<title> is not available yet` in the status message.

**`say(text, error)`** sets the single message slot.

**`refresh(repo)`**: `api.repoStatus`. On failure it stores the error string (:76-82).

**`setRepos(list)`** (:84-90):
- Replaces `app.repos`.
- Drops `statuses` and `outcomes` entries for repos no longer listed.
- If the current repo is gone or unset, **current becomes the first repo**, or null when there are none.

**`refreshAll()`** (:92-109):
1. If a refresh is already running, it sets `again` and returns. When the running pass finishes it loops once more, so calls coalesce.
2. Each pass: `api.listRepos()` (which rescans the disk, so new and deleted worktrees appear and disappear), `setRepos`, then `repoStatus` for every repo with **8 at a time** (`pool`).
3. Errors go to `say(…, true)`.

**`loadRepos(list?)`** (:111-121): applies `list` when given; `getSettings()` supplies `workspaces` and `fileNesting`; then `refreshAll()`.

**`settingsCall(f)`** (:123-129): runs a settings command (they all return `RepoInfo[]`), then `loadRepos(result)`. Errors → `say`.

**`addFolders(workspace = first active workspace's name ?? 'Default')`** (:133-137):
- Opens the native dialog (`@tauri-apps/plugin-dialog` `open({directory: true, multiple: true})`).
- Calls `addFolder` **sequentially** for each picked path.
- Backend: when **no workspaces exist yet**, it creates the named workspace as active; a duplicate folder is not added twice (settings.rs:125-141).

**`removeFolder` / `excludeRepo`** (:138-139): thin wrappers around `settingsCall`.

**`runOp(repo, label, f)`** (:142-156):
- Pushes an `Op` with `id = newId()` into `app.ops`.
- Passes `opId` and an `onProgress` callback that sets `op.line`.
- Finally removes the op and calls **`refresh(repo)`**.

**`cancelOp(id)`** → `api.cancelOp`. Errors → `say`.

**`prune(repo)`** (:160-165): `runOp('Prune', api.prune)`. Success says `<name>: pruned stale remote branches`; errors are said in red.

**`branchNames(repo)`** (:167-168): local names followed by `remote/name`. It is used outside this area.

**Sync outcomes** (`showOutcome`, :173-177): the outcome is stored per repo. A **success clears after 6s**, but only if the stored outcome is still the same object. A failure stays until the next sync.

**`syncOne(kind, repo)`** (:179-184):
- Fetch or pull a single repo; shows its outcome; says `<name>: <text>`, red when it failed.
- It does **not** set `app.sync`, so it doesn't block the bulk sync buttons.

**`syncRepos(kind, repos = all)`** (:186-204):
1. Returns immediately if a bulk sync is already running.
2. Sets `app.sync = {kind, done: 0, total}`.
3. Runs `syncAll` (4 at a time), updating `done` and the outcome as each repo finishes.
4. Says `Fetched|Pulled X of N repositories`, red if anything failed.
5. If anything failed, adds a **warn notice** titled `Fetch all: N not updated` or `Pull all: N not updated`, with one line per failure: `<name>: <text>`.

**`checkout(repo, target)`** (:206-225):
1. Says `Checking out <name>…` (for a rev target the name is the first 10 characters of the SHA).
2. On an exception: an error notice `Checkout of <name> failed`.
3. Always calls `refresh(repo)`.
4. On `switched` or `carried` with a non-rev target, the recent list becomes `[name, previousBranch, …rest]`, deduped and **capped at 6**.
5. Then a notice from `checkoutNotice`, plus the same title in the status message.

**`checkoutNotice`** (:227-250):

| Result | Notice |
| --- | --- |
| `switched` / `carried` | Level info, or warn when there is a warning. Title `Checked out <name>`, with `, local changes carried over` appended for `carried`. Body: the warning. |
| `refused` | Error. Title `Checkout of <name> refused`. Body: the error. |
| `stashKept`, still on the original branch | Error. Title `Checkout of <name> failed, your changes are kept in a stash`. |
| `stashKept`, after switching | Error. Title `Checked out <name>, your changes did not apply cleanly`. |

For `stashKept`, the body is `<error>\nStash kept: <stashMessage> (<stash sha10>)`, followed by `\nConflicts:` when there are conflicts. `repo` and `files = conflicts` are set so the notice offers the per-file Open in Zed buttons.

**`registerActions()`** (:257-282) binds these actions:
- The five palette modes: everywhere, file, action, vcs, recent.
- `nav.back` and `nav.forward`.
- Toggles: `panel.project` (`tool` ↔ files), `panel.log` and `panel.pipelines` (`bottom`), `panel.mergeRequests` (`tool` ↔ mrs).
- `app.settings` → open.
- `vcs.update` → pull the current repo; `vcs.fetch` → fetch the current repo.
- `vcs.fetchAll` and `vcs.pullAll`.
- `vcs.checkout`, `vcs.newBranch`, `vcs.deleteBranch` → set the `app.branches` intent.
- `vcs.prune` → prune the current repo.

The remaining actions are registered by other components:
- Viewer.svelte:219-225: `diff.show`, `diff.next`, `diff.prev`, `editor.gotoLine`, `markdown.preview`, `file.openInZed`, `find.inFiles`.
- Log.svelte:10: `vcs.fileHistory`.
- Pipelines.svelte:7: `pipeline.run`.
- MrChip.svelte:57: `mr.create`.

### 2.2 `src/lib/actions.ts`
- **Registry** (:11-44): 31 actions, each with `id`, `title` and `group` (`navigate`, `search`, `vcs`, `diff` or `view`). The titles are what the palette shows, for example "Update Project (Pull)", "Checkout Branch…", "Settings…", "Prune Remote Branches".
- **`register(id, run)`** (:47-55):
  - Throws on an unknown id.
  - **A later registration wins.** Its unregister restores the earlier handler, but only if it is still the active one. This is how an MR's diff viewer overrides the main viewer.
- **`runAction(id)`** returns whether a handler ran.

### 2.3 `src/lib/keymap.ts` and `docs/KEYMAP.md`

**Defaults** (:6-23):

| Action | Default chord |
| --- | --- |
| `search.everywhere` | Double Shift |
| `file.goto` | Cmd+Shift+O |
| `find.inFiles` | Cmd+Shift+F |
| `action.find` | Cmd+Shift+A |
| `file.recent` | Cmd+E |
| `editor.gotoLine` | Cmd+L |
| `nav.back` / `nav.forward` | Cmd+[ / Cmd+] |
| `panel.project` | Cmd+1 |
| `vcs.update` | Cmd+T |
| `vcs.popup` | **Ctrl+V on macOS**; **Alt+\`** on Windows and Linux |
| `diff.show` | Cmd+D |
| `diff.next` / `diff.prev` | F7 / Shift+F7 |
| `app.settings` | Cmd+, |
| `panel.log` | Cmd+9 |

All other actions are unbound by default.

**Reserved but unused** (:26-31): `panel.commit` Cmd+0, `vcs.commit` Cmd+K, `vcs.push` Cmd+Shift+K, `vcs.rollback` Cmd+Alt+Z.

**Parsing** (`parseChord`, :56-70):
- `"Double Shift"` (case-insensitive) parses as `doubleShift`.
- Otherwise the string is split on `+`. The last part is the key:
  - Letters become `Key<X>`, digits `Digit<n>`, `F1`–`F24` stay as is.
  - Punctuation maps to `BracketLeft`, `BracketRight`, `Comma`, `Period`, `Slash`, `Semicolon`, `Quote`, `Minus`, `Equal`, `Backquote`, `Backslash`.
  - Named keys: enter, esc/escape, tab, space, backspace, delete, up/down/left/right, home, end, pageup, pagedown.
- **Modifiers:** `cmd`/`command` means meta on macOS and **ctrl elsewhere**. Also `meta`, `ctrl`/`control`, `alt`/`opt`/`option`, `shift`.
- An unknown modifier or key gives null, which unbinds the action.

**Matching** (`matches`, :72-73) uses **`event.code`** with **exact** modifier equality, so chords work under any keyboard layout.

**Display** (`formatChord`, :75-80):
- macOS: symbols in the order `⌃⌥⇧⌘` followed by the key; Double Shift shows as `⇧⇧`.
- Elsewhere: `Ctrl+Alt+Shift+Meta+Key`; Double Shift shows as `Shift Shift`.

**Overrides** (`bindings`, :83-90): the defaults are merged with the overrides from `keymap.json`. An override that doesn't parse, including an empty string, **unbinds** the action. The backend reads `<config dir>/keymap.json` and returns `{}` on any error (settings.rs:224-230).

**Config directories** (KEYMAP.md:4-6):
- macOS: `~/Library/Application Support/dev.wdevs.detached`
- Windows: `%APPDATA%\dev.wdevs.detached`
- Linux: `~/.config/dev.wdevs.detached`

**Double Shift detection** (`DoubleShift`, :92-109):
- Only a keydown with `key === 'Shift'` and no meta, ctrl or alt counts.
- Any other keydown resets the detector.
- Auto-repeat is ignored.
- It fires on the second qualifying Shift within **≤300 ms** of the first (measured with `e.timeStamp`), then resets, so a third press doesn't fire again.

**Install** (:120-135):
- **One `keydown` listener on `window` in the capture phase.** It runs before Monaco and before inputs; that is how clashes with Monaco and text fields are handled.
- The bindings are checked in their insertion order. The first match calls `preventDefault`, `stopPropagation` and dispatches the action.
- Bound chords therefore **fire even while typing in inputs**.

**Titles:** `chordFor(id)` provides the chord shown in titles (strip buttons, search pill, palette action rows).

### 2.4 `src/lib/sync.ts` [ported]
- **`pool(items, limit, fn)`** (:6-21): runs up to `limit` workers; results come back in input order; a rejection doesn't stop the others.
- **`describe(result)`** (:25-33):

  | Result | Text |
  | --- | --- |
  | void (a fetch) | Fetched |
  | `upToDate` | Up to date |
  | `fastForwarded` | `Fast-forwarded <from7>..<to7>` |
  | `diverged` | `Diverged (↑a ↓b), not updated` |
  | `noUpstream` | No upstream |

- **`syncAll`** (:36-53): 4 at a time. `ok` is true unless the result is `diverged`; **No upstream counts as ok**. A thrown error gives `{ok: false, text: String(e)}`. `onDone` runs once per repo.

### 2.5 `src/lib/nav.ts` (used by the palette and back/forward)
- **Recent files:** up to 50, deduped by repo + ref + path, newest first. MR and job "docs" are not recorded as recent.
- **History:** up to 100 entries. A new visit drops the forward entries.
- **Dedupe:** a visit equal to the current entry (same file, line and doc) is not pushed again.
- **`back` / `forward`** replay through the opener without recording (:13-57).

### 2.6 `src/lib/view.svelte.ts` (the parts this area touches)
- `view.shown` decides whether `main` shows the Viewer, an MR page or a job log.
- `compare`: a `SvelteMap` of repo → base branch, where `''` means the default branch. The repo menu's "Compare with Base Branch" writes it.
- `refs` / `refOf`: the ref a repo's file tree browses. Go to File uses it for a single repo.

### 2.7 `src/lib/platform.ts`
- `mac` is `userAgent.includes('Mac')`.
- `fileManager` is "Finder" on macOS, "Explorer" on Windows, otherwise "File Manager".
- `primary(e)` is metaKey on macOS and ctrlKey elsewhere. It is not used in this area.

### 2.8 `src/lib/storage.ts`
`load` and `save` wrap localStorage in try/catch. A failure means null on load and nothing on save.

---

## 3. Persisted UI state

### localStorage

| Key | Owner | Meaning | Default |
| --- | --- | --- | --- |
| `sidebar.collapsed` | Sidebar.svelte:18-21 | JSON array of collapsed keys: `workspace:<name>`, folder paths, repo paths (worktrees hidden) | `[]`, everything expanded |
| `size.repos` | App.svelte:55 via Grip | Sidebar width, clamped 200–600 | 280 |
| `size.tool` | App.svelte:61 | Tool pane width, clamped 220–800 | 300 |
| `size.bottom` | BottomPanel.svelte:13 | Bottom panel height, min 120, max window height − 240 | (other area) |
| `size.log.side` | Log.svelte:234 | 260–1000 | (other area) |
| `size.mr.side` | MrPage.svelte:244 | 260–1000 | (other area) |
| `size.mr.files` | MrPage.svelte:251 | 180–700 | (other area) |
| `tree.nesting` | FileTree.svelte:31,36 | Nest related files; `'false'` turns it off | true (other area) |
| `viewer.sideBySide` | Viewer.svelte:32,187 | | true (other area) |
| `viewer.mdPreview` | Viewer.svelte:127,182 | | true (other area) |

### Not persisted in the frontend
The sidebar shown/hidden state, the open tool, the bottom panel, the current repo (it falls back to the first repo on every launch), `paletteAll`, recent branches, nav history and recent files, and the compare map.

### Backend-persisted
- **`settings.json`** in the config dir, written atomically through a `.tmp` file. Fields:
  - `workspaces[{name, folders, excludedRepos, active}]`
  - `repoAccounts`, `forgeAccounts`, `mrRemotes`, `gitPath`, `fileNesting`
- An invalid file is moved to `settings.json.bad`.
- Old pre-workspace settings (`folders` / `excludedRepos`) migrate into an active "Default" workspace (settings.rs:50-74).
- **`keymap.json`** holds the keymap overrides; it is read only.

---

## 4. Backend commands used by this area (`src/lib/api.ts`)

| api name | Tauri command | Called from |
| --- | --- | --- |
| `getSettings` | `get_settings` | `loadRepos`, Settings `reload` |
| `listRepos` | `list_repos` | `refreshAll` (startup, window focus, Refresh button, after every `settingsCall`) |
| `addFolder(ws, path)` | `add_folder` | `addFolders` |
| `removeFolder` | `remove_folder` | Sidebar `−`, repo × on a solo repo, folder and repo menus |
| `excludeRepo` | `exclude_repo` | repo ×, repo menu "Remove from List" |
| `includeRepo` | `include_repo` | Settings "Restore" |
| `setWorkspaceActive` | `set_workspace_active` | Workspaces popup toggle, workspace menu "Turn Off" |
| `saveWorkspace(name, from)` | `save_workspace` | create or rename a workspace |
| `removeWorkspace` | `remove_workspace` | Workspaces popup delete |
| `setRepoAccount`, `setForgeAccount`, `setMrRemote` | `set_*` | Settings selects |
| `setGitPath`, `envInfo` | | Settings Git section |
| `keymapOverrides` | `keymap_overrides` | App mount |
| `openFolder` | `open_folder` | repo and folder menus |
| `openInZed(repo, path, line)` | `open_in_zed` | repo menu (with `''`), Notices conflict files |
| `repoStatus` | `repo_status` | `refresh` |
| `listBranches` | `list_branches` | Branches popup, Palette everywhere mode |
| `checkout(repo, target)` | `checkout` | `checkout()` |
| `createBranch(repo, name, start, false)` | `create_branch` | Branches New |
| `setUpstream(repo, branch, upstream \| null)` | `set_upstream` | Branches track and untrack |
| `deleteBranch(repo, name, force)` | `delete_branch` | Branches delete; may return `notMerged {commits}` |
| `deleteRemoteBranch(…, opId, progress)` | `delete_remote_branch` | streams, cancellable |
| `prune(repo, opId, progress)` | `prune` | streams, cancellable |
| `fetch(repo, opId, progress)` | `fetch` | streams, cancellable, returns void |
| `pull(repo, opId, progress)` | `pull` | streams, cancellable, returns `PullResult` |
| `cancelOp(opId)` | `cancel_op` | StatusBar × |
| `listAccounts`, `saveAccount`, `removeAccount`, `verifyAccount`, `repoAccounts`, `forgeHosts` | | Settings and Accounts |
| `quickOpen(repos, rev, query, limit)` | `quick_open` | Palette |
| native dialog `open` | `plugin:dialog\|open` | `addFolders` |
| native menu | `plugin:menu\|*` | menu.ts |

**Streaming and progress** (api.ts:252-299): every cancellable op passes a Tauri `Channel` named `progress` that carries text lines, plus an `opId` from `newId()`, a module-level counter starting at 1. The frontend shows only the **latest** line in the StatusBar chip. Cancelling calls `cancel_op(opId)`, and the promise rejects with the backend's error.

**Refresh triggers:**
- Window `focus` → `refreshAll` (App.svelte:40).
- The Refresh button.
- Startup.
- After every settings mutation.
- After each `runOp` (that repo only).
- After `checkout`.
- After each Branches `act`.

**Timers:** there is **no polling** in this area. The only timers are:
- the 6s info-notice auto-dismiss,
- the 6s fade of successful sync outcomes.

**Concurrency:**
- Status refresh: 8 at a time.
- Bulk fetch or pull: 4 at a time.
- Bulk sync is single-flight (`app.sync` guard).
- `refreshAll` coalesces concurrent calls.

---

## 5. Visual design tokens (`src/app.css`)

### Fonts (app.css:1-2, :34-35, :81)
- `--font`: `'Manrope Variable', -apple-system, system-ui, sans-serif`, from `@fontsource-variable/manrope/wght.css`.
- `--mono`: `'JetBrains Mono', ui-monospace, 'SF Mono', Menlo, monospace`, from `@fontsource/jetbrains-mono/400.css`.
- Body text: `font: 500 13px/1.4 var(--font)`, `-webkit-font-smoothing: antialiased`.
- Body: `user-select: none`, `cursor: default`, `overflow: hidden`, `background: var(--ground)`, `color: var(--fg)`.
- Inputs and textareas can be selected.

### Custom properties

| Token | Light (`:root`, :4-37) | Dark (`prefers-color-scheme: dark`, :39-72) |
| --- | --- | --- |
| `--ground` | `#eceff5` | `#090c15` |
| `--panel` | `rgb(255 255 255 / 0.72)` | `rgb(255 255 255 / 0.045)` |
| `--panel-line` | `rgb(255 255 255 / 0.9)` | `rgb(255 255 255 / 0.09)` |
| `--panel-shadow` | `0 1px 0 #fff inset, 0 10px 30px rgb(30 41 80 / 0.08), 0 0 0 1px rgb(30 41 80 / 0.05)` | `0 1px 0 rgb(255 255 255 / 0.07) inset, 0 12px 40px rgb(0 0 0 / 0.45)` |
| `--code` | `#ffffff` | `#0e1220` |
| `--fg` | `#0f172a` | `#e8ecf6` |
| `--muted` | `#5b6478` | `#a5b0c8` |
| `--line` | `rgb(30 41 80 / 0.09)` | `rgb(255 255 255 / 0.08)` |
| `--accent` | `#6366f1` | `#818cf8` |
| `--accent-fg` | `#ffffff` | `#0b1020` |
| `--primary` | `#0f172a` | `#ffffff` |
| `--primary-fg` | `#ffffff` | `#0b1020` |
| `--hover` | `rgb(30 41 80 / 0.06)` | `rgb(255 255 255 / 0.07)` |
| `--selected` | `rgb(99 102 241 / 0.13)` | `rgb(129 140 248 / 0.24)` |
| `--selected-ring` | `rgb(99 102 241 / 0.35)` | `rgb(129 140 248 / 0.5)` |
| `--selected-inactive` | `rgb(30 41 80 / 0.06)` | `rgb(255 255 255 / 0.08)` |
| `--hit` | `#b45309` | `#fcd34d` |
| `--popup` | `rgb(255 255 255 / 0.92)` | `rgb(22 27 44 / 0.9)` |
| `--shadow` | `0 1px 0 #fff inset, 0 24px 60px rgb(30 41 80 / 0.18), 0 0 0 1px rgb(30 41 80 / 0.06)` | `0 1px 0 rgb(255 255 255 / 0.08) inset, 0 30px 80px rgb(0 0 0 / 0.6)` |
| `--scrim` | `rgb(15 23 42 / 0.12)` | `rgb(0 0 0 / 0.35)` |
| `--input` | `#ffffff` | `rgb(255 255 255 / 0.07)` |
| `--error` | `#e11d48` | `#fb7185` |
| `--warning` | `#b45309` | `#fbbf24` |
| `--success` | `#059669` | `#34d399` |
| `--added` | `#059669` | `#34d399` |
| `--modified` | `#4f46e5` | `#a5b4fc` |
| `--deleted` | `#64748b` | `#94a3b8` |
| `--conflict` | `#e11d48` | `#fb7185` |
| `--untracked` | `#b45309` | `#fbbf24` |
| `color-scheme` | `light` | `dark` |

The theme follows the OS (`prefers-color-scheme`). There is no manual toggle.

### Shared classes
- **`.panel`** (:105), the opaque-ish panel look: `background: var(--panel)`, `1px solid var(--panel-line)`, `box-shadow: var(--panel-shadow)`, radius 14. Used by the top bar, the strip, the sidebar and the tool pane.
- **`.glass`** (:107-114), for popups, the palette, Settings and notices: `background: var(--popup)`, `backdrop-filter: blur(24px) saturate(150%)` (with the `-webkit-` prefix too), `1px solid var(--panel-line)`, `box-shadow: var(--shadow)`, radius 16. Components override the radius: palette 18, Settings 20.
- **`.scrim`**: fixed, `inset: 0`, `background: var(--scrim)`. Used by the palette and Settings; the anchored popups use a transparent backdrop instead.
- **Form controls** (:93-103): `background: var(--input)`, `1px var(--line)` border, radius 9, `padding: 4px 9px`, no outline. Textareas use `padding: 6px 9px` and resize vertically. Focus: `border-color: var(--selected-ring)` plus `box-shadow: 0 0 0 3px var(--selected)`.
- **`.icon-btn`** (:118-131): 28px high, min 28 wide, `padding: 0 6px`, radius 8, no background, muted. Hover: `var(--hover)` background and `var(--fg)` text. Disabled: opacity 0.5.
- **`.btn`** (:133-143): 28px high, `padding: 0 14px`, a pill (radius 999), `1px var(--line)` border, `var(--input)` background, weight 700.
  - `.primary`: `--primary` background, border and fill, with `--primary-fg` text.
  - `.danger`: `--error` text.
  - Disabled: opacity 0.5.
- **`.seg`** (segmented control, :145-147): `padding: 2px`, radius 9, `var(--hover)` background. Buttons are 24px high, radius 7, 12px weight 600, muted. `.on` uses `var(--selected)`, fg color and weight 700.
- **`.st`** (status letter, :150-155): 18×18, radius 5, mono 700 10px, colored by `--c`, background `color-mix(var(--c) 18%)`. Class → color: `modified`/`renamed` → `--modified`, `added` → `--added`, `deleted` → `--deleted`, `untracked` → `--untracked`, `conflict` → `--conflict`.
- **`.dot`** (:157): 7px circle in `currentColor` with a glow `0 0 8px currentColor`.
- **Utilities:** `.muted`, `.mono` (12px), `.hit` (`--hit`, weight 700), `.ellipsis`.

### Radii
- 999: pills (`.btn`, search pill)
- 20: Settings dialog
- 18: palette
- 16: glass
- 14: panels and `main`
- 12: palette query input, account rows
- 10: strip buttons, top bar widgets, sidebar repo rows, not-merged commit box
- 9: inputs, list items, workspace items
- 8: `.icon-btn`
- 7: seg buttons
- 6: kbd
- 5: `.st`
- 4: badges, grips

### Spacing conventions
- App: 10px outer padding and 10px gap between all panes.
- Top bar: 46px high. Strip: 48px wide. Sidebar header: 44px. Sidebar rows: 40px.
- List item heights: 32 (switcher, workspaces), 28 (branches), 30 (palette).
- Anchored popups: `margin-top: 8px`, `padding: 8px 0`, items inset `margin: 0 8px`.

### Typography
- Section labels: 11px, weight 700, uppercase, 0.06em letter-spacing, muted.
- Titles: 14–15px, weight 800.

### Z-order
| Layer | z-index |
| --- | --- |
| Grip | 3 |
| Anchored popup backdrop / popup | 10 / 11 |
| Palette scrim / palette | 20 / 21 |
| Settings scrim / dialog | 30 / 31 |
| Notices | 30 |

---

## 6. Test coverage

### E2E (Playwright, `e2e/*.spec.ts`)
In e2e, the native folder dialog is mocked to return `__E2E__.pick`, by default `[ws, solo]`. The native menu calls are stubbed and do nothing (`e2e/shim.ts`), so **no context-menu item is exercised**. The fixture's `open()` (fixture.ts:163-167) clicks the empty-state "Add folder…" button, which covers the empty state, `addFolders` and the "Default" workspace being created.

| Spec / test | Behaviors covered |
| --- | --- |
| `frame.spec.ts` › "the forge repos track the seeded fake; Cmd+9 and the strip open the bottom panel" | Cmd+9 toggles the Log bottom panel (open, then on the Log tab, then closed); BottomPanel tab switch to Pipelines; the strip's Pipelines and Git Log buttons get `.on` and toggle the panel; the strip button titles start with "Git Log" |
| `sidebar.spec.ts` › "adding a folder lists nested repos, worktrees under their repo and branch state" | Path-sorted names (`app, app-wt, group/api, group/web, lib, solo`); a solo folder has no header; folder header text `ws`; worktree row gets `.worktree` and is indented further; line 2 formats `main ↓1`, `main ↑1 ↓1`, `feature`; the "Hide 1 worktree" toggle; clicking a folder header collapses it |
| sidebar › "a worktree deleted on disk drops out when the window gets focus" | Window focus → `refreshAll` rescans the disk; no `.err` rows remain |
| sidebar › "double-clicking empty sidebar space selects no text" | Global `user-select: none` |
| sidebar › "right-clicking the sidebar suppresses the web view menu" | main.ts contextmenu `preventDefault` |
| sidebar › "the repository switcher starts on the current repo; type, Enter selects" | Initial selection is the current repo; the folder label is shown for nested repos and hidden for solo; Enter chooses; typing filters; the dialog closes; the sidebar row becomes `.selected` |
| `sync.spec.ts` › "fetch all and pull all report per repo; the diverged repo is left as it was" | Fetch All and Pull All buttons; outcome texts Fetched / Fast-forwarded / Diverged… / Up to date / No upstream; warn notice "Pull all: 1 not updated"; the outcome shares line 2 with the branch; a success outcome fades within 10s while a failure stays; the diverged repo's HEAD is unchanged |
| `workspaces.spec.ts` › "workspaces: several on at once, each with its folders; rename, turn off, delete" | Create a workspace with the "New workspace" input + Enter; workspace headers appear when more than one is active; Escape closes the popup; hover "Add folder to Home"; the same repo is listed twice; toggling a workspace off; the widget label; rename via hover button, "Workspace name" input and Enter; all off → "No workspace is on" and the widget shows "No workspace"; delete |
| workspaces › "related files nest under their main file; the switch flattens the tree" | Cmd+1 opens the Project tool (the FileTree itself is another area) |
| `palette.spec.ts` › "go to file: filters, moves by keyboard under a resting mouse, opens, ignores case" | Cmd+Shift+O shows the "Go to File" mode title; results; ↑ / ↓ selection not stolen by a resting mouse (the movement check); Enter opens the file in the Viewer and closes the palette; case-insensitive matching |
| palette › "keymap opens each palette and settings" | Double Shift → Search Everywhere; Cmd+Shift+A → Find Action; Ctrl+V → VCS Operations; Cmd+E → Recent Files; Escape closes each; Cmd+, opens the Settings dialog |
| `branches.spec.ts` › "checkout over conflicting local changes keeps the stash and names it" | Branch widget click, search, Enter checks out; stashKept error notice title, body text "Stash kept: …", conflict file list |
| branches › "deleting an unmerged branch lists its commits and asks first" | VCS palette → "Delete Branch" action → delete intent (placeholder "Delete branch…"); the not-merged view with its commits; "Delete anyway" forces the delete |
| branches › "a new branch named -D is refused and its start branch survives" | ArrowRight opens the branch menu; `New Branch from 'other'…`; "Branch name" input; Create; error notice text `invalid name: "-D"` |
| branches › "Enter right after pasting a query checks out the first match, even before branches load" | The `enterPending` behavior |

### Unit tests (vitest)
- **`src/lib/keymap.test.ts`:** parseChord (modifiers and codes, invalid chords, macOS symbols, Cmd → Ctrl on Windows/Linux); `matches` (event.code across layouts, exact modifiers); `bindings` (every default maps to a registered action, M2 chords are free, overrides, empty or invalid override unbinds); DoubleShift (within 300 ms, too slow, interrupted by another key or a modifier chord, auto-repeat ignored, a fresh pair needed after firing); `install` (dispatches chords including actions another component owns; uses overrides).
- **`src/lib/sync.test.ts`:** pool limit and order, empty list; syncAll reports every repo and one failure doesn't stop the rest; a fetch is reported as "Fetched".
- **`src/lib/nav.test.ts`:** recent list (50, deduped), history cap of 100, back/forward with forward entries dropped, no duplicate push but a new line counts, the open hook.
- **`src/lib/actions.test.ts`:** register and unregister; unknown id rejected.

### Behaviors with no test
**App shell**
- Strip: the Repositories toggle (hiding the sidebar), the Changes and Merge Requests tool toggles, the Settings button, and mutual exclusivity of tools.
- Grip resizing and `size.*` persistence.
- `sidebar.collapsed` persistence across reloads.
- The Viewer staying mounted under the MR page.

**Top bar and status bar**
- The search pill click.
- The "No repository" and "—" labels, and the disabled states of the widgets and sync buttons.
- The `done/total` progress text during a bulk sync.
- The macOS 90px padding and the drag regions.
- Status message click-to-dismiss and its red error style.
- The op chip: spinner, progress line, Cancel (`cancel_op`).

**Notices**
- Info auto-dismiss after 6s and the Close button.
- "Open in Zed" in notices.

**Context menus** (none are exercised): every repo, folder and workspace menu item, including Compare with Base Branch, Copy Path with its status message, Open in Finder/Zed, Fetch/Pull All in Folder and its disabled state, and Turn Off.

**Sidebar**
- The hover × "Remove from list" (exclude vs. solo remove-folder) and the hover "Remove folder".
- The in-progress badge, the dirty count, tone dot colors, a status error row, and the tooltip with the last commit subject.
- Collapsing a workspace header.
- The Refresh button and the header "Add folder" button.
- Selecting `current` on right-click.

**RepoSwitcher**
- ↑ / ↓ clamping, Escape, backdrop click, the "No repositories match." empty state.

**Workspaces popup**
- Toggling with Space or Enter, backdrop close, Escape while renaming (cancel), blur committing a rename, duplicate or empty name errors, the folder-list tooltip.

**Branches popup**
- Checking out a remote branch.
- The Recent section and its 6-entry cap.
- Choosing the current branch opens its menu.
- Prune (row and action).
- Set and Unset Tracking Branch.
- Delete on Remote with its confirmation and runOp.
- The `gone` badge and the ↑/↓ display.
- Escape and ArrowLeft navigation between sub-views.
- The "Checkout branch" checkbox unticked.
- The New-intent popup opening directly in the form (from the repo menu or `vcs.newBranch`).
- Load error and "Loading…" states; "No branches match."

**Palette**
- Search Everywhere results mixing files, branches and actions, and choosing a branch (checkout).
- The "All repositories" checkbox and re-pressing Cmd+Shift+O toggling it.
- File mode showing recent files when the query is empty.
- Arrow wrap-around.
- Muted unavailable actions, and the "<title> is not available yet" message.
- Ref-scoped quickOpen (` @ ref` detail).
- Empty-state texts.

**Settings and Accounts**
- Every Settings section's content and selects (repo account, MR remote, forge account), Restore of a removed repo, Git path Apply and the env line, Close and backdrop close.
- The whole Accounts UI: add, edit, two-step remove, Check token, provider base URL locking, token placeholder when editing, auto-check after save.

**Keymap and actions**
- Chords not covered by e2e: Cmd+T (`vcs.update`), Cmd+[ / Cmd+], and the Alt+` VCS popup on non-macOS.
- `keymap.json` overrides end to end.
- Actions `vcs.fetch`, `vcs.fetchAll`, `vcs.pullAll`, `vcs.prune`, `vcs.checkout` and `vcs.newBranch` run from the palette.
- Checkout notice variants `switched`, `carried`, `refused`, warning, and stashKept still on the original branch.

**Other**
- The first folder creating the "Default" workspace is covered only indirectly: the Default header appears in the workspaces spec.
- main.ts `focusin` setting autocorrect and spellcheck off.
- `refreshAll` coalescing.
- The selected repo falling back to the first repo when it disappears.
