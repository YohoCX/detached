#!/usr/bin/env bash
# H4c part 0 on one OS (.github/workflows/h4c-evidence.yml): `claude` $1 from Anthropic's installer,
# then the spike's rows, ROWS of them (default all), into out/ with out/answers.txt.
set -euo pipefail
cd "$(dirname "$0")"
version=${1:?usage: ci.sh <claude version|latest>}

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
[ "$version" = latest ] || [ "${have%% *}" = "$version" ] || { echo "::error::claude is $have, not $version."; exit 1; }

cargo build
mkdir -p out
for row in ${ROWS:-all}; do ./target/debug/claude-direct-spike "$row"; done 2>&1 | tee out/answers.txt
