# Backlog

Features after milestone 1, newest first. Milestone 2 scope is in [PLAN-M2.md](../PLAN-M2.md) "Scope".

| Feature | Notes |
| --- | --- |
| Native Windows sandbox for panels | Isolate an untrusted subject's participants on Windows without WSL2: an AppContainer or a restricted token, with a proxy allowlist for egress. Or Claude Code's own native sandbox: 2.1.322 has one behind a feature gate, which needs `claude sandbox install` (admin) to provision a sandbox user and network filters. Today they run in WSL2 or unisolated after a consent ([PLAN-PANELS.md](../PLAN-PANELS.md) §3). |
| Theme editor | In Settings with a live preview, saving to a theme file; one mutable preview slot, so a `Theme` does not leak per edit. Themes come from JSON files first ([PLAN-EXTENSIBLE.md](../PLAN-EXTENSIBLE.md) step 2). |
| Icon packs | UI and file type icons replaceable from the config directory; needs a licensed or own set of file type icons first. |
| Density and a spacing scale | The ~530 `px()` in views onto tokens; many are geometry rather than spacing, such as the 90 px traffic light inset. |
| Plugin catalog | Plugins in GitHub Releases, a signed JSON index with a sequence number in a `detached-plugins` repo, moderated by PR, its own minisign key, a revocation list. The index URL can move to a VPS later. |
| Plugin install with account tokens | From a private repo with a connected account's token. |
| Process plugins and host widgets | JSON over stdio with a version handshake, host widgets `chat`, `diff`, `markdown`, `list`, `tree`, `table`. Extract from chat's private protocol when a second real consumer appears. |
| Local env store | Per git repo with profiles and worktree overrides, encrypted, master key in the OS store; injected on request, `.env` on demand. `detached env run` needs a console binary: the app is a GUI subsystem program on Windows, with no subcommands and not on PATH. A store beside envault or replacing it is undecided. |
| Auto-fetch | Fetches the active workspaces' repos every N seconds, 300 by default, 0 turns it off; a setting. Quiet: no top-bar progress and no toast per run, failures show on the repo. Skips a run while a fetch or pull is going; the branch widget's ahead/behind and the MR chip refresh after it. |
