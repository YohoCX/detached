# Tech debt

Bugs and shortcuts found after milestone 1, newest first. Features go to [BACKLOG.md](BACKLOG.md).

| Issue | Where | Notes |
| --- | --- | --- |
| Long markdown blocks the native UI on open | the kit's `TextView` (V2's markdown) | A 2,000-line (95 KB) file takes 220–275 ms of layout in one frame (kit #3695, measured in step 0). Work around in V2, e.g. by splitting large documents. |
| Native client draws in system fonts | `crates/app` `ui::theme` | The system UI font and the kit's mono font stand in for Manrope and JetBrains Mono (shell.md §5). Bundling them takes their TTF files in the repo (the fontsource packages hold woff2 only) and `add_fonts` at boot. |
| Worktree walks return `\` paths on Windows | `git::files::paths`/`list_files`, `search::find` | Git's own paths use `/`; quick open, the tree and find in files would mix both. Their tests are `#[cfg(unix)]` until fixed: `files` worktree listing, `find` literal/regex and glob. |
| Core tests that run on Unix only (`#[cfg(unix)]`) | crates/core | Shell stand-ins, process groups or `/` in expected paths. `git::checkout`: the 5 hook tests; `git::pull`: post-merge hook; `git::cred_tests`: all 3 (hooks); `git::runner`: cancel and silence limit (fake ssh, `ps`, `killpg`); `git::files`: fake zed; `git::remotes`: remote URLs; `git::discovery`: walk and names; `settings`: workspaces; `accounts`: credential helper script; `forge`: fake `ssh -G`; `env`: zsh login shell (3); `selftest` (2). |
| Token pull not checked against GitHub or from the Finder-started bundle | accounts, `make build` | Same code path as the GitLab check; run once with a GitHub PAT. |
