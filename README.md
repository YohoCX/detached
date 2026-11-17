# Detached

A multi-repo git client for macOS, Windows and Linux with the JetBrains git features and keymap,
native (GPU-drawn, no web view), with AI agents beside your repos: one sidebar for every repo under
your folders with fetch and pull all, branches and smart checkout, diffs, file browsing and find in
files at any ref, the git log, merge requests and pipelines for GitLab and GitHub (including
self-hosted GitLab and GitHub Enterprise), terminal tabs in the current repo, chats with Claude
Code, and panels of agents that review your branches. Default shortcuts:
[docs/KEYMAP.md](docs/KEYMAP.md); Settings › Keymap lists them all, and `keymap.json` in the config
folder overrides them.

Detached itself does not commit, stage or push your code. It changes a repo only when you ask:
switch, stash and pull; worktrees it creates and removes for you; restoring a chat's turn; and a
panel's fixes, which are committed on a branch of their own in their own worktree and reach your
branch only when you click Merge. Agents in a chat edit files with your permission.

The window is three zones around the open documents, Fleet-style: every app (Repositories, Chats,
Panels, Docs, Processes, Changes, Project, Merge Requests, Log, Pipelines, Terminal, Events) is a
tab you can drag to any zone; drag a zone's edge to resize it, double-click to reset it, drag it
nearly shut to close it. Errors and warnings from everywhere collect in Events. Themes: Fleet and
Classic, or your own JSON in the config folder's `themes`, reloaded as you edit.

Folders belong to workspaces, e.g. "Office" and "Personal"; tick any number of them on in the top
bar, and everything (sidebar, fetch and pull all, quick open, search) covers their repos. The
project tree nests related files under their main file (`a.spec.ts` under `a.ts`, `bun.lock` under
`package.json`); `fileNesting` in `settings.json` adds or replaces rules by main suffix, e.g.
`{".vue": [".spec.ts"]}`, and an empty list drops one.

## Agents

Detached runs the Claude Code you installed: the `claude` command on your PATH, or a path set in
Settings › Agents, version 2.1.322 or later, on your own login and plan. It ships no copy of Claude
Code, never reads your login, and does not change what Claude Code sends. Settings › Agents runs
Anthropic's installer and logs you in from a terminal tab. Detached is an independent project, not
made or endorsed by Anthropic.

- **Chats** on a repo or a whole folder, with the model, effort and permission mode you pick. Each
  tool call that needs permission asks you; "Always" is kept by Detached for that exact call, never
  written into the repo's settings. Each turn's changes are snapshotted, so you can restore them.
  The bar shows what is left of your plan's 5-hour and weekly limits. Past conversations, Claude
  Code's own from a terminal included, can be resumed, archived or deleted. Images in an agent's
  reply show as links and never load by themselves.
- **Trust.** A repo's own agent settings (hooks, MCP servers, permissions, skills, commands) load
  only once you accept them, and a change to them asks again.
  Claude Code's Read tool and a plain `cat`, `head`, `tail` or `less` of a `.env` file are refused;
  other commands and scripts are not, so a `.env` is not safe from an agent that runs commands
  without asking.
- **Panels** put several agents on one subject: a review or a pre-merge gate of a branch, triage of
  a merge request or an issue, a review of a plan or any document. On a question, a dialectic has
  two agents argue opposite sides and rebut each other, and a judge weighs them; a brainstorm
  collects ideas, groups them and ranks them by value and cost. Each finding is checked by another
  agent before you see it; you Fix, Dismiss or Defer it. A Fix runs in a worktree on its own branch
  and is checked again. A subject you have not marked trusted, such as a merge request from a fork,
  gets a sandboxed, read-only review and no Fix. Stats shows how each role, lens and model did, and
  past revmux rounds can be imported. Panels run on macOS and Linux; Windows is not done yet.
- **Agents read the client, not pasted text:** branches, merge requests, pipelines, docs and
  processes come to them through a read-only tool. Chats you put in one group can message each
  other.
- **Skills** from `.claude/skills`, `~/.claude/skills`, `~/.agents/skills` and the repo's own store
  reach every chat; Settings › Skills lists them.
- **Docs.** The Docs app reads a repo's `docs/INDEX.md`, its plans and decisions, and marks a doc
  stale when the code it covers changed after it. Agents look up docs the same way, and a turn that
  changed covered code says which docs may need an update.
- **Processes.** Define a dev server or any command once and run it from the Processes app, with an
  env profile whose values are encrypted under a key in the system's credential store and blanked
  out of the log. Each definition says whether agents can see its log or also start it.
- **Worktrees** from Settings › Worktrees, with a recipe of files to copy and an install command.
- **Sharing.** Skills, panel modes, recipes and process definitions are kept in the repo's git
  directory, or shared with your team in `.detached/` (Settings › Sharing).
- **`detached-cli`** (Settings › Command line installs it on PATH) lets a script in a Detached
  terminal tab ask the client what it knows, and run a command with an env profile after you
  confirm.

Codex support is built but stays off in this release.

## Install

macOS 15 or later, on Apple silicon or Intel:

```sh
curl -fsSL https://github.com/YohoCX/detached/releases/latest/download/install.sh | sh
```

Installs the package for the Mac's chip, `Detached-macos-arm64.zip` or `Detached-macos-x64.zip`, as
`/Applications/Detached.app` (`DETACHED_DIR=~/Applications` to change). The app is signed
with a self-signed identity and not notarized, so a copy downloaded in a browser is blocked by
Gatekeeper; the script avoids that. Account tokens are kept in the macOS Keychain. Older Macs stay
on v0.3.0:

```sh
curl -fsSL https://github.com/YohoCX/detached/releases/download/v0.3.0/install.sh |
  DETACHED_URL=https://github.com/YohoCX/detached/releases/download/v0.3.0/Detached-macos.zip sh
```

Windows 10+ and Linux (x86-64, with a Vulkan or GL driver) packages are on the
[releases page](https://github.com/YohoCX/detached/releases/latest): `Detached-windows-x64-setup.exe`
(unsigned, so SmartScreen asks once), `Detached-linux-x86_64.AppImage` and
`Detached-linux-amd64.deb`. They are built but not yet tested by hand. Tokens go to the Windows
Credential Manager or the Secret Service (GNOME Keyring, KWallet).

Chats and panels need Claude Code, which Settings › Agents installs and logs in to; everything else
works without it. The agents have been used by hand on macOS only; on Windows and Linux they have
passed the automated tests alone.

The app looks for a new release at launch and daily and offers it in the status bar, which opens
the Updates page of Settings: there it downloads with its progress, its signature is checked, and
it installs when you click Restart now, which first names the chats and shells a restart ends. A
deb is updated by installing the new `.deb`.

## License

[GPL-3.0-or-later](LICENSE). The license does not cover the Detached name and icon; forks should use their own.

## Development

Needs Rust 1.90+, git and curl; on macOS the Xcode command line tools, on Linux the packages
`.github/workflows/check.yml` installs. The terminal's emulator, libghostty-vt 1.0, is fetched by
the build: prebuilt on macOS, built from Ghostty's source with [Zig](https://ziglang.org) 0.17 on
`PATH` on Linux and Windows (a few minutes, once per target directory; `GHOSTTY_VT_DIR` keeps it
in a directory of your choice).

```sh
make app         # the app at release speed, on your real config directory
make check       # fmt, clippy, cargo test (core, unit and headless UI tests)
make ui-test     # the UI tests only
make smoke       # boots, draws one frame and exits, on a throwaway config directory
make sign-setup  # once: the "Detached Dev" signing identity that make build uses
make build       # the packages for this OS under target/package
make e2e-app     # packages Detached.app, starts it like Finder does on a temp config directory,
                 # checks PATH, node and hooks
make agent-test  # your `claude` on a fake Messages API, no model calls; needs `claude`
make release     # tags v<version in Cargo.toml>; CI builds every platform and publishes it
```

`DETACHED_CONFIG_DIR` points the app at another config directory; the tests use temp ones.

`.github/workflows/release.yml` signs the macOS build with the same "Detached Dev" identity, or
Keychain access breaks on update. Once, export it from Keychain Access (My Certificates, Detached
Dev, Export, `.p12` with an empty password), store it and delete the file:

```sh
base64 -i Detached.p12 | gh secret set MACOS_CERTIFICATE
```

Each package, and the `latest.json` the app checks for updates, is also signed with a minisign
key, whose public half is `PUBLIC_KEY` in `crates/core/src/update.rs`; the release job refuses to
publish without both. The key is a secret of the `release` environment: once, create it under
Settings, Environments, with yourself as required reviewer and only tags `v*` allowed.

With a key already in use, move it there; installed copies reject a new one:

```sh
gh secret set UPDATE_SIGNING_KEY --env release < minisign.key
gh secret delete UPDATE_SIGNING_KEY             # the repository copy, which any workflow can read
```

Only on first setup, make the key (`brew install minisign`):

```sh
minisign -G -W -p minisign.pub -s minisign.key  # -W: no password, so CI can sign
gh secret set UPDATE_SIGNING_KEY --env release < minisign.key
sed -n 2p minisign.pub                          # the PUBLIC_KEY value; commit it
```

Then keep `minisign.key` in a password manager and delete the file: an installed copy only takes
updates signed with the key it was built with.

Running the workflow by hand (`gh workflow run release.yml`) only builds, with the files as run artifacts.

A panic leaves a report in `crashes/` of the config directory (on macOS `~/Library/Application
Support/com.w-devs.detached`). Release builds carry no symbols, so its backtrace is addresses; on
macOS the release's `Detached-macos-<arch>-dSYM.zip` names them, with the report's load address:

```sh
ditto -x -k Detached-macos-arm64-dSYM.zip .
atos -o detached.dSYM -arch arm64 -l <load address> <addresses> | rustfilt  # cargo install rustfilt
```

`crates/core` is the backend (git, forges, accounts, search, settings, agents' processes, Claude
Code's client), `crates/acp` the client of agents on the Agent Client Protocol, `crates/kernel` the
app's live state, `crates/app` the GPUI client and `crates/forge-fake` a local GitLab and GitHub
Enterprise for the UI tests in `crates/app/tests`, which drive real views headless against temp
repos. The agent tests run scripted fakes of Claude Code and of an ACP agent.

Design and history: [PLAN-NATIVE.md](PLAN-NATIVE.md) (the native client), [PLAN-FOUNDATION.md](PLAN-FOUNDATION.md)
(the kernel), [PLAN-AGENT-HOST.md](PLAN-AGENT-HOST.md) and [PLAN-PANELS.md](PLAN-PANELS.md) (the
agents), [docs/decisions/](docs/decisions/), [docs/BACKLOG.md](docs/BACKLOG.md),
[docs/TECHDEBT.md](docs/TECHDEBT.md). Release notes: [docs/releases/](docs/releases/).
