#!/usr/bin/env bash
# CI's agent harness on one OS (.github/workflows/agent.yml): H4's tests on the fake ACP agent, then
# `claude` from Anthropic's installer and the real-adapter tests of `make agent-test`. `claude` is
# at the version the adapter's SDK pins. VET=1 runs them on each entry of the committed, signed
# catalog this app accepts instead, each at its min_cli; LATEST=1 takes the newest `claude`.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo test -p detached-acp
cargo test -p detached-kernel --lib session::
cargo test -p detached-core --lib -- proc:: trust::
./scripts/adapter-build.sh claude-agent-acp

# Installs `claude` $1 and puts it first on PATH.
cli() {
  case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*)
      powershell -NoProfile -Command "& ([scriptblock]::Create((irm https://claude.ai/install.ps1))) $1"
      bin=$(cygpath "$USERPROFILE")/.local/bin
      ;;
    *)
      curl -fsSL https://claude.ai/install.sh | bash -s "$1"
      bin=$HOME/.local/bin
      ;;
  esac
  export PATH=$bin:$PATH
  have=$(claude --version)
  echo "claude $have"
  [ "$1" = latest ] || [ "${have%% *}" = "$1" ] || { echo "::error::claude is $have, not $1."; exit 1; }
}
real() {
  cargo test -p detached-kernel --lib session::tests::real -- --ignored
  cargo test -p detached-acp -- --ignored
}

if [ "${VET:-}" != 1 ]; then
  version=$(jq -r .claudeCodeVersion adapters/claude-agent-acp/node_modules/@anthropic-ai/claude-agent-sdk/package.json)
  [ "${LATEST:-}" = 1 ] && version=latest
  cli "$version"
  real
  exit 0
fi

if [ ! -f adapters/adapters.json.minisig ]; then
  echo "::notice::adapters/adapters.json is not signed yet; nothing to vet."
  exit 0
fi
app=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml)
entries=$(jq -r --arg app "$app" 'def v: split(".") | map(tonumber);
  .adapters[] | select(.name == "claude-agent-acp" and (.min_app | v) <= ($app | v)
    and ((.max_app // $app) | v) >= ($app | v)) | "\(.version) \(.min_cli // "")"' adapters/adapters.json)
[ -n "$entries" ] || { echo "::error::The catalog has no claude-agent-acp entry for $app."; exit 1; }
py=python3; command -v python3 > /dev/null || py=python
"$py" -m http.server 8765 --bind 127.0.0.1 --directory adapters > /dev/null 2>&1 &
server=$!
trap 'kill "$server"' EXIT
export DETACHED_AGENT_CATALOG=http://127.0.0.1:8765/adapters.json
if [ "${LATEST:-}" = 1 ]; then cli latest; fi
# fd 3, so the installers and tests do not read the list.
while read -r entry min_cli <&3; do
  echo "::group::claude-agent-acp $entry"
  [ "${LATEST:-}" = 1 ] || cli "${min_cli:?the entry $entry has no min_cli}"
  DETACHED_AGENT_ENTRY=$entry real
  echo "::endgroup::"
done 3<<< "$entries"
