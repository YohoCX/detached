<!-- Generated from the Tauri client on 2026-11-04 by read-only survey agents; file:line refers to that code. -->
# Parity inventory: git history and forge (MRs, pipelines). Detached, Tauri 2 + Svelte 5

All paths are relative to the repo root. Citations are `file:line`.

---

## 0. Where these components are mounted and how they are opened

- **Bottom panel** (`src/App.svelte:74`): `{#if app.bottom}<BottomPanel/>`. `app.bottom` is `'log' | 'pipelines' | null` and defaults to `null`, closed (`src/lib/app.svelte.ts:31`).
- **Strip buttons** (`src/App.svelte:49-51`):
  - "Merge Requests" toggles `app.tool='mrs'`.
  - "Git Log ⌘9" runs `panel.log`.
  - "Pipelines" runs `panel.pipelines`.
  - Each shows an `on` class while active.
- **Actions** (`src/lib/app.svelte.ts:269-271`). Each one toggles:
  - `panel.log`: `bottom = bottom==='log' ? null : 'log'`, so from Pipelines it switches to Log.
  - `panel.pipelines`: same logic.
  - `panel.mergeRequests`: toggles `app.tool`.
- **Chords** (`src/lib/keymap.ts:22`): only `panel.log` = Cmd+9 has a default chord. These actions have no chord: `panel.mergeRequests`, `panel.pipelines`, `vcs.fileHistory`, `mr.create`, `pipeline.run` (`src/lib/actions.ts:35-40`).
  - All of them are in Find Action.
  - Group `vcs` (fileHistory, mr.create, pipeline.run) also appears in the VCS Operations palette (`src/components/Palette.svelte:73`).
  - Users can rebind them via `keymap.json` overrides (`keymap.ts:83-90`).
- **Main area** (`src/App.svelte:69-71`): either Viewer, MrPage (`view.shown.kind==='mr'`) or JobLog (`kind==='job'`). Viewer stays mounted but hidden under MR/job pages.
  - `showMr` / `showJob` go through `nav.open`, so they are recorded in back/forward history (`src/lib/view.svelte.ts:34-36`, `src/lib/nav.ts:44`).
  - Commit-file diffs from CommitDetails set `view.shown` directly (`CommitDetails.svelte:52`), so they are **not** recorded in history.
- **Top bar** (`src/components/TopBar.svelte:46-47`): `<MrChip repo={app.current}/>` and `<PipelineBadge repo={app.current}/>`.
- **MR list** sits in the tool panel (`App.svelte:60-63`). The panel is resizable, Grip key `tool`, default 300, min 220, max 800.

---

## 1. Git history

### 1.1 BottomPanel (`src/components/BottomPanel.svelte`) [ported]
- Height defaults to 280. A top-edge Grip (key `bottom`) resizes it, min 120, max `innerHeight-240` (`:9,13`).
- Header has the tabs "Log" and "Pipelines". The tab with `app.bottom===tab` gets class `on`, and a click sets `app.bottom` (`:15-17`).
- Content: `<Log repo={app.current}/>` or `<Pipelines repo={app.current}/>` (`:20`).

### 1.2 Log (`src/components/Log.svelte`)

**State held at module level, for the session, not persisted** (`:8`): `filters = {repo, refs:{kind:'all'}, text:'', author:'', path:''}`.
- When the repo changes, refs and path reset to all and '', and the selection clears. Text and author are kept (`:57-61`).

**States:**
- No repo: "No repository" (`:178-179`).
- Error: the error text in red, pre-wrap (`:207-208`).
- Empty: "No commits" when there are no rows and nothing is loading (`:209-210`).
- Status text: "Loading…" while loading, else "N commits" with `toLocaleString` (`:191`).
- Truncated: warning "History truncated at 100,000 commits" (`:192`).
- Old rows stay on screen until the first batch of a new query arrives, so there is no flicker and the scroll position is kept (`:82-99`).

**Filter bar** (`:182-194`):
- Refs `<select>`: "All branches" (`all`), "HEAD" (`head`), then every branch as `b:<name>`. Branches come from `branchNames` = local names + `remote/name` (`src/lib/app.svelte.ts:167-168`). The list reloads on repo change or reload (`:63-68`). A refs change applies at once (`:115-117`).
- "Text or hash", "Author", "Path" inputs, debounced 300 ms (`:109-113`). Values are trimmed, and empty means null (`:74-79`).
- Refresh button bumps `reload` (`:193`).

**When the log reloads** (`:70-80`):
- Any filter change.
- `reload++`.
- `${st.head} ${st.branch} ${st.behind}` changes (`:53`), i.e. HEAD moves or a fetch brings commits. A status refresh alone does not reload.

**Streaming** (`:83-107`):
- Starting a query cancels the previous one (`api.cancelLog(current)`) and allocates a new `newId()`.
- Events whose `queryId !== current` are ignored.
- `rows` events are appended. `done` sets `truncated`. `error` sets `error`.
- The query is cancelled on destroy (`:171-175`).

**Graph shown?** Only when neither text nor author is set (`:86,91`). The graph column width is `min(maxLanes, 24) * 14px` (`:220`), where `maxLanes` is the max of `lanes(row)` over all rows (`:90`, `src/lib/log.ts:14`).

**Row** (`:217-228`), 28 px, virtualized (`VirtualList`, `src/components/VirtualList.svelte`, overscan 5):
- Graph SVG.
- Ref chips: the first 2 refs, then "+N" with a tooltip listing the rest, one per line (`:222-223`).
  - Chip kinds come from `log.ts:6-11`: `HEAD -> x` gives head chip "x"; bare `HEAD` gives head chip; `tag: v` gives tag chip "v"; anything else is a branch chip.
  - Colors: head = accent, branch = success, tag = warning, more = muted (`:262-266`).
- Subject.
- Author (140 px).
- Date: `Intl.DateTimeFormat(dateStyle medium, timeStyle short)` (`log.ts:26-27`), right-aligned, 150 px.

**Selection** (`:119-123`, `log.ts:17-24`):
- Click selects one row. Cmd (Ctrl off macOS) + click toggles a row (`src/lib/platform.ts:8`). Shift + click selects the range from the anchor index.
- The anchor moves on one/toggle, not on range.
- The selected style differs while the list has focus (`:258-259`).

**Keyboard** (`:125-132`), on the focusable `.rows` listbox:
- ArrowDown / ArrowUp move a single selection from the anchor. With nothing selected they go to row 0.
- The list scrolls to reveal the row. There is no Enter, Home/End or PageUp/PageDown.

**Context menu** (`:144-155`): a native Tauri menu via `popup` (`src/lib/menu.ts:10-19`). Right-click on an unselected row selects it first. Items:
1. **Copy Revision Number**: copies the full SHA to the clipboard, then status "Copied <sha>".
2. **New Branch…**: opens an inline bar (`:195-205`) reading "New branch at <sha10>".
   - Name input with autofocus. Enter creates, Escape cancels.
   - "Checkout" checkbox, default checked.
   - Create button (disabled while the name is blank) and Cancel.
   - Create calls `api.createBranch(repo, name, sha, false)`, then status "Created branch X". If Checkout is ticked it then runs `checkout(repo, {kind:'local', name})`; otherwise it does `reload++`.
   - On error: notice "New branch X failed" (`:157-169`).
3. **Checkout Revision**: `checkout(r, {kind:'rev', sha})`, the smart detached checkout with notices (`app.svelte.ts:206-225`).
4. **Compare with Local**: selects the row and sets `worktree=true`, so CommitDetails compares the commit with the working tree.
5. separator.
6. **Open in Browser**: `api.commitUrl`, then `openUrl`. With no URL: status error "No web page is known for this repository" (`:134-142`).

**Side panel** (`:233-236`): width 420, left Grip key `log.side`, min 260, max 1000. Shows `<CommitDetails repo shas worktree/>`, or "Select a commit" when nothing is selected.
- `shas` are the selected rows in list order, newest first (`:54`).

**File history** (`:10-16`): action `vcs.fileHistory`, registered at module load.
- If `view.shown` is not a file or diff, status "Open a file to show its history".
- Otherwise it sets `app.current = shown.repo`, `filters.repo/path = shown.repo/path` and `app.bottom='log'`. Refs, text and author are left as they were.

### 1.3 Graph (`src/components/Graph.svelte`) [ported]
- Lane width `LANE=14`, row height 28.
- 8-color palette: `#6366f1 #10b981 #f59e0b #ec4899 #06b6d4 #8b5cf6 #ef4444 #84cc16`, picked as `color % 8` (`:3,15`).
- `row.top` edges are drawn from y=0 to the middle; `row.bottom` edges from the middle to 28 (`:24-25`).
  - Edges with `from===to` are straight verticals.
  - Others are cubic béziers that are vertical at both ends, with control points at mid-y, so they join smoothly across rows (`:17-20`).
  - Stroke 2, round caps (`:31`).
- Dot: circle r=4 at the row's column, in the row's color (`:26`).

**Lane rules, in the backend** (`src-tauri/src/git/graph.rs:27-89`):
- A commit takes the leftmost lane waiting for its SHA, or else the leftmost free lane.
- Its color is that lane's color, or a fresh color counter for a new lane.
- Top edges: every active lane. Lanes waiting for this commit converge to its column and end; other lanes pass straight through.
- Parents:
  - The first parent continues in the commit's own column and color.
  - Each further parent reuses a lane already waiting for it, or gets a new free lane with a new color.
- Bottom edges = passing lanes + edges from the column to each parent's lane.
- Trailing empty lanes are trimmed. Freed columns are reused, and colors stay with their lane.
- A lane whose parent never arrives (truncation, shallow clone) runs to the last row.

### 1.4 Backend log semantics (`src-tauri/src/git/log.rs`) [ported]
- **Command**: `git log --date-order --format=LOG_FORMAT` (`:51-55`). Refs:
  - all → `--branches --remotes --tags HEAD`
  - head → `HEAD`
  - branch → that name (`:56-60`)
- **Text filter**: if the text is 7–40 hex characters, it is treated as a SHA prefix only, filtered client-side in Rust, never `--grep` (`:64-67,90-92`). Otherwise `-i --grep=<t>`.
- **Author**: `--author=`. **Path**: `--parents -- <path>`, which rewrites parents so the graph stays connected (`:68-77`).
- **No lanes** when text or author is set (`:79`).
- **Batches** of 500 (`BATCH`) are sent over the Channel (`:16,95-103`).
- **Cap**: 100,000 commits (`CAP`). Past it the callback returns false, which stops git, and `truncated=true` (`:17,83-86`).
- **Cancel** is checked between batches and by the runner. A cancelled query emits `Done{truncated:false}`, which the UI drops because the queryId is stale (`:33-35,98-100`).
- It runs as `Kind::Read`, so it has a 30 s limit (`src-tauri/src/git/runner.rs:16,163`). A slow full log errors out.
- **Commands**:
  - `commit_details` (`:154-186`): `%B` message, author and committer with their times, parents.
  - `branches_containing` (`:189-203`): `git branch -a --contains`, dropping symrefs and the `(HEAD detached…)` entry.
  - `merge_base` (`:205-217`): exit code 1 gives "X and Y have no common commit".
  - `range_log` (`:219-225`): `base..head`.
  - `hunk_ranges` (`:227-249`): `git diff -U3 -M --diff-algorithm=myers base head -- [origPath] path`.

### 1.5 CommitDetails (`src/components/CommitDetails.svelte`)
- **Props**: `repo`, `shas` (newest first), `worktree`. Used by Log and by MrPage's Commits tab.
- **Loading** (`:21-38`): `commitDetails(oldest)`, then the sides:
  - left = `worktree ? d.sha : (d.parents[0] ?? EMPTY_TREE)`, i.e. first parent for merges and the empty tree for a root commit.
  - right = `worktree ? {kind:'worktree'} : rev(shas[0])`.
  - Changes come from `listChanges(left, right)`, sorted by path. A stale result is discarded.
- **States**: error text, "Loading…".
- **Multi-select** (`:79-81`): "N commits" and "<parentOfOldest10> → <newest10>". It uses the oldest's first parent → newest even if the selection is not contiguous.
- **Single commit** (`:83-99`):
  - Subject (bold) and body (pre-wrap).
  - `dl` with Commit (full SHA) and Author "name <email>, date".
  - Committer, only if name, email or time differs from the author.
  - Parent/Parents (short SHAs).
  - Branches: a link "Show branches containing this commit", then "Searching…", then a comma-joined list or "None". An error shows as muted text (`:40-47`).
  - The info block is text-selectable and capped at 50% height with scroll (`:127`).
- **File list** (`:102-119`):
  - Header "N file(s)", plus " changed since, in the working tree" in worktree mode.
  - Rows are virtualized at 28 px: status letter M/R/A/D/U/C (`src/lib/files.ts:6-15`), basename (bold), dirname (muted).
  - Deleted files are struck through. A rename shows the tooltip "orig → path".
  - Click selects the file and opens its diff (`view.shown = {kind:'diff', repo, path, origPath, left, right}`).
  - Double-click opens it and moves focus into the diff (`view.focusDiff++`) (`:49-54`).
- **Keyboard** (`:56-66`): ArrowUp / ArrowDown select the previous / next file and open its diff. Enter opens the selected file with focus.
- In the Viewer, the diff header pill reads `sideName(left) ↔ sideName(right)`. A 40-hex rev shows as 10 characters; the others read HEAD, Staged or Working tree (`view.svelte.ts:42-49`, `Viewer.svelte:320`).

---

## 2. Merge requests

### 2.1 Setup states (`src/lib/mr.ts:5-11`), used by MrList
- No MR remote: "Choose the remote that holds the merge requests in Settings."
- MR remote not on any forge host: "Remote X is not on a forge."
- Host ambiguous: "Several accounts fit HOST; choose one in Settings."
- No account: "No account for HOST; add one in Settings."

The backend gives the matching errors from `Remote::project()` (`src-tauri/src/forge/project.rs:24-38`) and `project::mr` (`:167-174`): "No MR remote; choose one in Settings" and "Remote X is not on a forge".

Rules behind them:
- **MR remote** (`project.rs:130-137`): the saved remote if it exists, else `origin`, else the only remote.
- **Account per host** (`project.rs:85-101`): the per-repo `forgeAccounts[host]`, else the only fitting account, else the M1 `repoAccounts` choice when it fits. Several fitting accounts with none chosen means ambiguous.
- The Settings "Merge requests for <repo>" section edits these (`src/components/Settings.svelte:102-129`):
  - Remote select: "Automatic (x)" or "Choose a remote".
  - Account select per host: "Choose an account", "Automatic (label)" or "No account fits".
  - With no forge remote: "No remote is on GitLab or GitHub."

### 2.2 MrList (`src/components/MrList.svelte`)
- **Header**: "Merge Requests" and a Refresh button (`:42-45`).
- **Bar** (only with a repo): Search input, and a filter select with Open / Mine / Review requested / Assigned to me / All states.
  - The filter defaults to `open`. Filter and query are component-local, so they reset when the tool panel remounts (`:13-15,47-50`).
- **States, in order** (`:52-63`):
  - A stale-error line (small, red) when a cached value exists and the latest refresh failed.
  - "No repository".
  - "Loading…" when nothing is cached.
  - The setup message with an "Open Settings" button, which sets `app.settings=true`.
  - The error text when there is no value.
  - "No merge requests match" with a query, else "No merge requests".
- **Search**: fuzzy subsequence over `"number title author sourceBranch"`, ranked by score only while a query is set (`:19`, `src/lib/fuzzy.ts:6-23`).
- **Row** (`:66-80`), the whole row a button with the title as tooltip:
  - Line 1: `!N` on GitLab or `#N` on GitHub; the title with any `Draft:` / `[draft]` / `(draft)` prefix stripped (`mr.ts:14`); a "Draft" tag; a state tag when not open; a pipeline dot whose tone is success / error / accent (running or pending) / deleted color (`mr.ts:16-17`), tooltip "Pipeline <status>".
  - Line 2: "source → target", author, "✓N" approvals (tooltip "Approved by a, b"), relative time `ago` (`mr.ts:19-25`: Nm / Nh / Nd, a locale date after 30 days).
  - Click: `showMr(repo, number)`. The row matching the shown MR is highlighted.
- **Refresh** (`:22-39`):
  - On key change (repo or filter).
  - Every 60 s while `!document.hidden` and the list is mounted.
  - On window focus, skipped within 30 s of the last refresh (`src/lib/cache.svelte.ts:11-21`).
  - Refresh button: always.
  - Each refresh first calls `forgeHosts` and `setupProblem`, then `listMrs`.

### 2.3 MrPage (`src/components/MrPage.svelte`)

**Header** (`:155-167`):
- `!N` or `#N` (it shows `#` until the MR loads, since `gitlab` is false before then).
- Title (bare), "Draft" pill, state pill (open = green, merged = accent, closed = muted).
- Segmented tabs: **Overview | Commits | Changes | Pipelines**.
- Refresh button.

**Bars**:
- Failure bar: the last action's error, with a Dismiss button (`:168`).
- On non-Overview tabs, when the head moved: "The source branch moved; line comments are off until you refresh." and a Refresh button (`:169-171`).

**Load** (`:43-61`):
- `getMr` and `mrThreads` run in parallel. If the head SHA changed, the resolved refs are dropped.
- Changing repo or number resets everything and the tab goes back to Overview.
- There is **no polling and no focus refresh**. Only Refresh and actions reload.

**Actions** (`:63-76`): `act` sets busy, calls `api.mrAction`, then either clears or sets the failure text, then always reloads.

**Refresh** (`:103-106`): reload, then `refs=null`, so Commits / Changes re-resolve.

**Commits / Changes resolution** (`:79-101`). It is lazy and runs on the first visit to Commits or Changes:
1. If `mergeBase(baseSha, headSha)` fails (a commit is missing locally), it fetches.
   - The remote is `forgeHosts.mrRemote`; without one: "No MR remote; choose one in Settings".
   - The ref is `refs/merge-requests/N/head` on GitLab or `refs/pull/N/head` on GitHub, fetched together with the target branch.
   - It goes through `runOp('Fetch !N')`, so progress and a Cancel button show in the status bar (`app.svelte.ts:142-156`, `StatusBar.svelte:13-18`).
   - Backend: `git fetch <remote> <headRef> <target>` with no destination ref, then reads `FETCH_HEAD` (`src-tauri/src/git/remotes.rs:87-103`).
   - If the fetched head differs from the API head, the MR is reloaded once.
2. The base is `baseSha` on GitLab, or `mergeBase(baseSha, head)` on GitHub.
3. `commits = rangeLog(base, head)`, `files = listChanges(rev base, rev head)` sorted by path.
4. `moved = head !== mr.headSha`.
5. While this runs the tab shows "Loading…"; on failure, the error.

**Overview tab** (`:177-224`):
- Meta line: "**author** wants to merge `[srcProject:]sourceBranch` into `target` · updated Xago ago", a pipeline dot, and an "Open in browser" link (`openUrl(webUrl)`).
- `dl` with Labels (pills), Reviewers and "Approved by", each only when non-empty.
- Description: Markdown (MR mode), or "No description.".
- **When the MR is open**:
  - `<MergeWidget>`.
  - Action row:
    - GitLab only: **Approve / Unapprove**, toggled by `m.approved`.
    - **Mark as ready / Mark as draft** (`setDraft`).
    - **Close** (danger style).
  - GitHub only, a review block:
    - Textarea "Review comment, required to request changes".
    - **Approve**, shown only if not already approved.
    - **Request changes**, disabled while the text is blank; clears the text on success.
- **When the MR is closed**: **Reopen**. Merged MRs get no actions.
- **Activity** heading, then each thread (`<Thread>`), or "No comments yet.". Below them, a new-comment box (`start = body → {kind:'comment', body}`).
- All buttons are disabled while busy.

**Commits tab** (`:230-247`):
- Rows: sha8, subject, author, `ago`, with the absolute date as tooltip. Click selects the row; there is no keyboard handling.
- "No commits" when empty.
- Side panel (Grip `mr.side`, 420, min 260, max 1000): `<CommitDetails shas=[commit]>`, or "Select a commit".

**Changes tab** (`:248-271`):
- File column: Grip `mr.files`, 280, min 180, max 700.
  - Header "N file(s)".
  - Rows: status letter, basename, dirname, and a thread-count badge (tooltip "N comment threads"). The count is non-outdated threads whose position path equals the file's path or origPath (`:124`).
  - Click `pick(f)`: on GitHub it also loads `hunkRanges(base, head, path, origPath)`; an error goes to the status bar.
- Diff: `<Viewer doc={{diff rev base → rev head}} sideBySide zones commentable ongutter>`. Side-by-side is forced and the Unified toggle is hidden (`Viewer.svelte:325`).
  - `ongutter` is undefined, so there is no `+` at all, when the head moved or the MR is not open (`:268`).
- "Select a file" when nothing is picked.

**Line comments, i.e. how they attach to diff lines**:
- **Viewer gutter** (`src/components/Viewer.svelte:201,213-239`):
  - The glyph margin is enabled when `ongutter` is set.
  - Hovering a line on either side (original = old, modified = new) shows the `.mr-plus` decoration ("+" in an accent square, `:386-387`) if `commentable(side, line)` allows it.
  - A mousedown on the glyph margin or the line numbers calls `ongutter(side, line, otherLine(diff.getLineChanges(), side, line))`.
  - `otherLine` (`src/lib/mr.ts:28-44`) gives the other side's line number for an unchanged line, and null inside a change.
- **commentable** (`MrPage.svelte:119-122`): GitLab allows any line. GitHub allows only lines inside a hunk on that side (`newStart..newStart+newLines-1`, or the old range).
- **Draft** (`:134-140`): a click sets `draft={side, line, other}`. The action sent is `lineComment {body, path, oldPath: origPath ?? path, oldLine, newLine, headSha: ready.head}`.
  - GitLab: both lines for an unchanged line, else only the clicked side.
  - GitHub: the clicked side only.
- **View zones** (`:126-149`): each non-outdated thread on the file, plus the draft, is mounted as a `Thread` (`zone=true`) into a DOM node.
  - Zones go `{side, afterLine: position.line}`. The Viewer adds them to the original editor for `old` and the modified editor for `new` (`Viewer.svelte:257-298`).
  - Height is auto-fitted by a ResizeObserver, and the width follows the content width.
  - Mouse and key events are stopped from reaching Monaco, and stray horizontal scroll is undone.
  - Zones are re-added on every diff reload.
  - All zone Threads are remounted whenever threads, draft, MR or file change, so an open reply box inside a zone is lost on any reload.
- **Backend positions**:
  - GitLab (`src-tauri/src/forge/gitlab.rs:338-360`): `POST /discussions` with `{position_type:'text', base_sha, start_sha` from the MR's current `diff_refs`, `head_sha` from the client, `old_path, new_path, old_line?, new_line?}`.
  - GitHub (`github.rs:374-388`): `POST /pulls/N/comments {commit_id, path, line, side: RIGHT if newLine else LEFT}`.
- **Thread positions read back**:
  - GitLab (`gitlab.rs:286-327`): side is new when `new_line` is set, else old. Path is `new_path` or `old_path`. `outdated = position.head_sha != current head`. System notes are skipped.
  - GitHub (`github.rs:297-355`): `diffSide` LEFT → old. Line is `line ?? originalLine`. Outdated comes from `isOutdated`.
- Outdated threads show only in Overview, with an "outdated" pill (`Thread.svelte:39-44`).

### 2.4 Thread (`src/components/Thread.svelte`)
- **Existing thread**:
  - Unless in a zone, a header "path:line" with an "outdated" pill.
  - Notes: author (bold), `ago` time, Markdown body.
  - Resolved threads are drawn at opacity 0.75 (`:36-50,78`).
- **Buttons** (`:60-71`), shown when the thread is replyable or resolvable:
  - A "Resolved" label when resolved.
  - **Reply** opens a textarea.
  - **Resolve / Unresolve** sends `{kind:'resolve', threadId, resolved:!resolved}`.
- **Reply / new box** (`:52-59`):
  - Textarea, placeholder "Reply" or "Write a comment".
  - Autofocus (preventScroll) for replies and drafts only; not for the general new-comment box.
  - **Cmd/Ctrl+Enter** sends. **Escape** cancels (replies and drafts only).
  - Cancel button (replies and drafts only). Primary button "Reply" or "Comment", disabled while busy or blank.
  - Send: `reply {replyTo: thread.replyTo, body}` or `start(body)`. On success the text clears and `oncancel` runs (`:21-33`).
- **Provider flags**:
  - GitLab: every discussion is replyable, and `reply_to` is the discussion id. Resolvable if any note is resolvable; resolved if all resolvable notes are.
  - GitHub: review threads can be resolved and replied to (the reply goes to the first comment's databaseId). Issue comments and reviews with a body (not PENDING) become one-note threads that can be neither resolved nor replied to. Everything is sorted by the first note's time (`github.rs:59-75,339-354`).

### 2.5 MergeWidget (`src/components/MergeWidget.svelte`)
- **Status line with dot** (`:25-30`):
  - `autoMerge`: accent, "Set to merge when the pipeline succeeds".
  - `blocking`: warning, "Cannot merge: <reason>".
  - Otherwise success, "Ready to merge".
- **Controls**, hidden while auto-merge is set (`:31-44`). There is no UI to cancel auto-merge.
  - Method segmented control, only if more than one method: names "Merge commit", "Squash", "Rebase". It defaults to the first allowed method (`:12,33-37`).
  - "Delete source branch" checkbox (default off), shown when `canDeleteSourceBranch`. On GitHub it is hidden when auto-merge is ticked and a pipeline is running (`:15`).
  - "When the pipeline succeeds" checkbox (default off), shown only while `mr.pipeline` is pending or running.
  - Button "Set auto-merge" (auto ticked and running) or "Merge". Disabled while busy, or when blocking unless it is setting auto-merge.
  - It sends `{kind:'merge', method, deleteSourceBranch, autoMerge, headSha}` (`:17-21`).
- **Backend, GitLab**:
  - Methods: `squash_option` always → [Squash], never → [Merge], else [Merge, Squash]. `canDelete` is always true.
  - `PUT /merge {squash, should_remove_source_branch, sha}`, plus `auto_merge:true` on GitLab ≥17.11 or else `merge_when_pipeline_succeeds` (from `/version`) (`gitlab.rs:148-154,247-259,372-386`).
  - Blocking = `detailed_merge_status` with underscores turned into spaces (`:123-127`).
- **Backend, GitHub**:
  - Methods from the repo's `allow_*` settings.
  - `canDelete` = push permission on the source repo and the head ref is not its default branch.
  - Blocking text by `mergeable_state`: dirty → "conflicts", blocked → "blocked by branch protection", behind, draft, otherwise "checking mergeability" (`github.rs:134-143,170-179`).
  - Merge: `PUT /merge {merge_method, sha}`. If delete was requested, it then deletes `refs/heads/<branch>` in the source repo, only with push access there and only while the ref still points at the merged head.
  - Auto-merge uses GraphQL `enablePullRequestAutoMerge` with `expectedHeadOid`. Combining it with delete is an error (`github.rs:407-437,190-205`).
- **Other actions**:
  - Draft: GitLab rewrites the title with a `Draft: ` prefix (`gitlab.rs:49-60,387-390`). GitHub uses GraphQL convert / markReady.
  - Close / reopen: GitLab `state_event`, GitHub `PATCH state`.
  - Unsupported, returning "X is not supported by this forge": GitLab requestChanges, GitHub unapprove (`gitlab.rs:371`, `github.rs:406`).
- **Cache**: the backend keeps merged and closed MRs from `get_mr` for the session; any `mr_action` drops that entry (`src-tauri/src/forge/mod.rs:96,194-218`).

### 2.6 MrChip and the create-MR flow (`src/components/MrChip.svelte`)
- **Cache key**: only when the repo has a branch **with an upstream** (`:15`): `chip\0repo\0branch\0upstream`.
- **Visibility**: rendered only if `mr !== undefined` (`:62`). It is hidden when unknown, on error with no cached value, or without an upstream.
- **With an MR**: a pill chip with the MR icon, `!N` / `#N` and the bare title (title as tooltip); click opens the MR (`:64-67`).
- **No MR (null)**: an icon button "Create Merge Request" (MR icon + plus) (`:69`).
- **Refresh**: on key change (which also clears the form) and on window focus with the 30 s skip. **No interval polling**.
- **Backend `current_mr`**: matched on source project and branch.
  - The source is `@{push}`'s remote and branch, else `@{upstream}` (`project.rs:185-228`).
  - GitLab: `?state=opened&source_branch=` filtered by `source_project_id`. GitHub: `?head=owner:branch` filtered by `head.repo.full_name` (`mod.rs:177-186`, `gitlab.rs:268-280`, `github.rs:276-291`).
  - With source and MR project on different hosts, it returns None.
- **openForm** (`:30-40`), also the `mr.create` action, registered on mount:
  - With no key: status error "Create Merge Request needs a branch with an upstream".
  - If an MR exists, it opens it.
  - Otherwise `api.newMr`: title = the last commit's subject, target = the MR project's default branch, plus source and target projects (`mod.rs:220-235`). Error "The current branch has no upstream", or "The branch is on X, the MR project on Y" across hosts (`mod.rs:140-152`).
- **Form popup** (`:71-89`), `role=dialog`, aria-label "Create Merge Request":
  - "New merge request", then "`srcPath:srcBranch` into `targetPath`".
  - Title (autofocus), Target branch, Description (5-row textarea), "Draft" checkbox.
  - Error text.
  - Cancel; Create, disabled while busy or when the title or target is blank.
  - Backdrop click or Escape closes it (`:73,93`).
- **Create** (`:42-55`): `createMr {title, description, targetBranch, draft}`, then close the form, reload the chip, and open the new MR.
  - GitLab: `POST` on the **source** project with a Draft title; `target_project_id` for forks (`gitlab.rs:401-419`).
  - GitHub: `head` = `owner:branch` for forks, plus `draft` (`github.rs:456-478`).

### 2.7 Markdown (`src/components/Markdown.svelte`, `src/lib/mr.ts`, `src/lib/markdown.ts`) [ported]
- **MR mode**: `render(text, links(mr))`. Rendering:
  - `marked` with `breaks:true`, then DOMPurify with an allowlist of tags and attributes (`markdown.ts:4-9,26-34`).
  - Task-list checkboxes are kept but disabled; other inputs are removed.
- **Link resolution** (`mr.ts:46-59`):
  - Scheme or `#` links are kept.
  - Root-relative `/uploads/` on GitLab resolves under the project URL; other root-relative links under the base URL (which keeps a sub-path).
  - Relative links resolve against the project URL + `/`.
- **Images** (`mr.ts:64-79`): `https:` / `data:` images are kept, except GitLab project uploads. Those and every other image become a link with the alt text.
- **Clicks** (`:73-83`, click and auxclick):
  - http(s) opens in the default browser.
  - `#anchor` scrolls to the heading whose GitHub-style slug matches (`markdown.ts:37`).
  - In MR mode, everything else (e.g. `javascript:`) is dropped. Repo-relative links open in the app in file mode only.
- **Code blocks**: colorized by Monaco when the language is known (`:57-63`).
- **Mermaid** (`:29-49`):
  - Rendered with `securityLevel strict` and a theme that follows `prefers-color-scheme`, re-rendered on theme change.
  - The source stays in a hidden `<pre>`. On error, `.mermaid-error` shows the message.

---

## 3. Pipelines

### 3.1 Pipelines (`src/components/Pipelines.svelte`)
- **Props**: `repo`, optional `mr`.
  - In the bottom panel it has no `mr`.
  - In MrPage's Pipelines tab it gets `mr=number`: no branch toggle, no Run button, and data from `mrPipelines`.
- **Module state, for the session** (`:5`): `ui.all` (default false, "Current branch") and `ui.run`.
- **`pipeline.run` action** (`:7-10,81-85`): sets `app.bottom='pipelines'` and `ui.run=true`. The non-MR instance then opens the run form.
- **Bar** (`:162-172`):
  - Segmented "Current branch" / "All branches" (not on MR pages).
  - A note area (red, ellipsized, with tooltip): the last action error, or the refresh error when a list is cached.
  - "Run Pipeline…" (not on MR pages; disabled while busy).
  - Refresh.
- **States** (`:194-203`):
  - "No repository".
  - Error with no cached list. If the message ends with "in Settings", it adds an **Open Settings** button.
  - "Loading…".
  - "No pipelines".
- **Pipeline row** (`:205-223`), with a `data-pipeline` attribute:
  - Chevron, which turns into a down arrow when expanded.
  - Status glyph in color, with the status as tooltip.
  - Bold `#id`, ref, mono `sha8`.
  - Muted "source · created date · duration". The duration counts live up to now while the pipeline is unfinished.
  - Buttons:
    - **Retry Failed**: GitHub host && retryable && failed.
    - **Retry**: retryable.
    - **Cancel**: cancellable.
- **Click on a row toggles expand** (`:110-114`). It shows "Loading…", or the error, or the stages.
  - Stage line: glyph + name.
  - Job rows (`data-job`): glyph, name, an "allowed to fail" tag, duration. Click opens the job log in the main area (`showJob`).
  - Job buttons: **Play** (playable), **Retry** (retryable), **Cancel** (cancellable).
  - Expanded state clears when the key (repo, mr, all) changes (`:47-58`).
  - Each refresh re-fetches the jobs of every expanded pipeline and drops pipelines that are gone (`:87-97`).
  - Fetched jobs are recorded in `seen` (`src/lib/pipelines.ts:25-26`) for JobLog.
- **Glyphs and colors** (`pipelines.ts:5-15`):

  | Status   | Glyph | Color   |
  |----------|-------|---------|
  | success  | ✓     | success |
  | failed   | ✕     | error   |
  | running  | ●     | accent  |
  | pending  | ○     | warning |
  | manual   | ▶     | muted   |
  | canceled | ⊘     | muted   |
  | skipped  | »     | muted   |

  Durations (`took`): `Ns`, `Mm SSs`, `Hh MMm`.
- **Actions** (`:116-131`): `act` sets busy, which disables all action buttons. It clears the note, runs the action, puts any error in the note, then reloads.
- **GitHub detection** (`:60-68`): a host counts as GitHub when its resolved account's provider starts with `github`. Only those hosts get Retry Failed and workflows.
- **Run form** (`:133-153,173-193`):
  - Project = the first listed pipeline's project, else `newMr().sourceProject`; a failure goes to the note.
  - Branch defaults to the **local** branch name.
  - GitHub: a Workflow select of active workflows, default the first.
  - Key/Value rows, each with a remove ×. The add button is labelled "+ Input" on GitHub and "+ Variable" on GitLab.
  - **Run** is disabled while busy, with a blank branch, or on GitHub with no workflow. Keys that are blank are dropped. Success closes the form.
  - Cancel closes it. The form also resets on repo change.
  - Backend:
    - GitLab: `POST /pipeline {ref, variables:[{key, value}]}` (`gitlab.rs:525-538`).
    - GitHub: `workflow_dispatch {ref, inputs}`, then waits up to 10 s, polling every 1 s, for the new run. Error: "The workflow was dispatched; its run has not appeared yet" (`github.rs:584-617`).
- **Refresh and polling** (`:47-58,70-74,156`):
  - On key change and on HEAD or branch change.
  - **Every 5 s while any listed pipeline or any expanded job is pending or running**, and only while the document is visible.
  - Window focus, with the 30 s skip.
  - Refresh button.
- **Backend list semantics** (`mod.rs:244-260`, `gitlab.rs:421-447`, `github.rs:480-499`):
  - With an upstream: the source project, filtered by branch and the remote-tracking SHA (GitLab `ref` + `sha`, GitHub `branch` + `head_sha`), or every branch when `all`.
  - Without an upstream: the MR project, every branch.
  - Page size 20, single page.
  - MR pipelines: GitLab's MR endpoint (resolving each pipeline's project id); GitHub runs for the PR head SHA, per_page 100.
  - Actions address the pipeline's own project via `forge_for` (`mod.rs:116-124`).
- **Flags**:
  - GitLab pipeline: cancellable while created / waiting / preparing / pending / running / scheduled; retryable when failed or canceled.
  - GitLab job: playable when manual; retryable when failed, canceled or success; `liveLog=true`. Stages are ordered by job id, and a stage's rollup counts allow-failure failures as success (`gitlab.rs:62-78,449-489`).
  - GitHub run: cancellable while not completed; retryable when completed.
  - GitHub job: never cancellable or playable; retryable when done; `liveLog=false`; has steps. All jobs go under a single stage named after the workflow (`github.rs:41-57,501-537`).
  - Rollup priority: Running > Pending > Failed > Canceled > Manual > Success, else Skipped (`mod.rs:361-368`).

### 3.2 PipelineBadge (`src/components/PipelineBadge.svelte`)
- **Key**: only with an upstream: `badge repo head branch` (`:12`).
- **Value**: `listPipelines(repo, false)[0] ?? null`.
- **Visibility**: rendered only when a pipeline exists. It is hidden with no upstream, no pipelines, or an error with nothing cached.
- **Shape**: a button with glyph + status text, tooltip "Pipeline #id: status". Click sets `app.bottom='pipelines'`; it opens the panel and does not toggle (`:34-38`).
- **Polling**: every 5 s while the pipeline is active and the document is visible; focus refresh with the 30 s skip (`:25-32`).

### 3.3 JobLog (`src/components/JobLog.svelte`)
- **Header** (`:106-113`):
  - Status glyph, only if the job is known from `seen`.
  - Bold name, muted "project.path #id".
  - "Following…" while following.
  - Warning "Over 20 MB: showing the end only".
  - "Open in Browser" (`job.webUrl`).
- **States** (`:114-125`):
  - Error.
  - For a GitHub job that is still running (`following && job && !job.liveLog`): "GitHub serves the log once the job has finished." followed by the list of steps with glyphs.
  - Empty: "Waiting for output…" while following, else "Empty log".
- **Fetch loop** (`:32-85`): `api.jobLog(repo, project, id, offset)` returns `{text, next, complete}`.
  - The text is pushed into a `LogParser` and `offset = next`.
  - Job status and steps are re-read from `pipelineJobs` after every chunk when the job is not live (GitHub), and once at completion. This needs `seen`, which only exists for jobs listed in Pipelines this session.
  - **Polling interval**: 5 s for non-live (GitHub) jobs; for live (GitLab) jobs 3 s while offset < 2 MB, 10 s above. It stops when `complete`.
  - **Over 20 MB**: the parser resets and parses only the last ~1 MB (from a newline). `cut=true`, following stops, polling stops.
- **Errors**: shown in the body, and following stops. It is cancelled on unmount or prop change.
- **Backend** (`gitlab.rs:511-519`, `github.rs:561-568`, `mod.rs:387-398`):
  - GitLab: every poll does `GET /jobs/:id` for the status **and re-downloads the full `/trace`**, then slices from the offset. Complete when the status is not active and not `canceling`.
  - GitHub: empty and incomplete until the job completes; then the whole log via the 302 to a signed blob URL. The blob is fetched once, https (or test loopback) only, without the token (`http.rs:215-234`).
  - A chunk never splits a UTF-8 sequence in a running log.
  - The log download has no total timeout: 10 s connect and response, 120 s per read (`http.rs:44-58`).
- **Rendering** (`:126-139`):
  - Virtualized rows of 18 px, monospace 12 px, `white-space: pre`, text-selectable.
  - Indent `12 + depth*16 px`.
  - Section heads show a fold chevron, and a click toggles the fold.
  - Clicking an OSC 8 link opens it in the browser.
- **Auto-scroll**: stays pinned to the bottom while the view is within 4 px of it. Toggling a fold unpins (`:22,26-30,99-102`).
- **No `{@html}`**: spans come only from parsed codes. 16-color CSS variables differ for light and dark (`:144-154`).
- **ANSI parsing** (`src/lib/ansi.ts`):
  - SGR:
    - 0 = reset, keeping any link.
    - 1, 2, 3, 4 = bold, dim, italic, underline.
    - 22, 23, 24, 39, 49 = the matching resets.
    - 30–37 / 90–97 = foreground; 40–47 / 100–107 = background.
    - 38/48 `;5;n` = 256 colors (6×6×6 cube plus grayscale); `;2;r;g;b` = truecolor (`:16-40`).
  - OSC 8 links: http(s) only.
  - Other CSI / OSC / charset escapes and control characters are dropped.
  - A bare CR clears the line so far (the last overwrite wins); trailing CRs are stripped (`:43,96-107`).
  - GitLab `section_start:<ts>:<name>[collapsed=true]\r\e[0K`: the head starts open unless `collapsed=true`. `section_end` closes it. Sections nest (depth).
  - GitHub `[ts ]##[group]`: a head that starts collapsed. `##[endgroup]` closes it and its line is dropped.
  - An unclosed section folds to the end.
  - Lines made only of markers are dropped (`:44-45,80-111`).
  - The partial last line of a chunk is re-parsed on the next push (`:52-73`).
  - `visible()` skips the lines of folded sections (`:115-123`).

---

## 4. Persisted UI state

**localStorage**, via `src/lib/storage.ts`. Only Grip sizes (`size.<key>`) and viewer toggles (`src/components/Grip.svelte:14,25`) are persisted:

| Key | Default | Min / max | Where |
|---|---|---|---|
| `size.bottom` | 280 | 120 / innerHeight−240 | `BottomPanel.svelte:9,13` |
| `size.log.side` | 420 | 260 / 1000 | `Log.svelte:44,234` |
| `size.mr.side` | 420 | 260 / 1000 | `MrPage.svelte:26,244` |
| `size.mr.files` | 280 | 180 / 700 | `MrPage.svelte:27,251` |
| `size.tool` (MR list panel, shared) | 300 | 220 / 800 | `App.svelte:35,61` |
| `viewer.sideBySide` | true | (not a size) | `Viewer.svelte:32,187`. Commit diffs follow it; the MR Changes tab is always side-by-side. |

**Not persisted** (in memory only; lost on restart):
- Session level:
  - `app.bottom` (default closed) and `app.tool` (default `'changes'`).
  - Log `filters` (module level; refs and path reset on repo change).
  - Pipelines `ui.all` (module level, default "Current branch").
- Component level (reset when the component remounts):
  - MrList filter (default `open`) and query.
  - MrPage tab (Overview on each open).
  - MergeWidget choices.

**Session caches** (`src/lib/cache.svelte.ts`): the last value per key plus the latest error. A focus refresh within 30 s of the last one is skipped. Keys:
- `mrs\0repo\0filter`
- `pipes repo mr all`
- `badge repo head branch`
- `chip\0repo\0branch\0upstream`

**Backend in-memory caches**:
- `DONE`: merged and closed MRs (`mod.rs:96`).
- `ETAGS`: GitHub GET ETags with their replies; `If-None-Match` is sent and a 304 returns the cached reply (`http.rs:15,80-94,127-135`).
- `LIMITED`: rate-limit deadline per API base (`http.rs:13`).
- `seen`: job → pipeline map in the frontend (`pipelines.ts:25`).

---

## 5. Backend commands used (`src/lib/api.ts`)

**Streaming and cancel:**
- `log(repo, query{refs, path, text, author}, queryId, Channel<LogEvent>)` (`api.ts:311-312`). The command returns at once; events are `rows {queryId, rows}`, `done {queryId, truncated}` and `error {queryId, error}`. Cancel with `cancel_log(queryId)` (`src-tauri/src/git/commands.rs:207-225`).
- `fetch_mr(repo, remote, headRef, targetBranch, opId, Channel<string> progress)` returns the head SHA. Cancel with `cancel_op(opId)`.

**Git, request/response:**
- `commit_details(repo, sha)`
- `branches_containing(repo, sha)`
- `merge_base(repo, a, b)`
- `range_log(repo, base, head)`
- `hunk_ranges(repo, base, head, path, origPath)`
- `list_changes(repo, from: Side, to: Side)`
- `file_diff` (Viewer)
- `list_branches` (refs select)
- `create_branch(repo, name, start, checkout=false)`
- `checkout(repo, {kind:'rev', sha} | {kind:'local', name})`

**Forge, request/response** (`src-tauri/src/forge/commands.rs`):
- `forge_hosts(repo)` → `{hosts:[{host, remotes, accountId, ambiguous}], mrRemote}`
- `commit_url(repo, sha)` → string or null
- `list_mrs(repo, filter)` (filter: open, mine, reviewRequested, assigned, all)
- `current_mr(repo)`
- `get_mr(repo, number)`
- `mr_threads(repo, number)`
- `mr_action(repo, number, MrAction)`. The 11 kinds are in `api.ts:185-204`.
- `new_mr(repo)`
- `create_mr(repo, {title, description, targetBranch, draft})`
- `list_pipelines(repo, all)`
- `mr_pipelines(repo, number)`
- `pipeline_jobs(repo, project, id)` → `Stage[]`
- `pipeline_action(repo, project, id, retry | retryFailed | cancel)`
- `job_action(repo, project, id, retry | cancel | play)`
- `job_log(repo, project, id, offset)` → `{text, next, complete}`
- `workflows(repo, project)`
- `run_pipeline(repo, {project, branch, workflow, variables})`
- `list_accounts`, used by Pipelines for GitHub detection.

**Polling summary:**

| What | When | Where |
|---|---|---|
| MR list | every 60 s while mounted and visible | `MrList.svelte:34` |
| Pipelines | every 5 s while anything is active | `Pipelines.svelte:72` |
| Badge | every 5 s while active | `PipelineBadge.svelte:27` |
| Job log | 3 s / 10 s (GitLab), 5 s (GitHub) | `JobLog.svelte:73` |
| MrChip, MrPage | never | |

Window focus refreshes the MR list, Pipelines, badge and chip with the 30 s skip. `App.svelte:40` also refreshes repo statuses on focus, which can change the head key.

**HTTP semantics** (`src-tauri/src/forge/http.rs`):
- No redirects followed (`max_redirects 0`), except the GitHub log blob.
- API calls have a 10 s global timeout.
- Pagination: GitLab `X-Next-Page`, GitHub `Link rel=next` restricted to the same base, at most 50 pages (`:169-199`).
- Lists of MRs are capped at 50 (`LIST`, `mod.rs:94`).
  - GitLab: one GraphQL query, falling back to REST (list + per-MR GET + approvals, 8 threads at a time) (`gitlab.rs:162-266`).
  - GitHub: one GraphQL query of the 50 newest, with the mine / review / assigned filters applied client-side (`github.rs:210-274`).
- **Rate limit** (`:114-126,237-243`): 429, or 403 with `x-ratelimit-remaining: 0` or `retry-after`.
  - The deadline is `retry-after` or `x-ratelimit-reset` / `ratelimit-reset`, else now + 60 s.
  - Until then every call to that base fails without network, with "Rate limited; retrying after HH:MM:SS".
  - The UI has **no dedicated state**: the string appears as a normal error or stale-error line.
- **Errors** (`:246-291`):
  - A missing scope is named only when the provider signals it (GitLab `insufficient_scope`, GitHub `x-accepted-oauth-scopes` vs `x-oauth-scopes`): "<status>: the token lacks the X scope".
  - Otherwise "<status> <provider message>: <details>".

---

## 6. E2E coverage

### 6.1 Infrastructure
- **Playwright** (`playwright.config.ts`): WebKit, 1 worker. Ports from `E2E_PORT` (default 1520): app, bridge = +1, forge = +2.
- **Three web servers**: the Vite app; the `e2e-bridge` Rust binary, which runs the real commands over `/invoke` and streams NDJSON (`{event}` for Channel messages, then `{ok}` or `{err}`); and `node e2e/forge.ts`.
- **Shim** (`e2e/shim.ts:17-43`):
  - `mockIPC` forwards calls to the bridge and turns `{event}` lines into Channel callbacks.
  - `plugin:opener|open_url` URLs are collected in `window.__E2E__.opened`.
  - The dialog returns `__E2E__.pick`.
  - **`plugin:menu|*` is stubbed**, so native context menus (the Log row menu) cannot be exercised.
- **Fixture** (`e2e/fixture.ts:93-131,186-195`). `openForge` creates two local repos:
  - `forge/gitlab` (project `group/app`) and `forge/github` (project `owner/app`).
  - Base commit: `README.md` (30 lines) and `src/old.ts` (10 lines).
  - Branch `feature`: renames `src/old.ts` → `src/new.ts` with line 10 changed, changes README line 3, adds `added.txt`.
  - `origin` = `${forgeUrl}/{gitlab|ghe}/{path}.git`. `refs/remotes/origin/{main,feature}` are written locally and the upstream is `origin/feature`, so **nothing ever fetches from the fake**.
  - The forge is reset with a seed: user `tester`; MR 1 "Add feature" `feature→main`, with a description containing `<img src=x onerror=alert(1)>`; pipeline 100 (GitLab) / 200 (GitHub), success on the head, with job 101 / 201 "build" whose log is `\e[32mbuilt\e[0m <img src=x onerror=alert(1)>`.
  - Accounts are saved over the bridge: `gitlabSelfHosted` "Fake GitLab" (token `gitlab-token`) and `githubEnterprise` "Fake GitHub" (`ghe-token`), base URLs `${forgeUrl}/gitlab` and `/ghe`.

### 6.2 Fake forge (`e2e/forge.ts`), to be ported
- **Control endpoints**:
  - `GET /health` → `{projects:n}`
  - `POST /reset` (Seed)
  - `GET /__writes`: non-GET API requests with their bodies, excluding GraphQL `query`s
  - `GET /__requests`: every API request, method + decoded path
  - `POST /__delay {ms}`
  - `POST /__append {forge, path, job, text, status?}`: grows a job's log and can change its status
  - `GET /__blob/:projectId/:jobId`: returns 400 if an `Authorization` header is present (`:460-497`)
- **Auth**: GitLab requires `PRIVATE-TOKEN: gitlab-token`; GHE requires `Authorization: Bearer ghe-token`; otherwise 401 (`:505-506`).
- **Pagination**: `paged()` uses `per_page` (default 20) and `page`, with `X-Next-Page` (GitLab) or a `Link` header (GHE) (`:102-111`).
- **GitLab** `/gitlab/api/v4` (`:167-258`):
  - `/version` = 17.11.0, `/user`.
  - Project GET by path or id (`squash_option default_off`).
  - MR list with filters `state`, `source_branch`, `scope=created_by_me`. `assigned_to_me` and `reviewer_username` always return empty.
  - MR create (a Draft title sets draft).
  - Pipelines list (`ref`, `sha`), `POST /pipeline` (new pending pipeline, source `api`, one pending "build" job).
  - Pipeline jobs, retry (failed/canceled → pending, pipeline running), cancel.
  - Job GET, `/trace`, retry (resets the log), cancel, play.
  - MR GET; PUT (draft via title, `state_event` close/reopen); approvals GET; approve/unapprove; MR pipelines by head SHA.
  - Discussions GET; notes POST; discussions POST (with position); reply; resolve PUT.
  - Merge PUT: 405 if not open, 409 on SHA mismatch. Auto-merge flag when the head pipeline is not success, else merged. Deletes the branch.
  - GraphQL `/gitlab/api/graphql`: only the `mergeRequests(` query (state, authorUsername).
- **GHE** `/ghe/api[/v3]` (`:262-458`):
  - `/user`.
  - Repo GET (merge and squash allowed, rebase not; push permission).
  - Pulls list (`state`, `head`) and create.
  - `git/ref/heads` GET, `git/refs/heads` DELETE.
  - `actions/runs` and `workflows/:id/runs` (`branch`, `head_sha`, `event`); workflows (one "CI", id 1); dispatches (new pipeline `workflow_dispatch`).
  - Run GET, jobs, rerun, rerun-failed-jobs, cancel.
  - Job GET; `/logs` returns 404 while active and a 302 to `/__blob` when done; job rerun.
  - Issue comments GET/POST.
  - PR GET, PATCH state.
  - Reviews GET/POST: 422 for REQUEST_CHANGES without a body.
  - Review comments POST: 422 when `commit_id` is not the head; creates thread `PRRT_<id>`. Replies POST.
  - Merge PUT: 405, 409.
  - GraphQL: `pullRequests(`, `reviewThreads` (isOutdated always false), resolve/unresolveReviewThread, convertPullRequestToDraft, markPullRequestReadyForReview, enablePullRequestAutoMerge.
- **Not modeled**: labels, reviewers, assignees, ETag/304, rate limits, scope errors, outdated threads, git smart-HTTP, multi-page threads in practice.

### 6.3 Spec → behavior
- **`e2e/frame.spec.ts`**:
  - The forge repos track the seeded fake (`@{u}` = head SHA).
  - Cmd+9 opens the panel on Log. The Pipelines tab switches. Cmd+9 from Pipelines goes back to Log; Cmd+9 again hides the panel.
  - The strip Pipelines and Git Log buttons toggle and show `on`.
- **`e2e/mr.spec.ts`**:
  - **GitLab** (`:47-87`):
    - List row shows the title and "feature → main"; the page header shows the title.
    - The description is sanitized (no `img[onerror]`).
    - Changes file order: added.txt, README.md, new.ts.
    - A line comment on modified line 3 sends `new_line` only.
    - A comment on original line 5, an unchanged line, sends `old_line=5, new_line=5`, with the full position body.
    - Activity `.where` shows README.md:3 and README.md:5.
    - A general comment posts `/notes`.
    - Clicking the relative markdown link opens the resolved project URL in the browser; a `javascript:` link is not opened.
    - Approve turns into Unapprove and shows "Approved by tester".
    - Squash + Delete source branch + Merge sends `{squash:true, should_remove_source_branch:true, sha}`; the pill reads merged.
  - **GitHub** (`:89-126`):
    - `+` only inside hunks: README right side 20 no, 6 yes, 7 no.
    - Renamed new.ts: right 2 no, left 2 no, left 8 yes.
    - A line comment on right 9 sends `{commit_id, path:'src/new.ts', line:9, side:'RIGHT'}`.
    - Request changes is disabled without text; it posts, clears the text, and the review appears as an activity thread.
    - Review Approve sends `{event:'APPROVE'}` and shows "Approved by".
    - Merge sends `{merge_method:'merge', sha}`.
  - **Chip** (`:128-136`): the GitHub chip "#1" opens the page.
  - **Create MR** (`:138-158`):
    - A new `topic` branch with an upstream; sidebar refresh; the create icon opens the form.
    - The form shows "group/app:topic into group/app", Title prefilled "topic work", Target "main".
    - Create opens "!2" and the POST body is checked; the chip then shows "!2".
  - **Ambiguous host** (`:160-174`): "Several accounts fit 127.0.0.1", then Open Settings, pick the account in the "Merge requests for gitlab" section, Escape, Refresh, and the list loads.
  - **Cache** (`:176-194`): the list is one GraphQL POST. Returning to the repo shows the cached list within 1 s while the server is delayed 3 s, and a background refresh still happens.
- **`e2e/pipelines.spec.ts`**:
  - **GitLab** (`:33-73`):
    - The badge "✓success" opens Pipelines. Row #100 "feature" expands to job 101 "build"; the stage reads "✓build".
    - A successful GitLab pipeline has no action buttons.
    - Job log: header "group/app #101", the ANSI span carries `var(--ansi-2)`, the `<img>` is literal with no element.
    - Job Retry POSTs retry, and then a Cancel button shows.
    - Reopening the log shows "Waiting for output…". Appended text shows with "Following…"; status success ends following.
    - Run Pipeline: branch prefilled "feature", one Variable `DEPLOY=yes`, POST body checked, the form closes.
    - The new pipeline (source "api") shows first. Cancel POSTs cancel; Retry POSTs retry.
  - **GitHub** (`:75-113`):
    - Stage "✓CI". No Retry Failed on a success run; no job Cancel.
    - The finished log comes through the blob redirect.
    - Job Retry → rerun; the job then shows ○.
    - The log shows the steps view ("GitHub serves the log…", step "○Run", Open in Browser). Appending + success makes the log appear.
    - Run: workflow select value "1", one Input `level=2`, dispatch body checked.
    - The new run shows `workflow_dispatch`. Cancel → `runs/:id/cancel`; Retry → `rerun`.
  - **No account** (`:115-127`): with the GitLab account removed, the Pipelines tab shows "No account for … add one in Settings", the badge is hidden, and Open Settings opens Settings.
- **`e2e/markdown.spec.ts`** covers Markdown in **file** mode only: mermaid, code colorization, anchors, links, repo images.

### 6.4 Unit tests
- **Frontend**:
  - `src/lib/log.test.ts`: chips, lane count, pick.
  - `mr.test.ts`: sanitizer, link resolution, uploads as links, `otherLine`, `setupProblem`.
  - `ansi.test.ts`: SGR, CR, OSC 8, split chunks, GitLab sections, GitHub groups.
  - `pipelines.test.ts`: durations.
  - `cache.test.ts`, `markdown.test.ts`.
- **Backend**:
  - `src-tauri/src/git/graph.rs:137-235`: linear, branch and merge, two tips, octopus, criss-cross, column reuse with stable colors, missing parent.
  - `git/log.rs:393-560`: refs, filters, text/hash/author without graph, path filter keeps the graph, details/branches/merge base/range, renamed hunks, a new query drops old batches, 100k cap.
  - `src-tauri/src/forge/tests.rs:260-1519`: mapping, pagination, write bodies and positions, current MR matching, GraphQL list and fallback, DONE cache, pipeline project, GitLab log offset, GitHub blob redirect without the token, refused redirects, rate limits, scopes, If-None-Match, dispatch, account order, MR remote, URL resolution, forge_hosts, api bases, verify.

### 6.5 Behaviors with no e2e test
- **Log panel and CommitDetails**, entirely:
  - Graph rendering, filters (refs / text / hash / author / path, 300 ms debounce), truncation.
  - Selection (Cmd/Shift, arrow keys), the range diff, the context menu (stubbed in e2e), New Branch bar, Checkout Revision, Compare with Local, Open in Browser.
  - "Show branches containing", opening the commit-file diff, file-list keyboard, `vcs.fileHistory`, reload on head change.
- **MR page**:
  - The Commits tab.
  - The Pipelines tab.
  - The MR-ref fetch path and the head-moved notice: the fixture keeps all commits local.
  - Outdated threads, thread-count badges, failure-bar dismiss.
- **MR actions**: reply, resolve/unresolve, unapprove, draft/ready, close/reopen, auto-merge (incl. GitLab version key), the Rebase method, GitHub delete source branch, the blocking-state UI.
- **MR list**: filters other than Open, fuzzy search, 60 s polling, focus-refresh skip.
- **MrChip**: Escape/backdrop close, the error state, the `mr.create` palette action with no upstream.
- **Pipelines**:
  - The All branches toggle, Play (manual job), Retry Failed.
  - The 5 s polling (only indirectly), the `pipeline.run` action.
  - Job-log folding UI, OSC 8 link clicks, the 20 MB cut, the 3 s/10 s cadence, the Open in Browser click.
- **Markdown in MRs**: images and mermaid.
- **Errors and limits**: rate-limit and scope error display; "Remote X is not on a forge" / no MR remote states.
