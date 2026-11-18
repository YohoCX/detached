.PHONY: check app ui-test agent-test codex-live agent-login warm ramdisk smoke e2e-app fmt build sign-setup release adapters-sign

# The tests' sandboxes go on the RAM disk when `make ramdisk` made one: a quarter faster. The build
# runs first without it, so the linker's temp files stay off it.
TESTS = cargo test --no-run $(1) && $(if $(wildcard /Volumes/DetachedTests),TMPDIR=/Volumes/DetachedTests) cargo test $(1)

check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
# The product as shipped: no tests, no test-only features.
	cargo clippy --workspace -- -D warnings
	$(call TESTS,--workspace --no-fail-fast)

# Release speed, on the real config directory.
app:
	cargo run -p detached-app --profile app

# One test file with t=<name>, as in `make ui-test t=repos`.
ui-test:
	$(call TESTS,-p detached-app $(if $(t),--test $(t)))

# The user's `claude` on a scripted fake Messages API, in temp homes; not in check, since it needs
# `claude` and runs a 3-minute command. A missing one fails.
agent-test:
	cargo test -p detached-kernel --lib session::tests::real -- --ignored
	cargo test -p detached-kernel --lib panel::tests::real -- --ignored

# The owner's, never CI's: Codex's live rows through the bun build of its adapter, on his own `codex`
# and login. Three short model turns; each checks his config.toml unchanged.
codex-live:
	./scripts/adapter-build.sh codex-acp
	cargo test -p detached-acp --lib codex_live -- --ignored --nocapture --test-threads=1

# The owner's, never CI's or an agent's: `detached --session` on his own `claude` and login, in a
# temp repo and config directory kept between runs; prompt=<text> (else stdin), args=--past,
# --resume <id> or --delete <id>. Between turns stdin takes /mode, /model and /stop <task id>.
LOGIN = $(patsubst %/,%,$(or $(TMPDIR),/tmp))/detached-agent-login
agent-login:
	test -d "$(LOGIN)/repo" || (git init -q "$(LOGIN)/repo" && \
		git -C "$(LOGIN)/repo" -c user.name=d -c user.email=d@d commit -q --allow-empty -m init)
	DETACHED_CONFIG_DIR="$(LOGIN)/config" cargo run -p detached-app -- \
		--session "$(LOGIN)/repo" $(args) $(if $(prompt),"$(prompt)")

# In a new worktree: its build starts from the worktree that built last.
warm:
	./scripts/warm.sh

# Once per boot, macOS: a RAM disk the tests' sandboxes go on.
ramdisk:
	./scripts/ramdisk.sh

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

# The fallback to adapters.yml's signing: signs adapters/adapters.json with the catalog's key, made
# outside the repo on first use; entry=<entry.json> from an adapters-* release adds that entry first.
adapters-sign:
	./scripts/adapters-sign.sh $(entry)

# Tags v<version from Cargo.toml>; CI builds every platform and publishes the release.
release:
	./scripts/release.sh
