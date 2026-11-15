.PHONY: check app ui-test agent-test warm ramdisk smoke sidecar e2e-app fmt build sign-setup release adapters-sign

# The tests' sandboxes go on the RAM disk when `make ramdisk` made one: a quarter faster. The build
# runs first without it, so the linker's temp files stay off it.
TESTS = cargo test --no-run $(1) && $(if $(wildcard /Volumes/DetachedTests),TMPDIR=/Volumes/DetachedTests) cargo test $(1)

check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
# Core alone with e2e and testutil on, so no other crate is built with them.
	cargo clippy -p detached-core --all-targets --all-features -- -D warnings
	$(call TESTS,--workspace)

# Release speed, on the real config directory.
app:
	cargo run -p detached-app --profile app

# One test file with t=<name>, as in `make ui-test t=repos`.
ui-test:
	$(call TESTS,-p detached-app $(if $(t),--test $(t)))

# The bun-built Claude adapter and the user's `claude` on a scripted fake Messages API, in temp homes;
# not in check, since it needs both and runs a 3-minute command. A missing one fails.
agent-test:
	./scripts/adapter-build.sh claude-agent-acp
	cargo test -p detached-kernel --lib session::tests::real -- --ignored
	cargo test -p detached-acp -- --ignored

# In a new worktree: its build starts from the worktree that built last.
warm:
	./scripts/warm.sh

# Once per boot, macOS: a RAM disk the tests' sandboxes go on.
ramdisk:
	./scripts/ramdisk.sh

# The Claude sidecar, into plugins/claude of the config directory (the owner's unless DETACHED_CONFIG_DIR is set).
sidecar:
	./scripts/sidecar.sh

# Boots, draws one frame and exits, on a throwaway config directory.
smoke:
	DETACHED_CONFIG_DIR="$$(mktemp -d)" cargo run -p detached-app -- --smoke

fmt:
	cargo fmt --all

# The native packages for this OS in target/package: Detached.app and its zip per arch, NSIS, or deb and AppImage.
build:
	./scripts/package.sh

# One-time: a stable local code-signing identity, so Keychain access survives rebuilds.
sign-setup:
	./scripts/sign-setup.sh

# Bundled app's backend self-test, launched through LaunchServices like Finder does.
e2e-app:
	./scripts/e2e-app.sh

# Signs adapters/adapters.json with the catalog's key, made outside the repo on first use;
# entry=<entry.json> from an adapters-* release adds that entry first.
adapters-sign:
	./scripts/adapters-sign.sh $(entry)

# Tags v<version from Cargo.toml>; CI builds every platform and publishes the release.
release:
	./scripts/release.sh
