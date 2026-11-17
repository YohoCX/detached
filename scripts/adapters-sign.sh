#!/bin/sh
# Signs adapters/adapters.json with the catalog's own minisign key, kept outside the repo: CI's
# fallback, and what adapters.yml runs with the key from ADAPTERS_SIGNING_KEY. The first run makes
# the key and prints its public half for core's CATALOG_KEY. With an entry.json from an adapters-*
# release, the entry goes in first, replacing the same name and version. Every signing raises the
# serial by one.
set -eu
cd "$(dirname "$0")/.."

dir=${DETACHED_ADAPTERS_KEY_DIR:-$HOME/.config/detached-adapters}
key=$dir/adapters.key
pub=$dir/adapters.pub
catalog=adapters/adapters.json
for tool in minisign jq; do
  command -v "$tool" > /dev/null || { echo "Install $tool first." >&2; exit 1; }
done

if [ ! -f "$key" ]; then
  mkdir -p "$dir"
  chmod 700 "$dir"
  minisign -G -W -p "$pub" -s "$key"
  echo "Set CATALOG_KEY in crates/core/src/adapters.rs to:"
  sed -n 2p "$pub"
  exit 0
fi

want=$(sed -n 2p "$pub")
have=$(sed -n 's/^pub const CATALOG_KEY: &str = "\(.*\)";$/\1/p' crates/core/src/adapters.rs)
[ "$want" = "$have" ] || { echo "CATALOG_KEY in crates/core/src/adapters.rs is not $pub's key." >&2; exit 1; }

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
if [ -n "${1:-}" ]; then
  jq --slurpfile e "$1" \
    '.adapters |= (map(select(.name != $e[0].name or .version != $e[0].version)) + $e)' \
    "$catalog" > "$tmp"
else
  cp "$catalog" "$tmp"
fi
jq '.serial += 1' "$tmp" > "$catalog"
minisign -S -s "$key" -m "$catalog"
minisign -V -q -P "$want" -m "$catalog"
echo "Signed serial $(jq .serial "$catalog"). Commit $catalog and its .minisig, then push to main."
