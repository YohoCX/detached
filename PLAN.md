# Workbench — milestone 1 plan

Working name `workbench`. A native-feeling, read-only git GUI over many repos with the JetBrains
git features and macOS keymap. Decisions come from the handoff of 2026-10-28; this file is how
milestone 1 gets built.

## Scope

In: accounts and providers, multi-repo sidebar with fetch/pull all, branch management (local and
remote), smart checkout, side-by-side and unified diff, file browsing (tree, viewer, quick open,
find in files, browse at any ref), the keymap in [docs/KEYMAP.md](docs/KEYMAP.md), open in Zed.

Out (milestone 2 or later; further ideas in [docs/BACKLOG.md](docs/BACKLOG.md)): commit, staging, push UI, commit graph, blame, MR/PR view, merge tool,
GitHub OAuth, file-system watcher, notarization. M2 keymap rows bind nothing.

## Stack

| Part | Choice |
| --- | --- |
| Shell | Tauri 2 (stable 2.x; 3.0 is alpha), macOS only |
| UI | Svelte 5 + Vite + TypeScript, no SvelteKit (one window, no routing) |
| Viewer and diff | `monaco-editor` 0.57, `DiffEditor` with a side-by-side/inline toggle |
| Git | `git` CLI for everything, including status (`git status --porcelain=v2 --branch -z` gives branch, upstream, ahead/behind and dirty state in one call). `gix` only if status turns out measurably slow |
| Search | `ignore` walker + `grep-searcher`/`grep-regex` for the working tree; `git grep` for a ref |
| Fuzzy match | `nucleo-matcher` in Rust |
| Secrets | `security-framework` generic passwords (macOS Keychain) |
| Tests | `cargo test` against temp repos with a bare repo as remote; `vitest` for UI logic |

## Layout

```
src-tauri/src/
  lib.rs                 app setup; registers each module's `commands!` list
  env.rs                 login-shell PATH, git binary
  settings.rs            folders, excluded repos, per-repo account overrides
  git/                   runner, discovery, status, branches, checkout, diff, tree, grep
  accounts/              accounts.json, token store, account matching, credential args
  providers/             Provider trait: GitHub, GitHub Enterprise, GitLab.com, self-hosted GitLab
  search/                file list cache, fuzzy match, find in files
src/
  lib/api.ts             command names and payload types (the contract), typed wrappers
  lib/actions.ts         action registry
  lib/keymap.ts          parsing, matching, double-Shift detection
  components/            Sidebar, Branches, Changes, FileTree, Viewer, Palette, Accounts
docs/KEYMAP.md           default keymap
Makefile                 `make check`: fmt, clippy -D warnings, cargo test, svelte-check, vitest
```

Config lives in the app config dir (`~/Library/Application Support/<bundle id>/`):
`settings.json`, `accounts.json` (no secrets), `keymap.json` (overrides by action id).

## Design

**Environment.** An app started from Finder gets launchd's PATH, which lacks Homebrew (git-lfs,
hooks calling node through nvm or fnm). At startup the app reads PATH once from an interactive login
shell, so `.zshrc` counts too: `$SHELL -l -i -c 'printf "__WB__%s__WB__" "$PATH"'`, taking the text
between the markers since rc files may print. 5 s limit, fallback to the launchd PATH plus
`/opt/homebrew/bin`. That PATH is used for every child and to resolve `git`. Settings can pin the
git binary. The resolved git path and version show in Settings.

**Git runner.** One function runs `git -C <repo>` with that PATH, `GIT_TERMINAL_PROMPT=0`,
`GIT_OPTIONAL_LOCKS=0` for reads and `LC_ALL=C` so stderr can be matched. Each child starts in its
own process group, so a kill reaches remote helpers and LFS filters too. The runner returns exit
code, stdout and stderr as soon as git exits, after at most 1 s more of reading: a hook's background
job can inherit the pipes and keep them open forever. The reader threads keep draining (and
discarding) until EOF, so that job never writes into a closed pipe. Credential arguments (see
Accounts) go to network calls, and to switch, merge and `cat-file --filters` only when LFS is in
play, since those calls download objects. LFS is in play when `filter=lfs` appears in
`.git/info/attributes` or in any `.gitattributes` at HEAD or at the target commit
(`git grep -q filter=lfs <rev> -- ':(glob)**/.gitattributes'`): checkout smudges under the target's
attributes, so the first pull that adds LFS still gets the token. Hooks inherit git's
environment, so this keeps the token out of post-checkout and post-merge hooks in every other repo;
in an LFS repo they can see it.
Limits:
- reads (status, for-each-ref, cat-file, ls-tree): 30 s, then SIGKILL to the group;
- network (fetch, `push --delete`): run with `--progress`, so the server reports progress even while
  it prepares the pack, over https and ssh alike. No wall-clock limit: the call is cancelled when
  stderr is silent for 120 s, or when the user cancels. Cancel sends SIGTERM to the group, SIGKILL
  5 s later; git removes its lock files on SIGTERM. Progress lines show in the UI;
- local writes (switch, stash, branch, merge): no limit and no cancel, so git is never killed
  mid-write.

A per-repo lock serializes mutating operations; reads do not take it. It is released when the git
process exits; descendants it leaves behind (a credential-cache daemon, a hook's background job) are
left alone. Fetch/pull all runs at most 4 repos at once and reports each repo's result separately;
one failure does not stop the rest.

**Hooks.** A failing post-checkout hook makes `git switch` exit non-zero after HEAD has moved. After
a non-zero switch the runner reads `git symbolic-ref -q HEAD` (or `rev-parse HEAD` for a detached
target): if it is the target and was not before, the switch counts as done and the hook's stderr
shows as a warning. post-merge cannot change a merge's exit code, so a non-zero merge is always a
failure.

**Discovery.** No built-in repos. The user adds folders (a repo, or a folder of repos) through a
folder picker; they are saved in `settings.json`. Each folder is walked up to 3 levels below it.
A directory with a `.git` dir or `.git` file (a linked worktree) is a repo; the walk does not
descend into a repo, `node_modules` or dot dirs. Submodules are not listed separately. Removing a
repo adds its path to `excluded_repos`, so the next walk skips it.

**Sidebar data.** Per repo: branch (or detached SHA), upstream, ahead/behind, dirty count, last
commit subject and date, and an in-progress state (merge, rebase, cherry-pick) read from `.git`.
Refreshed on window focus, after every operation, and on demand.

**Pull.** Pull is two runner calls: `git fetch <upstream remote>` (network, cancellable), then
`git merge --ff-only @{u}` (local write, not cancellable). Pull all does the same per repo. A branch
that diverged from its upstream is reported as "diverged" and left untouched, so the app never
starts a merge it cannot finish. A merge or rebase the user started elsewhere shows as a badge on
the repo.

**Branches.** Local and remote lists (`git for-each-ref`), create from any ref, checkout, set or
unset tracking, delete local (`-d`; when that refuses, a confirm lists the commits from
`git log <branch> --not <upstream or HEAD>` before `-D`), delete remote
(`git push <remote> --delete`), prune stale remote-tracking refs (`git fetch --prune`). Checking out
a remote branch creates a local tracking branch (`git switch --track`).

**Smart checkout.** Runs under the repo lock. Refusals are recognized by git 2.50's `LC_ALL=C`
messages, matched case-insensitively and pinned by tests against the real git. A switch counts as
done when HEAD reached the target (see Hooks), whatever its exit code.
1. `git switch <branch>`. Git carries uncommitted changes when the branches agree on those files.
   Done: finish.
2. Refused with "your local changes to the following files would be overwritten": record
   `git rev-parse -q --verify refs/stash`, run
   `git stash push --include-untracked -m "workbench: <from> -> <to> <timestamp>"`, and read
   `refs/stash` again. The new value is the stash id. Unchanged (nothing was saved): report the
   original error and stop.
3. `git switch <branch>`. Not done: `git stash apply --index <id>` back on the original branch, drop
   the stash by id when that applies cleanly, and report the switch error. If that apply also fails,
   handle it as step 5 on the original branch.
4. `git stash apply --index <id>`. If git answers "conflicts in index. Try without --index", retry
   `git stash apply <id>`. Clean apply: drop the stash (its `stash@{n}` found by id) and finish.
5. Apply conflicted or refused: keep the stash, list conflicted files
   (`git diff --name-only --diff-filter=U`), name the stash in the UI and offer Open in Zed.
6. Any other refusal at step 1, including "untracked working tree files would be overwritten" and
   unknown branch: shown as-is, nothing stashed.

**Accounts and providers.** An account is `{id, provider, base_url, username, label}` in
`accounts.json`; `base_url` carries a sub-path when GitLab is installed under one. Its token lives
in a `TokenStore` trait: the Keychain implementation stores a generic password (service
`workbench`, account = id); tests use an in-memory store. For each https remote host of a repo the
account is: the repo's override if its host matches, else the only account on that host. Two
accounts on one host and no override: no token is supplied, and the repo shows "choose an account".
Saving an account offers a token check (`/user` on GitHub, `/api/v4/user` on GitLab); a failed or
403 check still saves, since some corporate GitLab hosts answer only from the office network. The
provider trait in milestone 1 has `kind`, `api_base(base_url)` and `verify(token)`; milestone 2 adds
MRs/PRs behind it.

**Supplying the token to git.** `accounts` builds the credential arguments; the runner only applies
them. For each matched host `H` (index `n`):
`-c credential.https://H.helper= -c 'credential.https://H.helper=!f() { test "$1" = get && printf "username=%s\npassword=%s\n" "$WB_USER_n" "$WB_TOKEN_n"; }; f'`,
with `WB_USER_n` and `WB_TOKEN_n` set only in that child's environment. The helper is scoped to
`https://H`, so other remotes, submodules on other hosts and redirects to other hosts never see it
and keep the user's own helpers. The empty value first clears inherited helpers for `H` only, so
`osxkeychain` neither answers nor stores the token. Username and token never appear in the shell
string. Hosts with no matched account, and ssh remotes, get no arguments.

**Diff.** Change list from `git status` (working tree vs index vs HEAD) or
`git diff --name-status -z <a> <b>` (two refs). For one file, Rust returns both sides: a ref or index
side via `git cat-file --filters <rev>:<path>` (index: `:0:<path>`), so LFS, eol and
`ident` filters match the working tree; the working tree side is read from disk. A conflicted file
shows its working-tree copy with markers and Open in Zed. A submodule entry shows its old and new
commit. Binary (NUL in the first 8 KB) or larger than 5 MB shows a placeholder instead of Monaco.
F7/Shift+F7 move between hunks.

**File browsing.** Working tree: `ignore` walk that respects `.gitignore`, nodes colored by status.
Any ref: `git ls-tree -r -z --name-only <ref>` and blobs through `cat-file --filters`. The viewer is
Monaco read-only with language from the extension; Cmd+L goes to a line. Open in Zed runs `zed` from
the login PATH, else `/Applications/Zed.app/Contents/MacOS/cli`, with `<path>:<line>`; at a ref other
than the working tree it opens the working-tree file.

**Quick open and find in files.** Rust keeps a file list per repo and ref, rebuilt on refresh, and
ranks with `nucleo-matcher` (scope: current repo, toggle to all repos). Find in files streams matches
over a Tauri `Channel` tagged with a query id; a new query cancels the old search, and the UI drops
messages with a stale id. Results stop at 10,000 matches with a "results truncated" notice. Options:
case, whole word, regex, path glob.

**Actions and palette.** Every command is an action `{id, title, group, run}` in one registry
(`src/lib/actions.ts`); the keymap binds action ids. One palette component serves five keymap rows:
- Go to file: quick open's files.
- Find action: every registry action by title, its chord shown beside it.
- VCS operations popup: the `vcs` group only (fetch, pull, checkout, new branch, delete branch,
  prune, fetch/pull all).
- Search everywhere: files, branches and actions together, files first.
- Recent files: the last 50 opened `{repo, ref, path}`, in memory, most recent first.

Back/forward walk a navigation history of `{repo, ref, path, line}` pushed on every file open and
go to line, capped at 100 entries.

**Keymap.** Defaults and Monaco clashes are in [docs/KEYMAP.md](docs/KEYMAP.md). `keymap.json`
overrides by action id. Double Shift is two Shift keydowns within 300 ms with no other key between.

**Signing.** Keychain access is tied to the app's code signature, and ad-hoc signatures change every
build, which would prompt for each token read. `make sign-setup` creates a local self-signed
"Workbench Dev" code-signing identity once; `bundle.macOS.signingIdentity` uses it for every build.

## Build order

1. **Scaffold** (main session). Tauri 2 + Svelte 5 + TS, Monaco loading, `make check` green, plus
   everything the parallel agents share: all crates in `Cargo.toml`, `env.rs`, `settings.rs`,
   `lib.rs` registering one `commands!` list per module, the runner's signature including the
   credential argument set, the full command contract in `api.ts` (names and payload types,
   with stub Rust commands returning fixtures) and the action registry with every M1 action id.
   Agents change the contract only through the main session.
2. **In parallel**, one agent each in its own worktree, owning only its directories:
   - A — `git/`: runner, discovery, status, branches, smart checkout, fetch/pull all, with tests.
   - B — `accounts/`, `providers/`: accounts file, token store, matching, credential arguments,
     Accounts component, with tests.
   - C — UI shell: layout, sidebar and branches panels against the stub commands, keymap, palette,
     recent files and navigation history, with tests.
3. **After A and C**: D — `search/` plus diff, file tree, viewer, quick open, find in files.
4. **Wire** the UI to the real commands, try it on a folder of real repos, then a revmux review of
   the diff before handing it over.

Each step ends with `make check` green and a commit. Agents get the code-style rule: short, simple,
comments only for a non-obvious reason.

## Tests

Every git test runs with `HOME` set to a temp dir, `GIT_CONFIG_GLOBAL` pointing at a temp file and
`GIT_CONFIG_NOSYSTEM=1`, so the developer's config never applies. Key cases:
- smart checkout: each numbered step above, including the failed second switch, the `--index`
  retry (git's real lowercase message), an unrelated stash pushed in between, nothing to stash, a
  post-checkout hook that exits 1 after the switch, the untracked-file refusal and an unknown branch;
- credentials: the temp global config sets a recording helper. Through the runner's arguments,
  `git credential fill` for `H` returns the account's token and the recording helper is not called;
  `git credential approve` for `H` does not reach it; `fill` for another host does reach it and gets
  no token;
- pull all: one fast-forward repo, one diverged repo, one with a broken remote;
- runner: cancelling a fetch leaves no git process in its group and no `.lock` file; a fetch whose
  server stays silent past 120 s is cancelled; a post-checkout hook whose background job inherits
  stderr and writes a line every 500 ms for 5 s gets its result back within 2 s of git's exit,
  releases the repo lock, and the job still writes every line; a switch in a repo without LFS
  runs its hook with no `WB_TOKEN_*` in the environment; a fast-forward to a commit that first adds
  `filter=lfs` gets the credential arguments;
- pull: a diverged branch reports "diverged" even with a post-merge hook installed;
- environment: PATH from a temp `.zshrc` that prints text and adds a directory;
- discovery: depth 3, a worktree `.git` file, an excluded repo, a skipped `node_modules`.

## Done when

- Adding a folder lists every repo up to 3 levels below it, with branch, ahead/behind and dirty
  state matching `git status` in a terminal.
- Fetch all and pull all report a per-repo result; a diverged repo shows "diverged" and is unchanged.
- Smart checkout passes every case listed under Tests.
- Branches: create, checkout of a remote branch, set and unset tracking, delete with the `-D`
  confirm listing the unmerged commits, delete remote and prune each work against a bare-repo
  remote.
- Pull over https to a GitHub repo and a self-hosted GitLab repo works with a token from the app,
  from the bundled app started in Finder; afterwards `security find-internet-password -s <host>`
  shows no entry written by the pull.
- For a changed file, the diff shows both sides in side-by-side and inline modes; at another ref the
  tree lists that ref's files and the viewer shows their content.
- Quick open finds a file by a fuzzy query in under 100 ms on a repo with 50,000 files.
- Find in files: a new query replaces the old results with no stale matches; a query with more than
  10,000 matches stops there and says so.
- Open in Zed opens the file at the right line.
- Every M1 row in docs/KEYMAP.md fires the action described under Actions and palette, double Shift
  included, in the viewer and outside it.
- From the bundled app started in Finder, a checkout runs a husky hook that calls `node`.

## Open questions

- App name: `workbench` is a working name; the bundle id follows the final name.
