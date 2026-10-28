# Workbench — milestone 1 plan

Working name `workbench`. A native-feeling, read-only git GUI over many repos with the JetBrains
git features and macOS keymap. Decisions come from the handoff of 2026-10-28; this file is how
milestone 1 gets built.

## Scope

In: accounts and providers, multi-repo sidebar with fetch/pull all, branch management (local and
remote), smart checkout, side-by-side and unified diff, file browsing (tree, viewer, quick open,
find in files, browse at any ref), JetBrains keymap, open in Zed.

Out (milestone 2 or later): commit, staging, push UI, commit graph, blame, MR/PR view, merge tool,
GitHub OAuth, file-system watcher, signing and notarization. Keys for out-of-scope actions are
listed in the keymap but bind nothing.

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
  main.rs, lib.rs        app setup, command registration
  git/                   runner, discovery, status, branches, checkout, diff, tree, grep
  accounts/              accounts.json, keychain, host matching, credential injection
  providers/             Provider trait: GitHub, GitHub Enterprise, GitLab.com, self-hosted GitLab
  search/                file list cache, fuzzy match, find in files
  settings.rs            repo roots, per-repo account overrides, keymap path
src/
  lib/api.ts             typed wrappers over Tauri commands
  lib/keymap.ts          parsing, matching, double-Shift detection
  components/            Sidebar, Branches, Changes, FileTree, Viewer, Palette, Accounts
Makefile                 `make check`: fmt, clippy -D warnings, cargo test, svelte-check, vitest
```

Config lives in the app config dir (`~/Library/Application Support/<bundle id>/`):
`settings.json`, `accounts.json` (no secrets), `keymap.json` (user overrides on top of defaults).

## Design

**Git runner.** One function runs `git -C <repo>` with `GIT_TERMINAL_PROMPT=0`,
`GIT_OPTIONAL_LOCKS=0` for reads, `LC_ALL=C` so stderr can be matched, and a timeout. It returns
exit code, stdout and stderr. A per-repo lock serializes mutating operations; reads do not take it.
Fetch/pull all runs at most 4 repos at once and reports each repo's result separately.

**Discovery.** No built-in repos. The user adds folders (a repo, or a folder of repos) through a
folder picker; they are saved in `settings.json`. Each folder is walked to depth 3, stopping at a
`.git` and skipping `node_modules` and dot dirs, since repos are often grouped (`project/api`,
`project/web`). Single repos can be removed from the list without removing their folder.

**Sidebar data.** Per repo: branch (or detached SHA), upstream, ahead/behind, dirty count, last
commit subject and date. Refreshed on window focus, after every operation, and on demand.

**Branches.** Local and remote lists (`git for-each-ref`), create from any ref, checkout, set or
unset tracking, delete local (`-d`, then `-D` after a confirm naming the unmerged commits), delete
remote (`git push <remote> --delete`), prune stale remote-tracking refs (`git fetch --prune`).
Checking out a remote branch creates a local tracking branch (`git switch --track`).

**Smart checkout.**
1. `git switch <branch>`. Git carries uncommitted changes when the branches agree on those files.
2. If it refuses because local changes would be overwritten:
   `git stash push --include-untracked -m "workbench: <from> -> <to>"`, then `git switch`, then
   `git stash apply --index`; on a clean apply, `git stash drop`.
3. If the apply conflicts or is refused, keep the stash, list conflicted files
   (`git diff --name-only --diff-filter=U`), name the stash in the UI and offer to open the files in
   Zed. Nothing is dropped unless the apply was clean.
4. Any other refusal (unknown branch, a lock held) is shown as-is and nothing is stashed.

**Accounts and providers.** An account is `{id, provider, host, username, label}` in
`accounts.json`; its token is a Keychain generic password (service `workbench`, account = id).
A repo's account is its override if set, else the account whose host matches the `origin` remote's
host (https or ssh form). Saving an account offers a token check (`/user` on GitHub,
`/api/v4/user` on GitLab), but a failed or 403 check still saves: the corporate GitLab answers only
from the office network. The provider trait in milestone 1 has `kind`, `api_base(host)` and
`verify(token)`; milestone 2 adds MRs/PRs behind it.

**Supplying the token to git.** Only for https remotes with a matched account. The runner passes
`-c credential.helper= -c credential.helper=<inline helper>` where the inline helper answers `get`
with the username and the token read from an env var set only on that child process. The empty
first value clears inherited helpers so the global `osxkeychain` neither answers nor stores the
token. No match, or an ssh remote: plain git, with the user's own helpers and ssh agent.

**Diff.** Change list from `git status` (working tree vs index vs HEAD) or
`git diff --name-status -z <a> <b>` (two refs). For one file, Rust returns both sides: a blob via
`git cat-file blob <rev>:<path>`, the index via `:<path>`, the working tree from disk. Binary (NUL in
the first 8 KB) or larger than 5 MB shows a placeholder instead of Monaco. F7/Shift+F7 move between
hunks.

**File browsing.** Working tree: `ignore` walk that respects `.gitignore`, nodes colored by status.
Any ref: `git ls-tree -r -z --name-only <ref>` and blobs via `git cat-file`. The viewer is Monaco
read-only with language from the extension; Cmd+L goes to a line. "Open in Zed" runs `zed` from
`PATH`, else `/Applications/Zed.app/Contents/MacOS/cli`, with `<path>:<line>`; at a non-working-tree
ref it opens the working-tree file.

**Quick open and find in files.** Rust keeps a file list per repo and ref, rebuilt on refresh, and
ranks with `nucleo-matcher` (scope: current repo, toggle to all repos). Find in files streams matches
over a Tauri `Channel`, cancels when the query changes, and stops at 10,000 matches. Options: case,
whole word, regex, path glob.

**Keymap.** Defaults are the JetBrains macOS keymap from the handoff, as `action id → key chord`.
`keymap.json` overrides individual actions. Double Shift is detected as two Shift keydowns within
300 ms with no other key between. Monaco's own bindings that clash (Cmd+K chords, Cmd+D, Cmd+L,
F7) are removed or re-pointed to the app's actions.

## Build order

1. **Scaffold** (main session): Tauri 2 + Svelte 5 + TS, Monaco loading, `make check` green.
2. **In parallel**, one agent each in its own worktree, owning disjoint directories:
   - A — `git/`: runner, discovery, status, branches, smart checkout, fetch/pull all, with tests.
   - B — `accounts/`, `providers/`: accounts file, Keychain, host matching, credential injection,
     with a test that `git credential fill` through the injected helper returns the token.
   - C — UI shell: layout, sidebar and branches panels against a mocked `api.ts`, keymap with tests.
3. **After A and C**: D — diff, file tree, viewer, quick open, find in files (Rust + UI).
4. **Wire** the UI to the real commands, try it on a folder of real repos, then a revmux review of
   the diff before handing it over.

Each step ends with `make check` green and a commit. Agents get the code-style rule: short, simple,
comments only for a non-obvious reason.

## Done when

- Adding a folder lists every repo under it (nested ones included) with branch, ahead/behind and
  dirty state matching `git status` in a terminal.
- Fetch all and pull all report a per-repo result; one failing repo does not stop the rest.
- Smart checkout's three paths behave as above, tested against temp repos.
- Pull and fetch over https to GitHub and to a self-hosted GitLab work with a token from the app,
  and `security find-generic-password -s workbench` shows the token is in the Keychain.
- Diff, tree, viewer, quick open and find in files work in the working tree and at another ref.
- The keymap table's milestone 1 actions all fire, double Shift included.

## Open questions

- App name: `workbench` is a working name; the bundle id follows the final name.
