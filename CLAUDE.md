# Detached

Read-only multi-repo git client. Two clients share one backend while the native port runs:
- `crates/core`: the backend (git, forge, accounts, search, settings), blocking Rust, no UI types.
- `crates/app`: the native client on GPUI (gpui-kit). The future; see [PLAN-NATIVE.md](PLAN-NATIVE.md).
- `src-tauri` + `src/`: the Tauri/Svelte client, kept working until PLAN-NATIVE step 2 removes it.

## Commands

```sh
make check    # the merge gate: fmt, clippy -D warnings and cargo test over the whole workspace
              # (core, ported unit tests, UI tests), plus svelte-check and bun test until V6
make app      # the native client, against the dev.wdevs.detached.native config copy
make ui-test  # the native UI tests only
make smoke    # build and run `--smoke`
make build    # the native packages for this OS under target/package (scripts/package.sh)
make e2e-app  # macOS: packages Detached.app and runs its --self-test through LaunchServices
make e2e      # Playwright on the Tauri client (until removed)
make dev      # the Tauri client
make build-tauri  # the Tauri bundles (until removed)
make sign-setup  # once, macOS: the "Detached Dev" signing identity that make build uses
make release  # tags v<version in Cargo.toml>; release.yml builds, signs and publishes it
```

Keep this list in sync with the Makefile.

## Code style

- Simple, clean, short. If it can be shorter without losing clarity, make it shorter. Fewer lines,
  fewer abstractions, fewer helpers; no speculative code, no options nobody asked for.
- Comments only where code cannot explain itself: a non-obvious reason, a workaround, a trap. One
  short line. No comments that restate code, narrate history, cite tickets or reviews, or shout in
  capitals. History belongs in the commit message.
- This beats matching the surrounding style: in over-commented code, write the new code short and
  trim comments in what you touch.
- Every subagent prompt that writes code carries this section.

## Rust

- `cargo fmt`, `clippy --all-targets -D warnings`, edition 2024.
- Errors: `core::error::Result`; UI code shows them through notices or the status message, never
  `unwrap`/`expect` outside tests and true invariants.
- Core stays blocking and UI-free: no gpui, no tauri, no async runtime in `crates/core`.
- Never call core on the UI thread or on GPUI's executors: go through `blocking::run`/`stream`.
- Long ops take an op id and honor cancel; streams are batched per frame on the UI side.
- Platform code goes in `core::os` or behind `cfg` in the one place that needs it.
- The `e2e` feature of core is test-only: never a normal dependency of `crates/app`.

## Native UI

- Views read the `Store` and their own state; they do not cache what the store already holds.
- Colors, radii, spacing and fonts come from `ui::theme` tokens, never literals in views.
- Shared widgets live in `ui/`; a vertical adds one there in its own commit.
- Keys: every action has an id in `actions.rs`; default chords in `keymap.rs` match
  docs/KEYMAP.md; text fields and the editor keep their keys through binding contexts.
- UI state that persists goes through `ui_state` with the owner's key prefix (`sidebar.`, `size.`,
  `tree.`, `viewer.`, ...).
- gpui-kit and gpui-pre are pinned exactly (`=`). An upgrade is its own commit: bump, fix breakage,
  `make check`, a short note in the message of what broke. Do not patch the kit; work around in our
  code and link the upstream issue in a one-line comment.

## Tests

- Core: unit tests next to the code, temp repos through `testutil::Sandbox`, no network except the
  local fakes.
- Native UI: `crates/app/tests/<vertical>.rs` with the kit's headless harness, real git, the forge
  fake. A behavior ported from the parity docs gets a test unless the doc marks it untested and a
  test would be expensive; say which in the commit.
- Logic ported from `src/lib/*.ts` brings its `*.test.ts` cases along as Rust unit tests.
- No sleeps for timing; wait on state with the fixtures' `settle`/`wait_until`.
- Tests run on macOS, Windows and Linux CI. A test that can only work on Unix is `#[cfg(unix)]` and
  listed in docs/TECHDEBT.md.

## Parallel work (PLAN-NATIVE verticals)

- One vertical per worktree and branch (`native/v1-repos`, ...). Touch only your directory, your
  test file, and the append-only registration points (`actions.rs`, `keymap.rs`, `shell` slot
  constructors, `ui/` additions). A vertical's `api.rs` is a contract: its owner fills in the
  bodies; others only add to it, as below. Need a change elsewhere? Ask the foundation owner, or make it a
  separate commit with a reason.
- PLAN-NATIVE "Contracts": changing an existing signature takes a plan edit agreed with the owner;
  adding a missing piece is your own small commit on main (stub in the provider's `api.rs` or the
  store, plus its line in the plan). "Parity owners" says which docs/native sections are yours.
- Two clients share the machine: run the native one through `make app`, never against the real
  config directory, until V6.
- Before merge: rebase on main, `make check` green, parity sections ticked in docs/native, a revmux
  round (claude-only profile, opus high) without blocking findings, the owner's check on macOS.

## Docs

- Plans (`PLAN*.md`) keep a dated status line; update it when a step closes.
- docs/KEYMAP.md is the keymap contract; docs/BACKLOG.md takes features, docs/TECHDEBT.md bugs and
  shortcuts found along the way.
- README describes what a user installs today.

## Git

- Commit and push only when the owner asks (vertical agents commit on their own branch when the
  owner started them for that).
- Messages: one summary line in the style of `git log` (what changed, comma-separated parts), body
  only for the why.
- Never add Co-Authored-By or "Generated with" lines for Claude or Anthropic.
