# Detached

A read-only, multi-repo git client for macOS, Windows and Linux with the JetBrains git features
and keymap, native (GPU-drawn, no web view): one
sidebar for every repo under your folders with fetch and pull all, branches and smart checkout,
diffs, file browsing and find in files at any ref, the git log, and merge requests and pipelines
for GitLab and GitHub (including self-hosted GitLab and GitHub Enterprise). It does not commit,
stage or push code. Default shortcuts: [docs/KEYMAP.md](docs/KEYMAP.md).

Folders belong to workspaces, e.g. "Office" and "Personal"; tick any number of them on in the top
bar, and everything (sidebar, fetch and pull all, quick open, search) covers their repos. The
project tree nests related files under their main file (`a.spec.ts` under `a.ts`, `bun.lock` under
`package.json`); `fileNesting` in `settings.json` adds or replaces rules by main suffix, e.g.
`{".vue": [".spec.ts"]}`, and an empty list drops one.

## Install

macOS 15 or later (a universal app for Apple silicon and Intel):

```sh
curl -fsSL https://github.com/YohoCX/detached/releases/latest/download/install.sh | sh
```

Installs `/Applications/Detached.app` (`DETACHED_DIR=~/Applications` to change). The app is signed
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

The app looks for a new release at launch and daily and offers it in the status bar; a click
downloads it, checks its signature, installs it and restarts. A deb is updated by installing the
new `.deb`.

## License

[GPL-3.0-or-later](LICENSE). The license does not cover the Detached name and icon; forks should use their own.

## Development

Needs Rust 1.90+ and git; on macOS the Xcode command line tools, on Linux the packages
`.github/workflows/check.yml` installs.

```sh
make app         # the app at release speed, on your real config directory
make check       # fmt, clippy, cargo test (core, unit and headless UI tests)
make ui-test     # the UI tests only
make smoke       # boots, draws one frame and exits, on a throwaway config directory
make sign-setup  # once: the "Detached Dev" signing identity that make build uses
make build       # the packages for this OS under target/package
make e2e-app     # packages Detached.app, starts it like Finder does, checks PATH, node and hooks
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
key, whose public half is `PUBLIC_KEY` in `crates/app/src/update.rs`; the release job refuses to
publish without both. Once
(`brew install minisign`):

```sh
minisign -G -W -p minisign.pub -s minisign.key  # -W: no password, so CI can sign
gh secret set UPDATE_SIGNING_KEY < minisign.key
sed -n 2p minisign.pub                          # the PUBLIC_KEY value; commit it
```

Then keep `minisign.key` in a password manager and delete the file: an installed copy only takes
updates signed with the key it was built with.

Running the workflow by hand (`gh workflow run release.yml`) only builds, with the files as run artifacts.

`crates/core` is the backend (git, forges, accounts, search, settings), `crates/app` the GPUI
client and `crates/forge-fake` a local GitLab and GitHub Enterprise for the UI tests in
`crates/app/tests`, which drive real views headless against temp repos.

Design and history: [PLAN.md](PLAN.md) (milestone 1), [PLAN-M2.md](PLAN-M2.md) (milestone 2),
[PLAN-NATIVE.md](PLAN-NATIVE.md) (the native client), [docs/BACKLOG.md](docs/BACKLOG.md),
[docs/TECHDEBT.md](docs/TECHDEBT.md).
