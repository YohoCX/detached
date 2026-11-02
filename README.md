# Detached

A read-only, multi-repo git client for macOS with the JetBrains git features and keymap.
Tauri 2 (Rust) + Svelte 5 + Monaco. Plans: [PLAN.md](PLAN.md) (milestone 1), [PLAN-M2.md](PLAN-M2.md) (milestone 2). Keymap: [docs/KEYMAP.md](docs/KEYMAP.md).

## Install

```sh
curl -fsSL https://github.com/YohoCX/detached/releases/latest/download/install.sh | sh
```

Installs `/Applications/Detached.app` (`DETACHED_DIR=~/Applications` to change); run it again to update.
The app is signed with a self-signed identity and not notarized, so a copy downloaded in a browser
is blocked by Gatekeeper; the script avoids that.

## Setup

Needs Rust 1.90+, Node 24, pnpm 10 and Xcode command line tools.

```sh
pnpm install
make dev        # run the app
make check      # fmt, clippy, cargo test, svelte-check, vitest
make e2e        # Playwright in WebKit: the real UI against the real commands and git (~1 min)
make e2e-app    # builds Detached.app, starts it like Finder does, checks PATH, node and hooks
make build      # Detached.app under src-tauri/target/release/bundle
```

The command contract between UI and backend is `src/lib/api.ts` and `src-tauri/src/types.rs`;
change both together. `make e2e` replaces only the Tauri IPC transport: `e2e/shim.ts` forwards
`invoke` to `src-tauri/src/bin/e2e-bridge.rs` (cargo feature `e2e`), which calls the same code.
`e2e/forge.ts` fakes GitLab (`/gitlab`) and GitHub Enterprise (`/ghe`) for the MR and pipeline
specs; `openForge()` in `e2e/fixture.ts` seeds it.

## License

[GPL-3.0-or-later](LICENSE). The license does not cover the Detached name and icon; forks should use their own.
