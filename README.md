# Detached

A read-only, multi-repo git client for macOS with the JetBrains git features and keymap: one
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

```sh
curl -fsSL https://github.com/YohoCX/detached/releases/latest/download/install.sh | sh
```

Installs `/Applications/Detached.app` (`DETACHED_DIR=~/Applications` to change); run it again to update.
The app is signed with a self-signed identity and not notarized, so a copy downloaded in a browser
is blocked by Gatekeeper; the script avoids that. Account tokens are kept in the macOS Keychain.

Windows and Linux builds are on the [releases page](https://github.com/YohoCX/detached/releases/latest):
`Detached-windows-x64-setup.exe` (unsigned, so SmartScreen asks once), `Detached-linux-x86_64.AppImage`
and `Detached-linux-amd64.deb`. They are built but not yet tested by hand. Tokens go to the Windows
Credential Manager or the Secret Service (GNOME Keyring, KWallet).

## License

[GPL-3.0-or-later](LICENSE). The license does not cover the Detached name and icon; forks should use their own.

## Development

Needs Rust 1.90+, Bun 1.3+, Node 24 (Playwright runs on it) and the Xcode command line tools.

```sh
bun install
make dev         # run the app
make check       # fmt, clippy, cargo test, svelte-check, bun test
make e2e         # Playwright in WebKit: the real UI against the real commands and git (~1 min)
make sign-setup  # once: the "Detached Dev" signing identity that make build uses
make build       # the native client's packages for this OS under target/package
make build-tauri # the Tauri bundles under target/release/bundle
make e2e-app     # packages Detached.app, starts it like Finder does, checks PATH, node and hooks
make release     # tags v<version in Cargo.toml>; CI builds every platform and publishes it
```

`.github/workflows/release.yml` signs the macOS build with the same "Detached Dev" identity, or
Keychain access breaks on update. Once, export it from Keychain Access (My Certificates, Detached
Dev, Export, `.p12` with an empty password), store it and delete the file:

```sh
base64 -i Detached.p12 | gh secret set MACOS_CERTIFICATE
```

Running the workflow by hand (`gh workflow run release.yml`) only builds, with the files as run artifacts.

The command contract between UI and backend is `src/lib/api.ts` and `crates/core/src/types.rs`;
change both together. `make e2e` replaces only the Tauri IPC transport: `e2e/shim.ts` forwards
`invoke` to `crates/core/src/bin/e2e-bridge.rs` (cargo feature `e2e`), which calls the same code.
`e2e/forge.ts` fakes GitLab (`/gitlab`) and GitHub Enterprise (`/ghe`) for the MR and pipeline
specs; `openForge()` in `e2e/fixture.ts` seeds it. `E2E_PORT` (default 1520) moves its three ports.

Design and history: [PLAN.md](PLAN.md) (milestone 1), [PLAN-M2.md](PLAN-M2.md) (milestone 2),
[docs/BACKLOG.md](docs/BACKLOG.md), [docs/TECHDEBT.md](docs/TECHDEBT.md).
