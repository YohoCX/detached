<!-- Generated from the Tauri client on 2026-11-04 by read-only survey agents; file:line refers to that code. -->
# Parity inventory: files, viewer, diff, search (Detached, Svelte 5 + Monaco 0.57.0)

All paths are relative to the repo root. Everything below describes what the code does today.

---

## 0. Wiring and layout (context) [ported]

- `src/App.svelte:59-65`: the tool panel shows one of `FileTree` (`app.tool==='files'`), `MrList` or `Changes` (the default, `app.tool` defaults to `'changes'`, `src/lib/app.svelte.ts:30`). It always gets `repo={app.current}`. The left strip buttons toggle the tool (`App.svelte:47-49`). `Cmd+1` toggles Project (`app.svelte.ts:267`).
- `App.svelte:68-69`: **the Viewer stays mounted** (it is only `display:none`) while an MR page or job log is shown, so its Monaco editors and models survive. A second Viewer instance lives inside the MR page (`src/components/MrPage.svelte:267`). It has a `doc`, is forced side-by-side, has view zones and `ongutter`.
- `App.svelte:79,81`: `Palette` is mounted while `app.palette` is set. `FindInFiles` is mounted while `view.find` is set. Both unmount on close.
- `App.svelte:40`: window focus runs `refreshAll()`, which calls `repo_status` for every repo. Many views below reload on that status refresh.
- `src/main.ts:13-16`: the web view's context menu is suppressed everywhere except `input`, `textarea` and `.monaco-editor`, so Monaco's own context menu stays. `main.ts:5-11`: all inputs get autocorrect, autocapitalize, autocomplete and spellcheck turned off.
- Keymap (`src/lib/keymap.ts:6-23`, `docs/KEYMAP.md`):

  | Action | Chord |
  |---|---|
  | `search.everywhere` | Double Shift (two lone Shift keydowns within 300 ms, `keymap.ts:93-109`) |
  | `file.goto` | Cmd+Shift+O |
  | `find.inFiles` | Cmd+Shift+F |
  | `file.recent` | Cmd+E |
  | `editor.gotoLine` | Cmd+L |
  | `nav.back` / `nav.forward` | Cmd+[ / Cmd+] |
  | `panel.project` | Cmd+1 |
  | `diff.show` | Cmd+D |
  | `diff.next` / `diff.prev` | F7 / Shift+F7 |
  | `vcs.popup` | Ctrl+V on mac, Alt+` elsewhere |
  | `markdown.preview`, `file.openInZed`, `vcs.fileHistory` | no default chord |

  Chords match on `event.code` and are caught by one capture-phase listener on window that calls preventDefault and stopPropagation (`keymap.ts:120-135`). Overrides come from `keymap.json` through `keymap_overrides`. Chords shown in tooltips and hints use `chordFor()`.
- Action registry (`src/lib/actions.ts:47-55`): the latest `register()` wins and the previous handler comes back on unregister. This is how the MR Viewer's F7/Shift+F7/Cmd+L take over from the main Viewer while it is mounted.

---

## 1. User-visible behavior per component

### 1.1 Viewer (`src/components/Viewer.svelte`) [ported]

**What it shows.** `shown = doc ?? view.shown`, but only when it is of kind `file` or `diff` (`:47`). `FileDoc` is either `{kind:'file', repo, ref, path, line}` or `{kind:'diff', repo, path, origPath, left: Side, right: Side}` (`src/lib/view.svelte.ts:6-8`).

**Modes** (`:29`): `empty | editor | diff | placeholder`.

- **Empty** (nothing shown): three centered hint lines: "Go to file ⇧⌘O", "Search everywhere ⇧⇧", "Find in files ⇧⌘F" (`:353-358`). There is no header.
- **Editor** (a file, or a conflicted diff): a read-only Monaco editor.
- **Diff**: Monaco DiffEditor.
- **Placeholder**: centered muted text (`:351-352`).
  - For a file, `describe(c)` (`src/lib/files.ts:166-174`) gives one of:
    - "Binary file"
    - "File too large to show (X.X MB)" (MiB, one decimal)
    - "No file here" (missing, or a directory)
    - "Submodule at <first 10 chars of sha>"
  - For a diff where either side is not text, or both are missing: `"<SideName>: <describe(left)>. <SideName>: <describe(right)>."` (`:138-139`).
  - Any thrown error shows `String(e)` (`:141-142`).
- **Loading**: there is no loading indicator. The previous content stays until the new content arrives. A `seq` counter drops stale responses (`:116,120,133`).
- **Image files**: there is no image viewer. PNG and similar files hit the NUL check and show "Binary file". SVG shows as XML text.
- **LFS**: there is no LFS-specific UI. The backend smudges through `cat-file --filters` with LFS credentials (see §6), so LFS files show their real content (or binary/too large).
- **Symlinks** in the working tree show the link target path as text (`src-tauri/src/git/diff.rs:332-337`).
- **Conflicted**: if a file read returns `conflicted`, or a diff's right side is `conflicted`, the plain editor shows the working-tree text with conflict markers. A bar reads "This file has merge conflicts. [Open in Zed]" (`:136-137, 341-343`). Markdown preview is disabled while conflicted (`:49`).

**Header** (only when something is shown, `:312-340`):
- Bold basename, then the muted directory, then the muted repo name plus `@ <ref>` when browsing a file at a ref.
- For diffs, a pill `"<left> ↔ <right>"`. `sideName` (`view.svelte.ts:42-49`) gives HEAD / Staged / Working tree / a rev (40-hex shas cut to 10 chars).
- In diff mode:
  - "Previous change ⇧F7" and "Next change F7" icon buttons.
  - A segmented "Side-by-side | Unified" control, hidden when `sideBySide` is forced (MR viewer).
- For markdown files (editor mode, not conflicted): a segmented "Source | Preview" control (`:332-337`).
- An "Open in Zed" icon button, always present.

**Load and reload** (`:300-308`): the effect re-runs when `shown` changes (a fresh show) and whenever `statuses.get(repo)` changes (a status refresh after operations or window focus). On a reload (not fresh), the content is refetched and updated in place (see model caching, §2). There is no reveal on reload.

**Fresh file open** (`:118-130`):
- `api.readFile(repo, ref, path)`.
- Sets `markdown = text`.
- Markdown preview turns on if `!s.line && localStorage viewer.mdPreview !== 'false'`. A file opened at a line always shows the source.
- `revealLine(editor, line ?? 1)`: sets the cursor at (line, col 1), runs `revealLineInCenter` and **focuses the editor**. Focus therefore moves into the editor whenever a file is opened from the tree, palette, find or a link.

**Fresh diff open** (`:132-140, 78-100`):
- `api.fileDiff(repo, path, origPath, left, right)`.
- If fresh, it waits for `onDidUpdateDiff` once, then calls `revealFirstDiff()`. This puts the cursor on the first change; e2e checks for line 5.
- If `focusPending` is set (`view.focusDiff` was bumped, `:250-252`), the modified editor is focused after the diff model is set.

**Go to line** (`editor.gotoLine`, Cmd+L, `:146-160, 360-369`):
- Only in editor or diff mode. It turns markdown preview off (not persisted).
- An overlay input appears at the top center (placeholder "Line[:column]", 220px wide) and is focused.
- Enter parses `line[:col]`. A valid line reveals and centers in the active editor (the modified side in a diff) at that column (default 1). For files, the location goes into nav history with the line (`nav.visit`). An invalid line just refocuses the editor.
- Escape closes and refocuses the editor. Blur closes.

**Next/previous change** (F7 / Shift+F7, or the header buttons): `diff.goToDiff('next'|'previous')`, only in diff mode (`:219-220, 323-324`).

**Side-by-side / Unified toggle**: persisted (`:185-188, 254`). It is forced on when the `sideBySide` prop is set.

**Show Diff** (Cmd+D, `diffAction`, `:170-178`, main Viewer only):
- If `view.change` is set (the selected row in Changes) and either focus is inside `[data-changes]` or the shown item is not a file: `showDiff(change.repo, change.path, change.origPath)`. The left side is HEAD, even when Changes is in Compare mode.
- Else, if a file at a ref is shown: diff rev(ref) ↔ Working tree.
- Else, if a working-tree file is shown: HEAD ↔ Working tree, with `origPath` taken from the repo status (renames).

**Open in Zed** (`file.openInZed`, `:162-167`): `api.openInZed(repo, path, cursorLine | null)`. The line comes from the active editor's cursor (the modified side in a diff). Errors go to the status message (`say(...,true)`). Other callers: the repo context menu "Open in Zed" (repo root, path `''`, `src/lib/menu.ts:41`) and conflict notices per file (`src/components/Notices.svelte:6,21`).

**Find in Files** action (`find.inFiles`) is registered by the main Viewer (`:225`) and sets `view.find = true`.

**MR-mode props** (used by MrPage):
- `doc` replaces `view.shown`. With `doc`, Viewer does not register diff.show, openInZed or find.inFiles (`:223-225`).
- `zones` are view zones (§2).
- `ongutter` with `commentable`: a hovered line shows a "+" glyph where `commentable(side,line)` allows. Clicking the glyph margin or the line number calls `ongutter(side, line, otherLine(...))`. `otherLine` (`src/lib/mr.ts:28-44`) maps an unchanged line to its counterpart on the other side, or null inside a change.

**Keyboard clash guard** (`:345`): keydown in the capture phase on `.editors` stops propagation of the chords in `clashesWithMonaco` so Monaco never sees them (§2).

**Theme**: follows `prefers-color-scheme` live (`:191, 211-212`).

### 1.2 FileTree (`src/components/FileTree.svelte`) [ported]

**Header** (`:103-113`):
- Title "Project".
- Ref input (140px, placeholder "Working tree", title "Browse a branch, tag or commit"). A `<datalist>` holds branch names, loaded on focus through `branchNames()`: local names plus `remote/name` (`src/lib/app.svelte.ts:167-168`).
- Enter or `change` commits the trimmed value into `refs.set(repo, v)`. Empty means `refs.delete` (`:68-72`).
- When a ref is set, the input gets a highlighted border (`.at`) and a "Back to the working tree" × button clears it.
- "Nest related files" toggle (aria-pressed, accent when on) is persisted.

**States**:
- "No repository" (`!repo`).
- An error text (red, pre-wrap) when `list_files` fails, e.g. a bad ref.
- Otherwise the virtual list.
- There is no loading or empty state text. Old entries stay until the new list arrives. Stale responses are dropped by `seq` (`:43-62`).

**Data**:
- `api.listFiles(repo, ref)`.
- For the working tree it reloads on every status refresh (`:48`). At a ref it does not.
- The working-tree listing walks the disk honoring gitignore, so deleted-but-tracked files do not appear.
- Files under an untracked nested repo are marked untracked (`src-tauri/src/git/files.rs:60-81`).

**Tree building** (`files.ts:83-125`):
- Directories come first, then files. Both sort with `localeCompare(..., {sensitivity:'base', numeric:true})`.
- Rows have depth indentation `8 + depth*16` px, a 24px row height, and a `title` of the full path.
- A directory is "dirty" if any descendant has a change. Status color comes from the file's change (`statusOf` → conflict, untracked, added, deleted, renamed, modified) or `modified` if dirty (directories, and nesting parents with dirty kids) (`FileTree.svelte:124`, `files.ts:6-13`).

**File nesting** (`files.ts:45-81, 110-121`):
- Rules map a main suffix to kid suffixes. Defaults (`nestingDefaults`):
  - `.ts` → `.spec.ts`, `.test.ts`
  - `.tsx` → `.spec.tsx`, `.test.tsx`, `.stories.tsx`, `.module.css`, `.module.scss`
  - `.js` → `.spec.js`, `.test.js`, `.d.ts`, `.js.map`, `.min.js`
  - `.jsx` → `.spec.jsx`, `.test.jsx`, `.module.css`
  - `.css` → `.css.map`, `.min.css`
  - `.scss` → `.css`, `.css.map`
  - `.go` → `_test.go`
  - `package.json` → `package-lock.json`, `bun.lock`, `bun.lockb`, `pnpm-lock.yaml`, `yarn.lock`
  - `Cargo.toml` → `Cargo.lock`
  - `go.mod` → `go.sum`
  - `pyproject.toml` → `poetry.lock`, `uv.lock`
  - `Gemfile` → `Gemfile.lock`
  - `composer.json` → `composer.lock`
- Merged with `app.fileNesting` from settings (`Settings.fileNesting`, `src-tauri/src/settings.rs:24-25`). There is no UI for editing these. Merge: `{...nestingDefaults, ...app.fileNesting}` (`FileTree.svelte:32`).
- Matching is per directory: `stem + kidSuffix` must exist. The first parent claim wins.
- Nesting is one level only: chains collapse to the top parent, so `a.js.map` under `a.js` under `a.ts` all go under `a.ts`.
- A nesting parent shows a chevron. Clicking the chevron toggles (stopPropagation). Clicking the row opens the file.
- Kids render at depth+1 with `parent` set.
- With nesting off, rules are `{}`, a flat list.

**Clicks** (`:76-80`): selects the row (`sel`). A file opens `open({repo, ref, path, line:null})`, which records nav history and moves focus to the editor. A directory toggles its expansion.

**Keyboard** (on the `role=tree` container, tabindex 0, `:82-99`):
- Up/Down: move the selection (clamped, no wrap) and `reveal`.
- Right: expands a directory or nesting parent.
- Left: collapses if expanded, else jumps to the parent (nesting parent or `dirName`).
- Enter: activates.
- All of these call preventDefault. There is no Home, End, PageUp/PageDown or type-ahead.

**Selection styling**: unfocused `--selected-inactive`. With `:focus-within`, `--selected` plus a ring.

**Expansion persistence** (`:1-5, 26-30`):
- A module-level `Map<repoPath, SvelteSet<path>>` keeps expansion per repo for the session (in memory only). It survives the component unmounting when switching tools.
- The same set is shared between the working tree and every ref.
- `sel` is component state and is lost on unmount.

**Context menu**: none (the web view menu is suppressed).

### 1.3 Changes (`src/components/Changes.svelte`) [ported]

**Header** (`:85-95`):
- Title "Changes".
- Segmented "Local | Compare". Compare has the title "Changes since this branch left a base branch, like a merge request".
- A "Refresh" icon button that calls `refresh(repo)` (`repo_status`).

**Local mode** (`compare` has no entry for the repo):
- Groups come from `groupChanges(status.changes)` (`files.ts:17-24`): "Conflicts", then "Changes" (not conflicted, not untracked), then "Unversioned Files". Each is sorted by path and empty groups are dropped.
- Staged and unstaged are not distinguished. A row diff is always HEAD ↔ Working tree.

**Compare mode** (`compare.set(repo, base)`, where `''` means the default branch):
- A row under the header: "with" plus an input with a branch datalist. Placeholder is `info.base` or "default branch"; value is the base. On `change` it sets the trimmed value.
- `↑ahead ↓behind` with the title "Commits on this branch / on <base> since they split".
- Data (`:23-34`): `compareBase(repo, base||null)` then `listChanges(repo, rev(mergeBase), worktree)`, sorted by path. This includes uncommitted working-tree changes.
- It reloads on status refresh or a base change. Stale results are dropped.
- There is one group: "Changes since <base>".
- Compare can also be entered from the repo context menu "Compare with Base Branch" (`menu.ts:32-36`, which also selects the repo and opens Changes).
- `compare` is a session-only SvelteMap (`view.svelte.ts:23`).

**States** (`:105-112`):
- "No repository".
- Status error text (red).
- Compare error text, e.g. "no default branch found; type one to compare with".
- Empty: "No local changes" (also shown before status loads, since there is no loading state), "No changes since <base>", or "Comparing…" (compare in flight).

**Rows** (28px, virtual list):
- A status letter badge (`letter`: M/R/A/D/U/C, `files.ts:15`), the bold basename and the muted directory.
- Deleted rows have a strikethrough name. Conflicts are colored red.
- Title: `orig → path` for renames, else the path.

**Group headers**: buttons that collapse or expand the group, with a chevron and "N file(s)". Collapse state is component-local, keyed by title, and not persisted.

**Interaction** (`:57-81`):
- Click: `view.change = {repo, change}` plus `showDiff(repo, path, origPath, left)`. `left` is `rev(mergeBase)` in Compare, HEAD in Local, and right is always the working tree.
- Double-click, or Enter on the selected row: the same, plus `view.focusDiff++` so focus moves into the diff once it loads.
- Up/Down (on the `role=listbox` container): select the next or previous file (skipping group headers, clamped), show its diff, and reveal it. Focus stays in the list.
- Cmd/Ctrl+Down: opens the selected file in the working tree as a file (not a diff).
- The selection is derived from the global `view.change` (`:55`), shared with Cmd+D.
- There is no context menu, no stage/unstage and no revert.

**Diffs from elsewhere**: CommitDetails sets `view.shown` to a diff between two revs, or a rev and the working tree, plus `focusDiff` (`src/components/CommitDetails.svelte:50-54`). `vcs.fileHistory` uses the shown file or diff path (`src/components/Log.svelte:10-16`).

### 1.4 FindInFiles (`src/components/FindInFiles.svelte`)

**Modal**: a scrim backdrop (click closes) and a centered glass dialog (min(860px, 92vw) × min(580px, 80vh)).

**Head** (`:140-148`):
- "Find in Files".
- A status line: "Searching… N" while running, else (when text is set) "N match(es) in M file(s)".
- An "All repositories" checkbox.

**Inputs**:
- The text input (mono, 40px, placeholder "Text to find") is **selected on open** (`:129`). The previous text is kept between openings.
- Toggle buttons "Cc" (Match case), "W" (Words), ".*" (Regex) with `onmousedown` preventDefault so focus stays in the input (`:151-153`).
- The glob input has the placeholder "File mask: *.ts, !*.test.ts" (`:156`).
- A scope label on the right: "All repositories", or `<repo name>[ @ <ref>]`, or "No repository" (`:35`).

**Scope** (`:33-34`):
- Normally the current repo at its tree ref (`refOf(app.current)`), so it searches the browsed ref.
- With "All repositories": all repos' working trees (rev null).

**Search lifecycle**: see §4.

**Results list** (24px rows, virtual):
- File header rows show the basename, the directory and `<repo name if All> <count>`.
- Match rows (mono) show the right-aligned line number and the text with leading indentation dropped and hits highlighted (`.hit`).

**Keyboard** (on both inputs):
- Escape closes.
- Enter opens the selected match.
- Up/Down move between match rows, skipping file headers, with no wrap.

**Mouse**: moving the mouse over a match (with real movement, so content scrolling under a still pointer is ignored) selects it. A click opens it.

**Open**: closes the dialog, then `open({repo, ref: searchedRev, path, line})`. The Viewer reveals that line centered with the cursor at column 1 (not the match column). A markdown file opened this way shows the source.

**Notes**:
- "Results truncated at 10,000 matches. Narrow the search." when truncated.
- The error text (red) for regex or glob errors and the like.

### 1.5 Markdown (`src/components/Markdown.svelte`): see §3. [ported]

### 1.6 Palette: go to file, search everywhere, recent (`src/components/Palette.svelte`)

**Opening** (`src/lib/app.svelte.ts:252-255`):
- `file.goto` → mode `file`; `search.everywhere` → `everywhere`; `file.recent` → `recent`.
- Pressing Cmd+Shift+O again while Go to File is open toggles "All repositories" (`app.paletteAll`).

**Layout**:
- Titles: "Go to File", "Search Everywhere", "Recent Files" (`:16-22`).
- An "All repositories" checkbox in file and everywhere modes (`:131-133`). `app.paletteAll` is global and in memory only.
- Query input placeholder: "Files in <repo>" (file mode, single repo) or "Type to search".
- The list is not virtualized (max-height 420, scrolls).

**Entries** (`:68-85`):
- **file**:
  - Empty query: the recent files list (`nav.recent`, fuzzy-filtered client-side).
  - With a query: `api.quickOpen(repos, ref, q, 50)`. `ref` is `refOf(current)` for one repo and null across repos. A `seq` guard drops stale responses. Errors give an empty list.
- **recent**: `nav.recent` fuzzy-filtered by path (`src/lib/fuzzy.ts`).
- **everywhere**:
  - Empty query gives nothing, with the empty text "Search files, branches and actions".
  - Otherwise: files (`quickOpen` limit 10), then branches (local plus `remote/name`, fuzzy top 10, loaded once per palette open for `app.current`), then actions (fuzzy top 10).

**Row rendering**:
- Files: tree icon, the bold name with matched characters highlighted, the directory with highlights, and the detail `repo[ @ ref]`. `split()` maps code-point indices around the last `/` (`:112-119`).
- Branches: the name, then "Checkout".
- Actions: the title (muted when it has no `run`), then the chord.
- Empty list: "Nothing found".

**Keyboard**:
- Escape closes.
- Enter chooses the selected entry.
- Up/Down **wrap around** and `scrollIntoView({block:'nearest'})`.
- Typing resets the selection to 0. Changing mode resets the selection and focuses the input (`:87-91, 102-110`).

**Mouse**: moving the mouse (with real movement) selects; a click chooses; the backdrop closes the palette.

**Choose** (`:93-100`):
- A file opens `open({...f, line:null})`.
- An action runs `run(id)`.
- A branch runs `checkout(current, local|remote)`.

### 1.7 Navigation history (`src/lib/nav.ts`) [ported]

- `open(loc)` records into `nav` and calls the opener, which sets `view.shown` (`view.svelte.ts:29-31`).
- `recent`: files only (not MR or job docs), deduped by repo, ref and path, most recent first, capped at 50 (`nav.ts:19-21`).
- `history`: capped at 100. Forward entries are dropped on a new visit. Re-visiting the current entry (same file, line and doc) is not pushed (`:22-25`). Go to line pushes an entry with the line (`Viewer.svelte:159`).
- Back/forward (Cmd+[ / Cmd+], `app.svelte.ts:265-266`) replay a location without recording it again. The Viewer treats the replayed object as fresh, so it re-reveals the stored line.
- **Diffs are not in history** (`showDiff` and CommitDetails set `view.shown` directly). MR pages and job logs are.
- History and recent are in memory only. There are no mouse back/forward buttons and no back/forward UI buttons in these components.

---

## 2. Monaco usage (`src/lib/monaco.ts`, `Viewer.svelte`, `Markdown.svelte`) [ported differently: the kit's Editor, Diff and TextView in place of Monaco; built-in find kept; view zones become Diff annotations; the clash guard becomes key contexts]

**Setup**:
- Full `import * as monaco from 'monaco-editor'`, version 0.57.0 (`package.json`).
- **Worker**: `self.MonacoEnvironment = { getWorker: () => new Worker('/editor.worker.js', {type:'module'}) }` (`monaco.ts:4`). Only the generic editor worker exists. It is built as a separate Bun entrypoint (`build.ts:12-13`).
- **Language services off** (`monaco.ts:7-11`): `setModeConfiguration({})` for the TypeScript, JavaScript, CSS, SCSS, LESS, HTML, Handlebars and Razor defaults. JSON keeps `{tokens:true}`. So there is syntax coloring only: no hover, diagnostics, completion, document symbols or formatting.
- **Themes** (`monaco.ts:13-56`):
  - `wb-dark` (base `vs-dark`): editor and gutter background `#0e1220`, line highlight `#ffffff08`, no line highlight border, line numbers `#5b6685` (active `#a5b0c8`), selection `#818cf84d`, diagonal fill `#ffffff0d`, scrollbar slider `#ffffff14` (hover `#ffffff24`).
  - `wb-light` (base `vs`): `#ffffff` backgrounds, line highlight `#1e29500a`, line numbers `#8a93a8` (active `#3b4458`), selection `#6366f133`, diagonal `#1e29500d`, sliders `#1e295014` / `#1e295024`.
  - Diff colors use `add`/`del` bases: dark `#34d399`/`#fb7185`, light `#10b981`/`#f43f5e`. Inserted line `1f`, inserted text `38`, removed line `22`, removed text `40`, gutter inserted `2a`, gutter removed `2e` (alpha suffixes).
  - The theme switches live on `prefers-color-scheme` change (`Viewer.svelte:211-212`).
- **Instances**: one `monaco.editor.create` (a code editor for files) plus one `createDiffEditor`, **both created once on mount** in each Viewer (`Viewer.svelte:203-210`) and shown or hidden by CSS (`:346, 350`). Disposed on unmount.
- **Options set explicitly** (`:192-210`):
  - Common: `readOnly`, `domReadOnly`, `automaticLayout`, `minimap {enabled:false}`, `scrollBeyondLastLine:false`, `fontFamily` = CSS `--mono` (JetBrains Mono, ui-monospace, SF Mono, Menlo), `fontSize:13`, `theme`, `glyphMargin` only when `ongutter` was passed at mount.
  - DiffEditor: `originalEditable:false`, `renderSideBySide: split` (live via `updateOptions`, `:254`), `useInlineViewWhenSpaceIsLimited:false` (never auto-switches to inline when narrow), `ignoreTrimWhitespace:false` (whitespace changes count).
- **Left at Monaco 0.57 defaults** (not overridden; from `node_modules/monaco-editor/esm/vs/editor/common/config/editorOptions.js` and `diffEditor.js`):
  - Editor: line numbers on, `folding:true`, `wordWrap:'off'`, `renderWhitespace:'selection'`, `stickyScroll.enabled:true` (maxLineCount 5, outline model), `links:true` (URL detection), `contextmenu:true`, `occurrencesHighlight:'singleFile'`, `selectionHighlight:true`, `renderLineHighlight:'line'`.
  - Diff: `enableSplitViewResizing:true`, `renderOverviewRuler:true`, `renderIndicators:true` (+/- markers), `diffAlgorithm:'advanced'`, `maxComputationTime:5000`, `maxFileSize:50`, `hideUnchangedRegions.enabled:false` (the full file is shown), `renderMarginRevertIcon:true`, `renderGutterMenu:true`, `experimental.showMoves:false`.
  - I did not check whether the revert icon and gutter menu are inert on read-only models.
- **Built-in widgets that are reachable but not used by the app**: Monaco's own find widget (Cmd+F, which `clashesWithMonaco` deliberately lets through: `src/lib/files.test.ts:82`), Monaco's Ctrl+G go-to-line, F1 command palette and the Monaco context menu. None of these is intercepted. The app's "Go to line" is its own overlay input, not Monaco's.
- **Language detection**: `languageFor(path, monaco.languages.getLanguages())` (`files.ts:152-164`):
  1. Case-insensitive exact basename match against a language's `filenames` (e.g. `Dockerfile`).
  2. Otherwise the last extension (lowercased, only if the dot is not at index 0, so `.env` has no extension). An `extra` table comes first: `.svelte`, `.vue`, `.astro` → `html`; `.mjs`, `.cjs` → `javascript`. Then the first language whose `extensions` include it.
  3. Otherwise `plaintext`.

  The languages available are Monaco's registered set: plaintext; json, css, scss, less, html, handlebars, razor, typescript and javascript (with language-service modules); and the monarch basic languages: abap apex azcli bat bicep cameligo clojure coffee cpp csharp csp css cypher dart dockerfile ecl elixir flow9 fsharp freemarker2 go graphql handlebars hcl html ini java javascript julia kotlin less lexon lua liquid m3 markdown mdx mips msdax mysql objective-c pascal pascaligo perl pgsql php pla postiats powerquery powershell protobuf pug python qsharp r razor redis redshift restructuredtext ruby rust sb scala scheme scss shell solidity sophia sparql sql st swift systemverilog tcl twig typescript typespec vb wgsl xml yaml. There is no TOML.
- **Models and caching** (`Viewer.svelte:55-100`):
  - At most one model (file) or one original/modified pair (diff) per Viewer.
  - Key for a file: `repo\0ref\0path`. Key for a diff: `repo\0path\0JSON([left,right])` (`origPath` is not in the key).
  - Same key (a reload): `setValue` only if the text changed (`:65`). Different key: dispose the old models, `createModel(text, lang)`, `setModel`.
  - A placeholder disposes the models.
  - Editor view state (scroll, cursor) is not saved per file.
- **Diff APIs used**: `setModel({original, modified})`, `onDidUpdateDiff` → `revealFirstDiff()` (once per fresh show), `goToDiff('next'|'previous')`, `getLineChanges()` (for `otherLine`), `getOriginalEditor()` / `getModifiedEditor()`.
- **Editor APIs used**: `setPosition`, `revealLineInCenter`, `focus`, `getPosition` (Zed line), `updateOptions`, `getLayoutInfo` (`contentWidth`, `verticalScrollbarWidth`), `onDidLayoutChange`, `getDomNode`.
- **Decorations**: per side, `createDecorationsCollection()` with one decoration `{range: line, options:{glyphMarginClassName:'mr-plus'}}` on the hovered line where commentable (`:227-229`). It is drawn as a 16px accent rounded "+" via CSS (`:386-387`).
- **Mouse events**:
  - `onMouseMove` (ignores `CONTENT_VIEW_ZONE` targets) and `onMouseLeave` clear the hover.
  - `onMouseDown` on `GUTTER_GLYPH_MARGIN` or `GUTTER_LINE_NUMBERS` calls `ongutter` (`:213-239`).
- **View zones** (`:256-298`):
  - Each `Zone{side, afterLine, node}` goes on the original or modified editor via `changeViewZones(addZone({afterLineNumber, heightInPx: 80 initially, domNode}))`.
  - The zone wrapper has `z-index:1` and stops propagation of mousedown, pointerdown, keydown and contextmenu so Monaco does not steal clicks or keys.
  - The node width is kept equal to `contentWidth - verticalScrollbarWidth` on layout change.
  - The node gets `display:flow-root`.
  - A ResizeObserver updates `heightInPx` and calls `layoutZone`.
  - A capture scroll listener resets native `scrollLeft` on Monaco's layers caused by focus inside the zone.
  - Zones are re-added whenever the diff reloads (the `loaded` counter).
- **Keybinding clashes** (`files.ts:176-180`, `Viewer.svelte:345`): on keydown inside the editors, these chords get stopPropagation so Monaco never handles them: Cmd+K (Monaco chord prefix), Cmd+D, Cmd+L, Cmd+E, Cmd+[, Cmd+], F7, Shift+F7, Cmd+Shift+O, Cmd+Shift+F and the `vcs.popup` default (Ctrl+V on mac). Bound app chords are already taken earlier by the window capture listener. This guard also covers chords that were unbound or are only reserved.
- **`monaco.editor.colorize`** is also used for markdown code blocks (§3).

---

## 3. Markdown preview (`src/lib/markdown.ts`, `src/components/Markdown.svelte`) [ported differently: mermaid-rs-renderer and kit highlighting in place of mermaid.js and Monaco colorize]

**Renderer**: `marked` ^18.1.2 (`new Marked` with a custom `code` renderer), then `DOMPurify` ^3.4.17 with an allowlist, then `mermaid` ^12.2.0 (lazy `import('mermaid')`).

- **Allowed tags** (`markdown.ts:4-8`): h1-h6, p, br, hr, ul, ol, li, blockquote, pre, code, kbd, table, thead, tbody, tr, th, td, a, img, em, strong, b, i, del, s, sup, sub, span, div, details, summary, input.
- **Allowed attributes**: href, src, alt, title, align, start, colspan, rowspan, type, checked. DOMPurify's default data-attribute allowance keeps `data-lang`, `data-mermaid` and `data-path`. There are no `id` or `class` attributes from source.
- **GFM**: marked's defaults apply. Tables, strikethrough and task lists are rendered.
  - Task-list checkboxes are kept and set `disabled`; every other `<input>` is removed (`markdown.ts:29-32`).
  - `breaks:false` for repo files, `breaks:true` for MR text (`src/lib/mr.ts:61-62`).
- **Code blocks** (`markdown.ts:14-23`):
  - The fence language is the first word of the info string.
  - Output is `<pre><code data-lang="…">escaped</code></pre>`; without a language, `<pre><code>`.
  - Highlighting (`Markdown.svelte:23-27, 60-63`): `langOf(tag)` matches a Monaco language id or alias case-insensitively, else `languageFor('x.'+tag)`. If the result is not plaintext, `monaco.editor.colorize(text, lang)` replaces the code's innerHTML. It re-runs on HTML or theme change.
- **Mermaid** (`Markdown.svelte:29-49`):
  - ```` ```mermaid ```` becomes `<pre data-mermaid>`.
  - Init: `startOnLoad:false`, `securityLevel:'strict'`, theme `dark` or `default` from the color scheme, `fontFamily` = CSS `--font`.
  - Each block renders to an SVG in a `div.mermaid` placed after the `pre`, and the `pre` is hidden.
  - On a parse error: a `div.mermaid-error` with the message (red mono), the source `pre` stays visible, and mermaid's leftover `#d<id>` element is removed.
  - Re-rendering on theme change replaces the previous output sibling.
  - A failure of the whole mermaid step goes to `say(..., true)`.
- **Images in repo files** (`renderFile`, `markdown.ts:47-64`; `Markdown.svelte:66-70`):
  - An `img` without `src` is removed.
  - `https:` and `data:` sources are kept and load directly (CSP `img-src 'self' data: https:`, `src-tauri/tauri.conf.json:28`).
  - Scheme-less (relative or root-relative) sources lose `src` and get `data-path = repoPath(src, filePath)`. Then `api.readImage(repo, ref, path)` returns a data URL that is set as `src`; null leaves it empty.
  - Other schemes (e.g. `http:`) become a link `<a href=src>alt||src</a>`.
- **Links** (`Markdown.svelte:73-83`; click and auxclick, always preventDefault):
  - `http(s)://` opens externally via `@tauri-apps/plugin-opener` `openUrl`; errors go to `say`.
  - `#anchor` scrolls to the first h1-h6 whose `slug(text)` equals the decoded anchor. The slug is GitHub-style: trimmed, lowercased, non-letter/number/space/`_`/`-` removed, whitespace becomes `-` (`markdown.ts:37`). Heading ids are not rendered; matching happens at click time.
  - Relative or root-relative (no scheme) in a repo file: `open({repo, ref: file.ref, path: repoPath(href, file.path), line:null})`. `repoPath` resolves against the file, strips query and anchor, and URL-decodes (`markdown.ts:42`). The target opens at its top; the anchor is ignored.
  - Other schemes (`mailto:`, `javascript:`) do nothing.
- **Styling** (`.markdown.doc` for files, `Markdown.svelte:89-118`):
  - Max width 880px, centered, padding 24/32/48, 14px text, line-height 1.6.
  - h1 26px and h2 20px with a bottom border; h3 16px.
  - `code` 12.5px mono on a `--hover` background; `pre` rounded with horizontal scroll.
  - Blockquote with a left border; tables collapsed with 1px cells; images max-width 100%; mermaid centered.
  - Text is selectable.
- **Source/Preview**:
  - Only for `.md` / `.markdown` (case-insensitive, `markdown.ts:44`) files in editor mode and not conflicted. Never for diffs.
  - Default is Preview. It is persisted in localStorage `viewer.mdPreview` by the toggle buttons and by the `markdown.preview` action (`Viewer.svelte:180-183, 222`).
  - Opening at a line, or using Go to line, forces Source without persisting.
  - The preview gets the latest file text (it reloads with status).
  - The same `Markdown` component renders MR descriptions and notes with `render(text, links(mr))` (resolving relative URLs against the forge project, `mr.ts:54-80`).

---

## 4. Find in files and quick open

### Find in files: frontend (`FindInFiles.svelte`)

- **Query** (`src/lib/api.ts:77-83`): `{text, caseSensitive, wholeWord, regex, glob: trimmed || null}`.
  - The options object is module-level `$state`, so text, toggles, glob and "all" persist between openings for the session only (`:1-4`).
- **Debounce**: 150 ms after any change of query, scope or rev (`:37-48`).
- **`start()`** (`:50-80`):
  1. Cancels the previous query (`cancelFind(current)`) and clears groups, rows, selection, truncated, error and counts.
  2. If the text is empty or there are no repos, it stops (not running).
  3. Otherwise it takes a new id from `newId()` (a shared incrementing counter, `api.ts:248-250`), stores `searchedRev`, and calls `findInFiles(repos, rev, q, id, onEvent)`.
- **Events**: an event whose `queryId !== current` is ignored.
  - `match` events are buffered and flushed once per `requestAnimationFrame` (`:66-69, 82-99`).
  - Grouping is by `repo\0path` in arrival order (Map insertion order). Rows are rebuilt as a file header followed by its matches.
  - The selection goes to row 1 (the first match) on the first flush.
  - `done`: flush, stop running, set `truncated`. `error`: flush, stop, show the error. A rejected invoke shows its error.
- **Close or unmount**: `cancelFind(current)` and cancel the pending animation frame (`:130-134`).
- **Highlighting**: `highlight(text, ranges)` (`files.ts:127-150`) maps UTF-8 byte ranges to UTF-16 indices (handling 2-, 3- and 4-byte code points). It drops leading whitespace (ranges that fall inside the indentation are clipped) and emits `{text, hit}` runs.
- **Navigation**: opens at `m.line`, cursor column 1 (§1.4).

### Find in files: backend

- `find_in_files(repos, rev, query, queryId, events: Channel<FindEvent>)` and `cancel_find(queryId)` (`src-tauri/src/search/commands.rs:33-76`).
- It runs under `state.op(queryId, …)`, which registers a cancel flag for the duration (`src-tauri/src/state.rs:41-71`). `cancel_find` sets that flag only if the op is currently registered; a cancel that arrives before start or after finish does nothing.
- **Matcher** (`src-tauri/src/search/find.rs:102-108`): `grep_regex` (Rust regex), `case_insensitive(!caseSensitive)`, `word(wholeWord)`, `fixed_strings(!regex)`, line terminator `\n`. A bad pattern gives `Error`.
- **Working tree** (`rev` null, `:143-172`):
  - Parallel walk with `ignore::WalkBuilder` (`files.rs:19-26`): hidden files included, `.git` excluded, gitignore honored, parent ignore files and `.ignore` files not honored.
  - Globs are comma-separated, trimmed, and added as `ignore` overrides (a plain glob whitelists, `!glob` excludes).
  - `grep_searcher` with line numbers and binary detection that quits at a NUL byte.
  - Emission order is nondeterministic.
- **At a ref** (`:174-218`):
  - `git grep -n -z --full-name -I --no-color (-P|-F) [-i] [-w] -e <text> <rev> -- <pathspecs>`.
  - Globs become pathspecs, with `!x` → `:(exclude)x`.
  - Output is streamed line by line, cancellable. Exit code 1 means no match. `rev` is validated (no leading `-`).
- **Each hit** (`:38-66`):
  - Strips `\n` and `\r`, converts lossily to UTF-8, truncates to **1000 bytes** on a char boundary.
  - Ranges are recomputed with the Rust matcher on the (truncated) text, as byte offsets.
  - Emits `Match{queryId, repo, path, line, text, ranges}`.
- **Cap**: 10,000 matches (`CAP`), then the search stops. Repos are searched sequentially.
- **Final event**: `Done{truncated: count > CAP}`. A cancel gives `Done{truncated:false}`. Any other error gives `Error{error}` (`:127-140`).

### Quick open (`quick_open(repos, rev, query, limit)` → `FileMatch[]`)

- Non-streaming (`commands.rs:11-31, 48-56`).
- Per repo it takes cached path lists (`search/quick.rs:18-32`):
  - Working tree: the same walker as above, so untracked non-ignored files are included.
  - At a rev: `git ls-tree -r -z --name-only <rev>`.
  - The cache key is `(repo, rev)`, and **every `repo_status` call invalidates the repo** (`git/commands.rs:15`).
- Repos whose listing fails are skipped unless all of them fail.
- **Ranking** (`quick.rs:35-71`): `nucleo_matcher` `Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart)`, `Config::DEFAULT.match_paths()`. Order: score descending, then shorter path, then lexicographic. Top `limit`.
- `indices` are sorted, deduped character (code point) indices used for highlighting.
- The frontend passes limit 50 (Go to File) or 10 (Search Everywhere). Recent-file, branch and action matching is client-side `fuzzy` (`src/lib/fuzzy.ts`): a case-insensitive subsequence match where tighter and earlier matches score higher.

---

## 5. Persisted UI state [ported]

**localStorage** (via `src/lib/storage.ts`; failures are ignored):

| Key | Default | Where |
|---|---|---|
| `viewer.sideBySide` | true (any value other than `'false'`) | `Viewer.svelte:32,187` |
| `viewer.mdPreview` | true | `Viewer.svelte:127,182` |
| `tree.nesting` | true | `FileTree.svelte:31,36` |
| `size.tool` | 300 (range 220-800) | tool panel width via `Grip`, `App.svelte:35,61`; `src/components/Grip.svelte:14,25` |

`size.repos` (sidebar) and `sidebar.collapsed` exist but are outside this area.

**Session memory only** (lost on reload):
- Tree expansion per repo (module Map).
- `refs` (tree ref per repo).
- `compare` (Compare mode and base per repo).
- `view.change` (the selected change).
- Changes group collapse state.
- Find options.
- `app.paletteAll`.
- `nav.recent` and `nav.history`.

**Backend settings file**: `fileNesting` (custom nesting rules; there is no UI to edit them).

---

## 6. Backend commands used (`src/lib/api.ts`; Rust in `src-tauri/src/git/commands.rs`, `search/commands.rs`) [ported]

| Command | Args | Returns | Notes |
|---|---|---|---|
| `read_file` | repo, rev\|null, path | `Content` | null rev = working tree. `AppState::read_file` → `diff::content` (`git/commands.rs:19-22`). |
| `file_diff` | repo, path, origPath\|null, left: Side, right: Side | `{left, right}: Content` | Left side read at `origPath ?? path` (`diff.rs:149-161`). |
| `list_files` | repo, rev\|null | `FileEntry[]` (path, change\|null) | Changes only for the working tree (`files.rs:52-85`). |
| `list_changes` | repo, from, to | `FileChange[]` | HEAD/Index/Worktree combos via status; otherwise `git diff --name-status -z -M` (`diff.rs:88-147`). |
| `compare_base` | repo, base\|null | `{base, mergeBase, ahead, behind}` | Default base: `origin/HEAD` symref, else the first existing of origin/dev, origin/main, origin/master, dev, main, master; else the error "no default branch found; type one to compare with". Counts from `rev-list --left-right --count base...HEAD` (`diff.rs:32-86`). |
| `read_image` | repo, rev\|null, path | data URL \| null | png, jpg/jpeg, gif, webp, avif, svg, ico, bmp only. Path must be plain components (no `..` or absolute). ≤5 MiB. At a rev through `cat-file --filters` with LFS creds (`diff.rs:239-298`). |
| `open_in_zed` | repo, path, line\|null | () | Finds `zed`, `zeditor` on PATH, or `/Applications/Zed.app/Contents/MacOS/cli`; runs `<repo>/<path>[:line]` detached; error "Zed not found: no zed on PATH" (`files.rs:88-106`). |
| `repo_status` | repo | `RepoStatus` | Drives reloads; invalidates the quick-open cache. |
| `list_branches` | repo | `Branches` | Datalists, Search Everywhere branches. |
| `quick_open` | repos[], rev\|null, query, limit | `FileMatch[]` | §4. |
| `find_in_files` | repos[], rev\|null, query: FindQuery, queryId, events: Channel | () after Done/Error | Streaming, §4. |
| `cancel_find` | queryId | () | Sets the cancel flag if running. |

**`Content` rules** (`diff.rs`):
- `MAX_SIZE` is 5 MiB. Larger gives `TooLarge{size}` (checked from `ls-tree -l` size or file metadata, and again after read).
- A NUL byte in the first 8192 bytes gives `Binary`.
- Otherwise `Text` with lossy UTF-8.
- Working tree:
  - Missing file → `Missing`.
  - A directory with its own `.git` → `Submodule{HEAD sha}`, else `Missing`.
  - A symlink → its target as text.
  - Unmerged per `ls-files -u` → `Conflicted{text}`.
- Index side:
  - A conflict stage reads the working tree.
  - Mode 160000 → `Submodule`.
  - Otherwise the `:0:path` blob.
- Rev/HEAD side:
  - `ls-tree -l -z`.
  - An unborn HEAD gives `Missing`.
  - A gitlink gives `Submodule`.
- Blobs are read with `git cat-file --filters <spec>`, so eol, ident, LFS and other smudge filters apply. LFS credentials are attached when `filter=lfs` appears in info/attributes or in any `.gitattributes` at HEAD or the target (`git/remotes.rs:33-65`).
- Rev arguments that are empty or start with `-` are rejected (`git/mod.rs:24-29`).

**Streaming and cancel model**:
- Only find in files streams, through a Tauri `Channel`.
- Ids come from the shared JS `newId()` counter.
- The UI filters events by id and cancels on query change or close.
- The backend checks the flag per walk entry and per hit. `git grep` is killed through `Opts.cancel`.

---

## 7. Test coverage

### E2E (Playwright against the real backend via a bridge; `e2e/shim.ts`, `e2e/fixture.ts`)

**`e2e/changes.spec.ts:7-27`**:
- Changes list order and status classes (modified, renamed, untracked).
- A click shows the "HEAD ↔ Working tree" diff.
- Cmd+Down opens the file (header with no ↔).
- A double-click shows the diff.
- The cursor lands on the first change (line 5, `revealFirstDiff`).
- F7 moves to the next change (line 50).

**`e2e/changes.spec.ts:29-35`**: Compare mode with `↑2 ↓1`, the group "Changes since origin/main", and only the branch's files.

**`e2e/find.spec.ts:7-21`**:
- Cmd+Shift+F opens find.
- Results are grouped by file.
- `.hit` highlighting.
- The "Searching…" state.
- A new query replaces one still streaming (no stale alpha matches).
- The "2 matches in 2 files" count.

**`e2e/find.spec.ts:23-39`**:
- The Regex toggle with `\d`.
- Escape closes.
- Cmd+1 plus the tree ref input "main" and Enter sets a ref.
- The find scope label shows "solo @ main".
- Matches at the ref equal the working-tree matches (the git grep `-P` path).

**`e2e/markdown.spec.ts:28-72`**:
- Go to File opens a `.md`, and Preview is the default.
- h1; a disabled task checkbox; Monaco-colorized `code[data-lang=ts]`.
- One mermaid SVG plus a `.mermaid-error` "Parse error"; no leftover `#dmermaid*`.
- Re-render on the dark color scheme (new SVG id).
- A repo image as an `svg+xml` data URL.
- An `#usage` anchor scrolls into view.
- An external link goes to `openUrl`.
- A relative link opens `other.md` in the app.
- "Source" hides the preview, and Source sticks when another `.md` is opened (persistence).

**`e2e/palette.spec.ts:3-32`**:
- Go to File filtering (40 results), keyboard Up/Down under a resting mouse.
- Enter opens; header name; editor content.
- Case-insensitive query ("COMP-07").

**`e2e/palette.spec.ts:34-51`**: chords open Search Everywhere (Double Shift), Find Action, VCS Operations (Ctrl+V) and Recent Files (Cmd+E); Escape closes each; Cmd+, opens Settings.

**Related specs outside the four**:
- `e2e/workspaces.spec.ts:41-53`: tree nesting (kids hidden, chevron expands, "Nest related files" flattens), directory click expansion, Cmd+1.
- `e2e/mr.spec.ts:24-45, 47-127`: Viewer `ongutter` "+" hover and click on both sides, view zones holding the comment textarea, `commentable` limits ("+" only inside hunks), forced side-by-side, renamed file diffs.

### Unit tests (bun)

- `src/lib/files.test.ts`: groupChanges, statusOf/letter, buildTree/flatten ordering, dirty flags, nesting one level, `highlight` byte→char mapping and indentation drop, `languageFor` (filenames, extensions, `.svelte`→html, `.env`→plaintext), `clashesWithMonaco` (Cmd+K and Shift+F7 yes; Cmd+F and Cmd+C no).
- `src/lib/nav.test.ts`: recent capped at 50 and deduped by ref; history capped at 100; back/forward; forward entries dropped; no duplicate push; a new line counts; open hook.
- `src/lib/markdown.test.ts`: code `data-lang`, mermaid escaping, task checkboxes disabled, other inputs dropped, image rewriting (data-path, https kept, http becomes a link, `../`), `repoPath`, `slug` (including Cyrillic).
- `src/lib/fuzzy.test.ts`, `src/lib/keymap.test.ts` (chord parsing and matching, DoubleShift, overrides), `src/lib/mr.test.ts` (`otherLine`, MR markdown sanitizing).

**Rust tests**:
- `git/diff.rs` tests: change lists, renames, filters (ident/eol), conflicted, binary, too large, submodule, unborn HEAD, compare_base, images.
- `git/files.rs` tests: gitignore listing with status, listing at a rev, the Zed args.
- `search/find.rs` tests: literal, regex, case, word, bad regex → Error, globs and gitignore, git grep at a rev with globs, the 10k cap and truncated, cancel, regex parity between tree and rev.
- `search/quick.rs` tests: ranking and indices, the cache and invalidation, 50k files in under 100 ms.

### Untested behaviors (no e2e or unit coverage found)

**Viewer**:
- Placeholder states: binary, too large, missing, submodule, mixed-side diff message, error text.
- The conflict bar and its Open in Zed button.
- The Go to line overlay (Cmd+L, `line:col`, Escape, blur, invalid input).
- Shift+F7 and the header prev/next buttons.
- The Side-by-side/Unified toggle and its persistence.
- Cmd+D in each branch: from Changes focus, a file at a ref, a working-tree file with a rename.
- Open in Zed from the viewer (only the Rust argument test exists).
- Live reload on status refresh and window focus.
- The light/dark theme switch for Monaco.
- The empty-state hints.
- The `clashesWithMonaco` guard in a real editor.
- Model reuse.
- The `focusDiff` behavior on Enter in Changes (double-click is covered only through the cursor line).

**FileTree**:
- Keyboard navigation (arrows, Left to parent, Enter).
- The error state for a bad ref.
- The Back to the working tree × button.
- The branch datalist.
- Expansion surviving a tool switch or repo switch.
- Status coloring.
- The untracked nested repo marking.

**Changes**:
- Up/Down and Enter keyboard.
- Group collapse.
- The Refresh button.
- Conflicts and Unversioned groups.
- The compare base input and its datalist.
- The compare error for no default branch.
- Empty-state texts.
- Compare entered from the repo context menu.

**Find**:
- Case and Words toggles in the UI.
- The glob field in the UI (backend tested only).
- "All repositories".
- Keyboard Up/Down skipping headers, and Enter or click opening a match at its line.
- The truncation note in the UI.
- The error note in the UI.
- Option persistence between openings.
- Cancel on close.

**Palette**:
- Search Everywhere results (files, branches with checkout, actions).
- Recent Files contents.
- Empty-query Go to File showing recents.
- "All repositories" and the second Cmd+Shift+O toggling it.
- Wrap-around arrows.
- Opening at a ref.
- Backdrop close.

**Markdown**:
- The `markdown.preview` action.
- Opening a `.md` at a line forcing Source.
- Root-relative links.
- `http:` images turned into links.
- `mailto:`/other schemes being ignored.
- Middle-click.
- Images at a ref.
- Tables and other GFM rendering details.

**Nav**: Cmd+[ / Cmd+] back/forward in the UI (unit tests only).

**Backend**: LFS smudging through `cat-file --filters` (only the ident/eol filters are tested); symlink-as-text.
