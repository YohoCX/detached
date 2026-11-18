#!/usr/bin/env bash
# CI's agent harness on one OS (.github/workflows/agent.yml): the tests on the fake ACP agent and the
# fake CLI, then `claude` from Anthropic's installer and the real tests of `make agent-test`. `claude`
# is at 2.1.322, or at CLI: a version, `floor` or `latest`; LATEST=1 takes the newest. WSL=<distro>
# on Windows runs the real tests again with the CLI in that distro.
set -euo pipefail
cd "$(dirname "$0")/.."

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
  cargo test -p detached-kernel --lib panel::tests::real -- --ignored
}
# Claude's floor: the manifest's, or the signed catalog's when that is higher.
floor() {
  { grep -m1 -o 'min_cli: Version::new([0-9, ]*)' crates/core/src/adapters.rs | tr -dc '0-9,' | tr , .
    echo
    jq -r '.cli[]? | select(.name == "claude") | .min_cli // empty' adapters/adapters.json
  } | sort -V | tail -1
}
# `cargo test` with the rest of the arguments and `claude` $1 in the distro $WSL, reaching the fake
# API on the distro's gateway address.
wsl_test() {
  export MSYS_NO_PATHCONV=1
  wsl.exe -d "$WSL" -e sh -c "curl -fsSL https://claude.ai/install.sh | bash -s $1 && ~/.local/bin/claude --version"
  shift
  host=$(wsl.exe -d "$WSL" -e sh -c "ip route show default | cut -d' ' -f3")
  echo "the gateway: $host"
  powershell -NoProfile -Command "New-NetFirewallRule -DisplayName detached-agent-wsl -Direction Inbound -LocalAddress $host -Action Allow | Out-Null"
  # A temp folder by its long name: the distro sees no 8.3 names.
  tmp=${RUNNER_TEMP:-$TMP}
  TMP=$tmp TEMP=$tmp DETACHED_AGENT_WSL=$WSL DETACHED_AGENT_WSL_HOST=$host \
    cargo test "$@"
}

version=${CLI:-2.1.322}
[ "${LATEST:-}" = 1 ] && version=latest
[ "$version" = floor ] && version=$(floor)
# With arguments, `claude` and only `cargo test <arguments>`, then again in $WSL.
if [ $# -gt 0 ]; then
  cli "$version"
  cargo test "$@"
  [ -z "${WSL:-}" ] || wsl_test "$version" "$@"
  exit
fi

# Inside WSL only what its kernel can change; acp's and core's tests run on native Linux.
if ! grep -qi microsoft /proc/sys/kernel/osrelease 2>/dev/null; then
  cargo test -p detached-acp
  cargo test -p detached-core --lib -- proc:: trust:: backend:: claude::
fi
cargo test -p detached-kernel --lib session::
cli "$version"
real
[ -z "${WSL:-}" ] || wsl_test "$version" -p detached-kernel --lib session::tests::real -- --ignored
