.PHONY: check e2e e2e-app fmt dev build sign-setup release

check:
	cd src-tauri && cargo fmt --check
	cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
	cd src-tauri && cargo test
	bun run check
	bun run test

# The real UI in WebKit against the real commands over the e2e bridge, on temp repos.
e2e:
	bunx playwright install webkit
	bun run e2e

fmt:
	cd src-tauri && cargo fmt

dev:
	bun tauri dev

build:
	bun tauri build

# One-time: a stable local code-signing identity, so Keychain access survives rebuilds.
sign-setup:
	./scripts/sign-setup.sh

# Bundled app's backend self-test, launched through LaunchServices like Finder does.
e2e-app:
	./scripts/e2e-app.sh

# Tags v<version from tauri.conf.json>; CI builds every platform and publishes the release.
release:
	./scripts/release.sh
