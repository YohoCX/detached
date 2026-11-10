.PHONY: check app ui-test warm ramdisk smoke sidecar e2e-app fmt build sign-setup release

# The tests' sandboxes go on the RAM disk when `make ramdisk` made one: a quarter faster.
TESTS := $(if $(wildcard /Volumes/DetachedTests),TMPDIR=/Volumes/DetachedTests) cargo test

check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
# Core alone with e2e and testutil on, so no other crate is built with them.
	cargo clippy -p detached-core --all-targets --all-features -- -D warnings
	$(TESTS) --workspace

# Release speed, on the real config directory.
app:
	cargo run -p detached-app --profile app

# One test file with t=<name>, as in `make ui-test t=repos`.
ui-test:
	$(TESTS) -p detached-app $(if $(t),--test $(t))

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

# The native packages for this OS in target/package: Detached.app and its zip, NSIS, or deb and AppImage.
build:
	./scripts/package.sh

# One-time: a stable local code-signing identity, so Keychain access survives rebuilds.
sign-setup:
	./scripts/sign-setup.sh

# Bundled app's backend self-test, launched through LaunchServices like Finder does.
e2e-app:
	./scripts/e2e-app.sh

# Tags v<version from Cargo.toml>; CI builds every platform and publishes the release.
release:
	./scripts/release.sh
