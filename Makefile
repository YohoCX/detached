.PHONY: check app ui-test smoke e2e-app fmt build sign-setup release

check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
# Core alone with e2e and testutil on, so no other crate is built with them.
	cargo clippy -p detached-core --all-targets --all-features -- -D warnings
	cargo test --workspace

# The native client on its own copy of the config directory, made from the real one on first run.
app:
	@case "$$(uname -s)" in \
	  Darwin) base="$$HOME/Library/Application Support" ;; \
	  MINGW*|MSYS*|CYGWIN*) base="$$APPDATA" ;; \
	  *) base="$${XDG_CONFIG_HOME:-$$HOME/.config}" ;; \
	esac; \
	dir="$$base/dev.wdevs.detached"; \
	if [ ! -e "$$dir.native" ] && [ -d "$$dir" ]; then cp -R "$$dir" "$$dir.native"; fi; \
	DETACHED_CONFIG_DIR="$$dir.native" cargo run -p detached-app --profile app

ui-test:
	cargo test -p detached-app

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
