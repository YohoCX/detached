# Tech debt

Bugs and shortcuts found after milestone 1, newest first. Features go to [BACKLOG.md](BACKLOG.md).

| Issue | Where | Notes |
| --- | --- | --- |
| A long markdown file reloaded with new text blocks a frame | V2 `files::markdown`, the kit's `TextView` | The kit lays out a whole text in one frame (kit #3695): the spike's 2,000 lines take about 300 ms. A first show goes in by 4 KB pieces, one a frame (max frame 20–40 ms); a reload with changed text replaces it at once, since feeding it again would scroll back to the top. |
| No space under a markdown file's h1 and h2 rules | V2 `files::markdown` | The rule is the heading's bottom border; the kit's list rows ignore a heading's margin and give it no wrapper, so the next block starts right under the rule (the Tauri page left 12–14 px). |
| Viewer line numbers are not Monaco's colors | V2 `files::code`, `ui::theme` | The kit's `Input` sets the editor style from the global theme's muted and foreground colors on every render and its setter is crate-private, so the line numbers cannot take Monaco's own colors without changing every muted text. |
| A diff reloading with new text loses its scroll | V2 `files::diff` | `DiffState::set_files` resets the viewport and the kit cannot restore one (kit #3670), so a file changed on disk scrolls back to the cursor on the next status refresh; an unchanged one is not reloaded. |
| Nested repo names hold `\` on Windows | `git::discovery` | `group\api` in the sidebar; the UI fixture test checks names and the worktree's main repo on Unix only (git there may also write a worktree's link with another spelling of the temp path). |
| Native client draws in system fonts | `crates/app` `ui::theme` | The system UI font and the kit's mono font stand in for Manrope and JetBrains Mono (shell.md §5). Bundling them takes their TTF files in the repo (the fontsource packages hold woff2 only) and `add_fonts` at boot. |
| Worktree walks return `\` paths on Windows | `git::files::paths`/`list_files`, `search::find` | Git's own paths use `/`; quick open, the tree and find in files would mix both. Their tests are `#[cfg(unix)]` until fixed: `files` worktree listing, `find` literal/regex and glob. |
| Core tests that run on Unix only (`#[cfg(unix)]`) | crates/core | Shell stand-ins, process groups or `/` in expected paths. `git::checkout`: the 5 hook tests; `git::pull`: post-merge hook; `git::cred_tests`: all 3 (hooks); `git::runner`: cancel and silence limit (fake ssh, `ps`, `killpg`); `git::files`: fake zed; `git::remotes`: remote URLs; `git::discovery`: walk and names; `settings`: workspaces; `accounts`: credential helper script; `forge`: fake `ssh -G`; `env`: zsh login shell (3); `selftest` (2). |
| Token pull not checked against GitHub or from the Finder-started bundle | accounts, `make build` | Same code path as the GitLab check; run once with a GitHub PAT. |
