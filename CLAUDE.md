# Detached

Multi-repo git client, native on GPUI. It writes to a repo only through switch, stash, pull, and the
chat's turn snapshots (private refs) and restores:
- `crates/core`: the backend (git, forge, accounts, search, settings, the agent's protocol and trust),
  blocking Rust, no UI types.
- `crates/kernel`: the app's live state and its exchange as GPUI entities (`blocking`, `ui_state`,
  `Activity`, `Doc`, the `Settings` snapshot), on gpui-pre alone; it draws nothing. See [PLAN-FOUNDATION.md](PLAN-FOUNDATION.md).
- `crates/app`: the client on GPUI (gpui-kit), binary `detached`; see [PLAN-NATIVE.md](PLAN-NATIVE.md).
- `crates/forge-fake`: a local GitLab and GitHub Enterprise for the tests.
- `sidecars/claude`: the chat's process on the Agent SDK, in TypeScript, kept out of the release packages.

## Commands

```sh
make check    # the merge gate: fmt, clippy -D warnings and cargo test over the whole workspace
              # (core, unit tests, UI tests)
make app      # the client at release speed, on the real config directory (the owner's only)
make ui-test  # the UI tests only; t=repos runs tests/repos.rs alone
make warm     # macOS, in a new worktree: clones target/debug of the worktree that built last
make ramdisk  # macOS, once per boot: the tests' sandboxes go on a RAM disk, a quarter faster
make smoke    # build and run `--smoke` on a temp config directory
make sidecar  # build the Claude sidecar (needs bun) and install it in the config directory
make build    # the packages for this OS under target/package (scripts/package.sh)
make e2e-app  # macOS: packages Detached.app and runs its --self-test through LaunchServices
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
- Core stays blocking and UI-free: no gpui, no async runtime in `crates/core`.
- Never call core on the UI thread or on GPUI's executors: go through `kernel::blocking::run`/`stream`.
- Long ops take an op id and honor cancel; streams are batched per frame on the UI side.
- The kernel draws nothing and imports no kit component: `crates/kernel/clippy.toml` bans GPUI's
  elements, `Render` and `Window`, and `forbid` at its root keeps an `#[allow]` from lifting that.
- Core owns `settings.json`: `AppState::update` is its only writer, and the kernel's `Settings` is a
  read-only snapshot that a `blocking::run` reloads when core's revision moved. A module keeps its
  own top-level key, a block (`block`, `set_block`); keys a build does not know survive its saves.
- Platform code goes in `core::os` or behind `cfg` in the one place that needs it.
- The `e2e` feature of core is test-only: never a normal dependency of `crates/app`.

## Native UI

- Views read the `Store`, the kernel's entities (`Activity`, `Settings`) and their own state; they do not cache
  what those already hold.
- Colors, radii, spacing and fonts come from `ui::theme` tokens, never literals in views.
- Shared widgets live in `ui/`; a vertical adds one there in its own commit.
- Keys: every action has an id in `actions.rs`; default chords in `keymap.rs` match
  docs/KEYMAP.md; text fields and the editor keep their keys through binding contexts.
- UI state that persists goes through `kernel::ui_state` with the owner's key prefix (`sidebar.`, `size.`,
  `tree.`, `viewer.`, ...).
- gpui-kit and gpui-pre are pinned exactly (`=`). An upgrade is its own commit: bump, fix breakage,
  `make check`, a short note in the message of what broke. Do not patch the kit; work around in our
  code and link the upstream issue in a one-line comment.

## Tests

- Core: unit tests next to the code, temp repos through `testutil::Sandbox`, no network except the
  local fakes.
- Kernel: unit tests next to the code, in GPUI's test context without a window, on GPUI's clock.
- Native UI: `crates/app/tests/<vertical>.rs` with the kit's headless harness, real git, the forge
  fake. A new behavior gets a test unless it would be expensive; say so in the commit.
- Tests and agents never touch the real config directory: `Sandbox` has a temp one and `make smoke`
  sets a temp `DETACHED_CONFIG_DIR`; run the app yourself only that way, never through `make app`.
- No sleeps for timing; wait on state with the fixtures' `settle`/`wait_until`.
- Tests run on macOS, Windows and Linux CI. A test that can only work on Unix is `#[cfg(unix)]` and
  listed in docs/TECHDEBT.md.

## Parallel work (PLAN-NATIVE verticals)

- One vertical per worktree and branch (`native/v1-repos`, ...); `make warm` first. Touch only
  your directory, your test file, and the append-only registration points (`actions.rs`,
  `keymap.rs`, your feature's `register(cx)` with its apps, document kinds, shell slots and Settings pages, its
  line in `lib.rs`'s `register`, `ui/` additions). A vertical's `api.rs` is a contract: its owner fills in the
  bodies; others only add to it, as below. Need a change elsewhere? Ask the foundation owner, or make it a
  separate commit with a reason.
- PLAN-NATIVE "Contracts": changing an existing signature takes a plan edit agreed with the owner;
  adding a missing piece is your own small commit on main (stub in the provider's `api.rs` or the
  store, plus its line in the plan).
- Before merge: rebase on main, `make check` green, a revmux round (claude-only profile, opus high)
  without blocking findings, the owner's check on macOS.

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
