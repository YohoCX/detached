# Workbench

A read-only, multi-repo git client for macOS with the JetBrains git features and keymap.
Tauri 2 (Rust) + Svelte 5 + Monaco. Plan: [PLAN.md](PLAN.md). Keymap: [docs/KEYMAP.md](docs/KEYMAP.md).

## Setup

Needs Rust 1.90+, Node 24, pnpm 10 and Xcode command line tools.

```sh
pnpm install
make dev        # run the app
make check      # fmt, clippy, cargo test, svelte-check, vitest
make build      # Workbench.app under src-tauri/target/release/bundle
```

The command contract between UI and backend is `src/lib/api.ts` and `src-tauri/src/types.rs`;
change both together.
