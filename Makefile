.PHONY: check e2e e2e-app fmt dev build sign-setup release

check:
	cd src-tauri && cargo fmt --check
	cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
	cd src-tauri && cargo test
	pnpm check
	pnpm test

# The real UI in WebKit against the real commands over the e2e bridge, on temp repos.
e2e:
	pnpm exec playwright install webkit
	pnpm e2e

fmt:
	cd src-tauri && cargo fmt

dev:
	pnpm tauri dev

build:
	pnpm tauri build

# One-time: a stable local code-signing identity, so Keychain access survives rebuilds.
sign-setup:
	./scripts/sign-setup.sh

# Bundled app's backend self-test, launched through LaunchServices like Finder does.
e2e-app:
	./scripts/e2e-app.sh

# Universal build published as GitHub release v<version from tauri.conf.json>, with install.sh.
release:
	./scripts/release.sh
