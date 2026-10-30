# Detached — milestone 2 plan

Git history with a commit graph, changes per commit, and merge requests / pull requests with
pipelines, for GitLab (gitlab.com and self-hosted) and GitHub (github.com and Enterprise) behind one
interface. Decided with the owner on 2026-10-29: the log is a bottom panel, it covers the current
repo, and MR/PR pages can act (comment, approve, merge), including on pipelines.

**Status (2026-10-30): steps 1–3 done (scaffold; git, forge and log; MR and pipelines UI), merged on main. Next: step 4, wiring on real repos and a revmux review.**

## Scope

In:
- Log panel: commit graph of the current repo, branch and text/author/path filters, commit details,
  a commit's changes in the viewer, changes across a selected range, file history.
- MR/PR: list, page with overview, commits, changes and pipelines; general comments, replies, line
  comments in the diff, resolve threads, approve, merge (with squash, delete source branch,
  auto-merge), draft/ready, close/reopen, create an MR from the current branch.
- Pipelines: per repo and per MR, stages and jobs, job logs (live on GitLab), retry, cancel, play a
  manual job (GitLab), run a pipeline for a branch; the current branch's pipeline status in the top
  bar.

Out (backlog): commit, staging, push (milestone 3), blame, merge tool, one log across all repos,
an MR inbox across all repos, editor tabs, avatars, desktop notifications, OAuth.

## Layout

```
src-tauri/src/
  types.rs               all UI payloads, new ones included (mirrored by src/lib/api.ts)
  git/log.rs             streamed `git log`, commit details, merge base, range log
  git/graph.rs           lane assignment for the graph
  git/remotes.rs         + fetch of MR refs
  accounts/              + public remote URL parsing (https, ssh, scp); credential matching unchanged
  forge/                 was providers/: http.rs (client, pagination, errors, rate limits),
                         project.rs (remote → project and API account), gitlab.rs, github.rs
src/components/
  BottomPanel.svelte     tabs Log | Pipelines, resizable, Cmd+9
  Log.svelte, Graph.svelte, CommitDetails.svelte
  MrList.svelte          tool panel in the strip
  MrPage.svelte          Overview | Commits | Changes | Pipelines, in the main area
  MrChip.svelte          the current branch's open MR in the top bar
  Thread.svelte, Markdown.svelte, MergeWidget.svelte
  Pipelines.svelte       pipelines of a repo, or of an MR when given one
  PipelineBadge.svelte   the current branch's pipeline status in the top bar
  JobLog.svelte
e2e/forge.ts             fake GitLab and GitHub API for e2e
```

The main area shows one document: a file or diff (today's Viewer), an MR page, or a job log.
Back/forward history covers all three. The MR Changes tab reuses Viewer, so it keeps the loading,
placeholders and F7 navigation: the scaffold gives Viewer props for the document (today's file or
diff, or an MR diff with fixed sides), a forced side-by-side flag, and hooks for view zones and
gutter clicks on either side.

## Design

**Log data.** One `git log --topo-order` streamed through the runner's `lines` callback, one record
per line, fields separated by `%x1f`:
`--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s`. Rows go to the UI over a Tauri `Channel` in
batches of 500, tagged with a query id like find in files: a new query cancels the old one and stale
batches are dropped. It is a read (30 s limit). At 100,000 commits the callback returns `false`,
which stops git, and the UI shows "history truncated". Branch filter: all refs
(`--branches --remotes --tags HEAD`), HEAD, or one branch. Path filter: `--parents -- <path>`, so
git rewrites parents and the graph stays connected; file history is this filter set from the viewer.
Text (`--grep -i`, or a hash prefix) and author (`--author`) filters only hide commits and leave
parents pointing at hidden ones, so under them the list shows no graph. On a large repo the first
rows come quickly only when it has a commit-graph file; without one, `--topo-order` sorts the whole
history first.

**Graph.** Rust assigns lanes while streaming: each active lane waits for one commit; a commit takes
the leftmost lane waiting for it (or a new one), other lanes waiting for it end there, its first
parent continues the lane, further parents get a lane already waiting for them or a new one. A lane
whose commit never arrives (truncation, a shallow clone) ends at the last row. Each row carries its
column and the edges crossing it (from column, to column, color), so the UI draws one small SVG per
row inside the existing VirtualList. A lane keeps its color for its whole length. Ref labels
(`HEAD -> main`, `origin/main`, tags) render as chips before the subject. Columns are our own and
need not match `git log --graph`'s layout.

**Commit details and changes.** Full message, author and committer with dates, parents, and on
demand the branches that contain it (`git branch -a --contains`, a read; past 30 s it says so).
The file list is `list_changes(parent, commit)`: first parent for a merge, the empty tree for a
root commit. Selecting several commits shows the changes from the oldest one's first parent to the
newest. Clicking a file opens its diff in the main viewer. Row menu: copy hash, new branch here,
checkout revision (a new `CheckoutTarget` `{kind: 'rev', sha}`: `git switch --detach` through the
smart checkout steps), compare with the working tree, open in the browser.

**Remote to project.** The MR project is the project of one remote, chosen per repo in Settings
(`mr_remotes: {repo → remote}`): `origin` by default, else the only remote; with several remotes,
no `origin` and no saved remote that still exists, it is unset and the MR and pipeline views say so
and open Settings. No guessing from remote
names, since forks are wired in both directions (`origin` the team's fork and `upstream` the
original, or the reverse). Host and path come from https, `ssh://` and scp-style URLs; ssh hosts go
through `ssh -G <host>` first, so `~/.ssh/config` aliases resolve to the real hostname, and
`ssh.github.com` counts as `github.com`. A base URL with a sub-path (GitLab under `/gitlab`) is
stripped from the path. The current branch's source project is the project of its upstream remote,
and its source branch is the remote branch name of `@{push}` when set, else of `@{upstream}`.
When that differs from the MR project (a fork), an MR is created with GitHub `head=owner:branch` or
GitLab's source and target project ids (on different hosts, create is not offered), the create form
shows the target project, and the current branch's MR is matched on source project and branch,
never on the branch name alone.

The API account for a host is, in order: the repo's forge account for that host
(`forge_accounts: {repo → {host → account id}}`, any account allowed); else the only account that
fits the host; else milestone 1's `repo_accounts` choice when it fits. Otherwise the host is
ambiguous (several fit, none chosen) or unknown (none fits), and the MR and pipeline views say so and
open Settings. An account fits an https remote on host plus base-path prefix, an ssh or scp remote
on host alone; one `accounts::fits` predicate, made public in the scaffold, serves both milestone 1's
matching and `forge/project.rs`, which builds on it and the public URL parser.
`forge_hosts(repo)` lists each remote's host (after `ssh -G`) with its API account and an
`ambiguous` flag, for Settings to offer every account per host and the MR remote.
`accounts::remove` prunes `forge_accounts` as it prunes `repo_accounts`. `accounts::credentials` and
`repo_accounts` keep milestone 1's rule (https remotes only, an override only when its host matches),
pinned by a test, so a token never goes to a host it does not belong to.

**Forge client.** ureq with no redirects, as in milestone 1. API calls keep milestone 1's 10 s
total timeout; job log downloads have none, only 10 s to the response, and stop when the log
closes or after 120 s without a byte (`timeout_per_read`), since a stalled read blocks
and no cancel flag can stop it. The one followed redirect is GitHub's job log, which
answers with a redirect to a signed blob
URL: that URL is fetched once, https only, without the token. Lists follow `X-Next-Page` (GitLab)
and `Link` (GitHub). GitHub polling sends `If-None-Match`, since a 304 does not count against the
rate limit. Endpoints: GitLab `{base}/api/v4`; GitHub REST `https://api.github.com` or
`{base}/api/v3`, GraphQL `https://api.github.com/graphql` or `{base}/api/graphql` on Enterprise.
GitHub uses GraphQL only where REST has nothing: review threads (list, resolve), draft/ready,
auto-merge. The GitLab version (`/api/v4/version`) picks `auto_merge` or
`merge_when_pipeline_succeeds`.

Errors: a rate limit (429, or 403 with `x-ratelimit-remaining: 0` or `retry-after`) backs off all
polling until the reset and says until when. A missing scope is named only when the provider says
so: GitLab `insufficient_scope`, or GitHub's `x-accepted-oauth-scopes` not covered by
`x-oauth-scopes`. Everything else shows the status and the provider's own message (a 401 from
GitLab's approve means "cannot approve", not a bad token). Scopes for full use: GitLab `api`
(`read_api` reads only); GitHub classic `repo`, or fine-grained Pull requests, Actions and Contents
read-write, Commit statuses and Metadata read.

Both providers map to one set of types in `types.rs`: `MergeRequest` (number, title, state, draft,
author, source project and branch, target branch, base and head SHA, labels, reviewers, approvals, mergeability with
the blocking reason, allowed merge methods, pipeline summary, web URL, updated), `Thread` (id,
resolvable, resolved, replyable, the id a reply goes to, optional position
`{path, side: old|new, line, outdated}`, notes with id, author, markdown body and time; on GitHub
the id is the review thread's GraphQL id and replies go to its first comment's REST id, and a
general PR comment or any submitted review with a non-empty body (any state) is a thread of one note
that can be neither resolved nor replied to), `Pipeline` (id, status, ref, SHA, source, times, web URL, `cancellable`,
`retryable`, project) with `Stage` and `Job` (id, name, status, duration, allowed to fail, `cancellable`,
`playable`, `retryable`, `liveLog`). The flags carry what each provider can do, and the UI shows
only actions whose flag is set. On GitHub one workflow run is one pipeline, its jobs grouped by
workflow under one stage; an MR shows every run for its head SHA. GitHub jobs are neither
cancellable nor playable one by one (a run is cancelled as a whole) and have no live log: the
logs endpoint serves a job's log only after it finishes.

**MR changes.** Diffed locally, so the existing diff viewer and its speed apply. When the base or
head commit is missing, `git fetch <MR project's remote> refs/merge-requests/<n>/head` (GitLab) or
`refs/pull/<n>/head` (GitHub) plus the target branch, through the existing network runner with the
token; no destination ref, so nothing new appears among the user's refs. Those refs exist only in
the MR project, never in a fork. When the fetched head differs from the API's head SHA, the MR is
reloaded once; if they still differ (GitHub updates `refs/pull/<n>/head` a moment after a push), the
diff shows the fetched head with a "head moved, refresh" notice, and line comments are off
until the refresh, since their positions would name one head and their lines another. The diff is GitLab's
`diff_refs.base_sha` → head, or on GitHub the local merge base of base and head → head. The Commits
tab is `git log base..head`, and a commit opens the same details component as the log. The Changes
tab always shows side by side: in inline mode Monaco hides the original editor and its view zones,
so threads on removed lines would vanish.

**Line comments.** Threads render as Monaco view zones under their line, on the original side for
`old` and the modified side for `new`; a `+` in the gutter of the hovered line starts one. GitLab
takes any line; GitHub only lines inside the PR diff's hunks, so on GitHub the `+` shows only within
the hunks of `git diff -U3 -M --diff-algorithm=myers <merge base> <head> -- <old path> <path>`
(both paths, so a renamed file pairs up as on GitHub). Positions sent: GitLab
`{position_type: "text", base_sha, start_sha, head_sha, old_path, new_path}` with `new_line`,
`old_line`, or both for an unchanged line; GitHub `{commit_id: head, path, line, side: LEFT|RIGHT}`.
Threads whose position no longer applies to the current head show as outdated in Overview only.

**Actions.** Comment, reply, line comment, resolve/unresolve, approve (GitHub also request changes,
which requires a text),
unapprove on GitLab, merge with the methods the project allows plus squash, delete source branch,
auto-merge when the pipeline succeeds, draft/ready, close/reopen, create from the current branch (it
must have an upstream; target defaults to the project's default branch, title to the last commit's
subject; on GitHub `head` is `owner:branch` when the branch lives in a fork). GitLab deletes the
source branch itself. GitHub's merge cannot, so after a successful merge the app deletes
`refs/heads/<branch>` in the source project, and only when it has access there and the ref still
points at the merged head SHA; otherwise, and with auto-merge, the option is not offered. Every write refreshes the MR;
a failure shows the provider's message.

**Foreign content.** MR descriptions, comments and job logs are the first foreign content the app
renders, and the webview can call every command, including git and token reads.
- Markdown: GitLab `/uploads/…` paths are prefixed with the project's web URL; other
  root-relative links keep the base URL's sub-path, and relative ones resolve against the project's
  web URL. Then `marked` and `DOMPurify` with an allowlist of text, headings, lists, quotes,
  code, tables, links and images: no `form`, `input`, `button`, `area`, `map`, `iframe`, no `style`
  element or attribute. Every link click is intercepted: http(s) goes to the default browser through
  `tauri-plugin-opener`, anything else is dropped. The webview refuses any navigation away from the
  app's origin (an `on_navigation` hook in `lib.rs` allowing only the app protocol, plus `devUrl`
  in debug builds). Images load only from `data:` and
  https; GitLab uploads need a session and always show as links.
- Job logs: never `{@html}`. Text stays text nodes; spans come only from parsed ANSI SGR codes; OSC 8
  links go through the opener like markdown links.
- CSP replaces `csp: null`: `default-src 'self'; script-src 'self'; style-src 'self'
  'unsafe-inline'; img-src 'self' data: https:; font-src 'self' data:; connect-src ipc:
  http://ipc.localhost; worker-src 'self' blob:; form-action 'none'` (Vite inlines small font
  subsets as `data:`), with `dangerousDisableAssetCspModification: ["style-src"]`. Tauri would otherwise
  add nonces to `style-src`, and with a nonce present browsers ignore `'unsafe-inline'`, which
  blocks Monaco's runtime styles. The e2e Vite server sends the same CSP as a header, its
  `connect-src` widened by exactly `'self'` and `ws:` (Vite) and `http://127.0.0.1:1521` (the e2e
  bridge the IPC shim calls), so e2e runs under it.

**Pipelines.** The Pipelines tab of the bottom panel lists the source project's pipelines (current
branch, matched on the source branch and its remote-tracking ref's SHA, or all), since a fork's
branch pipelines run in the fork; with no upstream (a new branch, a detached HEAD) it shows the MR
project's pipelines, all branches;
an MR's pipelines come from the MR's own endpoint. Every pipeline carries its project, and job and
pipeline actions address that project; a pipeline expands to stages and jobs. A job opens its log in the main area: a
virtualized viewer that renders ANSI colors and folds GitLab `section_start/end` and GitHub
`##[group]` blocks. A running GitLab job's log is re-fetched every 3 s while under 2 MB and every
10 s above; past 20 MB it stops following and shows the tail with a browser link. A running GitHub
job shows its steps' status and a browser link, and its log once it finishes. Actions, each shown
only when its flag allows: retry, cancel and play a job; retry and cancel a pipeline; run a pipeline
for a branch with variables (GitHub: `workflow_dispatch` on a chosen workflow with key/value inputs,
re-run all or failed jobs). A visible running pipeline is polled every 5 s; otherwise on window
focus.

**MR list.** A strip tool beside Changes and Files. Filters: open, mine, review requested, assigned,
all states, plus fuzzy search. A row: number, title, source → target, author, pipeline dot,
approvals, updated. Refreshed on focus and every 60 s while visible.

**Keymap.** `panel.log` Cmd+9 is bound. The new actions (MR list, pipelines, file history) are in
the registry and Find action, without chords. docs/KEYMAP.md moves the commit, push and rollback
rows to M3 and lists the new action ids.

## Build order

Agents own only their files, and change the contract (`types.rs`, `api.ts`, the action list, the e2e
bridge) only through the main session, as in milestone 1. All git work is A's, all HTTP is B's.

1. **Scaffold** (main session):
   - the contract for log, forge and pipelines in `types.rs` and `api.ts`, including
     `CheckoutTarget` `rev`, the `Job`/`Pipeline` flags, `forge_hosts`, the `forge_accounts` and
     `mr_remotes` settings (`forge_accounts` pruned on account removal), `accounts::fits`, and the exact log format, with stub
     commands returning fixtures, each also routed in `bin/e2e-bridge.rs`;
   - every M2 action id in `actions.ts`; `panel.log` moved to the keymap defaults; docs/KEYMAP.md;
   - `providers/` renamed to `forge/`; `accounts` URL parsing made public with ssh and scp support;
   - component files with fixed props: BottomPanel mounting Log and Pipelines, MrPage mounting
     Pipelines with an MR, TopBar mounting MrChip and PipelineBadge, the main-area document shell,
     Viewer's props and hooks, and nav entries for MR pages and job logs;
   - `marked`, `dompurify`, `tauri-plugin-opener` with its capability, the CSP, the `on_navigation`
     hook, and the CSP header in the e2e Vite server; one existing e2e spec passes under it, and by
     hand Monaco's diff renders, IPC works and navigation away is refused in the bundled app;
   - test-only allowances, under `cfg(any(test, feature = "e2e"))`: http loopback base URLs are
     accepted by `accounts::save` and match accounts, http loopback remote URLs parse as projects,
     and the job-log blob may be http loopback. The GitHub e2e account is a GitHub Enterprise one at
     the fake's URL;
   - the e2e fake forge's frame: `FORGE_PORT` beside `BRIDGE_PORT` and a `webServer` entry that
     starts `e2e/forge.ts`, serving GitLab under `/gitlab` and GitHub Enterprise under `/ghe` (two
     base paths, so the two e2e accounts never fit the same host and path); a `/reset` that takes
     the fixture's seed (projects, MRs with their base and head SHAs, pipelines), in a format fixed
     here;
   - in `e2e/fixture.ts`, two repos, one per provider, in their own folder that only an
     `openForge()` helper adds, so `open()` and the milestone 1 specs keep their six repos and their
     Fetch/Pull All results. Each repo's `origin` points at the fake and its current branch tracks it
     (remote-tracking refs written locally); it holds its MRs' base and head commits so no fetch
     reaches the fake, and the fixture seeds the fake with the SHAs it just made.
2. **In parallel**, one Opus agent each in its own worktree:
   - A — `git/`: streamed log, filters, lanes, details, merge base and range log, hunk ranges for
     GitHub comments, MR ref fetch, detached checkout, with tests.
   - B — `forge/` and the endpoints of `e2e/forge.ts`: project and API account resolution, GitLab and GitHub clients for MRs,
     threads, actions, pipelines and logs, errors and rate limits, with tests against a local fake
     server; the e2e fake covers every endpoint the clients call.
   - C — BottomPanel, Log, Graph, CommitDetails: graph rendering, filters, details, commit changes,
     file history, against the stubs.
3. **After A, B and C**, in parallel: D — MrList, MrPage, MrChip, Thread, Markdown, MergeWidget,
   Viewer's MR hooks, the Settings forge-account and MR-remote pickers, the MR e2e specs; E — Pipelines,
   PipelineBadge, JobLog, the pipeline e2e specs. D mounts E's Pipelines only through the scaffold's props.
4. **Wire** to the real commands, try it on real repos (writes only on the `sandbox` repo), then a
   revmux review.

Each step ends with `make check` green and a commit. Agents get the code-style rule: short, simple,
comments only for a non-obvious reason.

## Tests

- Graph: synthetic histories (linear, branch and merge, octopus, criss-cross, path-filtered, a
  missing parent) with expected columns and edges; 100,000 commits stream and get lanes in under
  3 s.
- Log: filters and range changes against temp repos; an MR ref fetch goes to the given remote; a new query drops the old one's batches; the
  100,000 cap stops git.
- Forge, against a local fake server: fixture JSON of both providers maps to the shared types and
  flags; pagination; redirects refused except the GitHub log blob, fetched without the token;
  rate-limit back-off; scope named only from the provider's signal; write bodies, including GitLab
  positions (`position_type`, an added, a removed and an unchanged line) and GitHub `side`/`line`;
  the Enterprise GraphQL URL; the MR project follows `mr_remotes`, else `origin`; the account order,
  including an ambiguous host; the current branch's MR matches on source project and branch; branch
  pipelines come from the source project and actions use the pipeline's project; a fork PR's branch delete goes to the source project and is skipped
  when the ref moved; `credentials` still gives no token to an ssh remote or to a host the override
  does not match; remote URL parsing for https, ssh, scp, an ssh alias and a sub-path
  base URL.
- e2e, against `e2e/forge.ts` as a GitLab and a GitHub Enterprise account, under the CSP: MR list
  and page, a line comment lands with the right position, GitHub offers `+` only inside hunks,
  a renamed file's hunks, approve, merge, retry a job, a live GitLab log; a description
  carrying `<img onerror>` has no `img[onerror]` in its DOM, and a job log shows `<img onerror` as
  literal text.
- vitest, without a CSP: Markdown's sanitizer strips `on*` attributes, `<script>`, `<style>`,
  `style=` and `<form>`, and relative links resolve as described.

## Done when

- Cmd+9 opens the log. On a repo with 100,000 commits and a commit-graph file the first rows show in
  under 300 ms. Rows come in the order of `git log --topo-order` for the same refs, and each row's
  edges lead to exactly its (rewritten) parents.
- A commit's changes and a selected range's changes open in the diff viewer; file history works from
  the viewer.
- For a repo on a self-hosted GitLab and one on GitHub: the MR list and page load; comment, line
  comment, reply, resolve, approve and merge work on a sandbox MR; an MR can be created from a pushed
  branch.
- Pipelines show with their jobs; a running GitLab job's log follows live and a finished GitHub
  job's log shows; retry and cancel work where the provider allows them; the top bar shows the
  branch's pipeline status.
- Foreign HTML and logs cannot run script (e2e, under the CSP), and the bundled app runs with the
  CSP on.

## Open questions

- a self-hosted GitLab's GitLab version decides which merge and approval endpoints exist; check it at
  the start of step 2.
