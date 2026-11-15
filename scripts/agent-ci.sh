#!/usr/bin/env bash
# CI's agent harness on one OS (.github/workflows/agent.yml): H4's tests on the fake ACP agent, then
# `claude` from Anthropic's installer and `make agent-test`. `claude` is at the version the
# adapter's SDK pins (its catalog entry's min_cli). VET=1 installs the adapter from the committed,
# signed catalog instead, at its newest entry's min_cli; LATEST=1 takes the newest `claude`.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo test -p detached-acp
cargo test -p detached-kernel --lib session::
cargo test -p detached-core --lib -- proc:: trust::

./scripts/adapter-build.sh claude-agent-acp
version=$(jq -r .claudeCodeVersion adapters/claude-agent-acp/node_modules/@anthropic-ai/claude-agent-sdk/package.json)
if [ "${VET:-}" = 1 ]; then
  if [ ! -f adapters/adapters.json.minisig ]; then
    echo "::notice::adapters/adapters.json is not signed yet; nothing to vet."
    exit 0
  fi
  newest='[.adapters[] | select(.name == "claude-agent-acp")] | max_by(.version | split(".") | map(tonumber))'
  DETACHED_AGENT_ENTRY=$(jq -r "$newest | .version // empty" adapters/adapters.json)
  version=$(jq -r "$newest | .min_cli // empty" adapters/adapters.json)
  [ -n "$DETACHED_AGENT_ENTRY" ] && [ -n "$version" ] \
    || { echo "::error::The catalog has no claude-agent-acp entry with a min_cli."; exit 1; }
  py=python3; command -v python3 > /dev/null || py=python
  "$py" -m http.server 8765 --bind 127.0.0.1 --directory adapters > /dev/null 2>&1 &
  server=$!
  trap 'kill "$server"' EXIT
  export DETACHED_AGENT_ENTRY DETACHED_AGENT_CATALOG=http://127.0.0.1:8765/adapters.json
fi
[ "${LATEST:-}" = 1 ] && version=latest

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    powershell -NoProfile -Command "& ([scriptblock]::Create((irm https://claude.ai/install.ps1))) $version"
    bin=$(cygpath "$USERPROFILE")/.local/bin
    ;;
  *)
    curl -fsSL https://claude.ai/install.sh | bash -s "$version"
    bin=$HOME/.local/bin
    ;;
esac
export PATH=$bin:$PATH
have=$(claude --version)
echo "claude $have"
[ "$version" = latest ] || [ "${have%% *}" = "$version" ] \
  || { echo "::error::claude is $have, not $version."; exit 1; }

make agent-test
