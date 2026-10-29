.PHONY: check fmt dev build sign-setup e2e-app

check:
	cd src-tauri && cargo fmt --check
	cd src-tauri && cargo clippy --all-targets -- -D warnings
	cd src-tauri && cargo test
	pnpm check
	pnpm test

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
