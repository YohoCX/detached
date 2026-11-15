#!/bin/sh
# Builds adapters/<name> with bun as the catalog ships it: out/<name>[.exe] and its licenses, and
# out.tar.gz of the two. A bun target (bun-linux-x64, ...) as the second argument cross-builds.
set -eu
name=$1
target=${2:-}
cd "$(dirname "$0")/../adapters/$name"

# --omit optional keeps the SDK's bundled Claude CLI out: the user's own `claude` runs.
bun install --frozen-lockfile --omit optional
exe=$name
case "${target:-$(uname -s)}" in *windows* | MINGW* | MSYS* | CYGWIN*) exe=$name.exe ;; esac
rm -rf out out.tar.gz
mkdir -p out/licenses
bun build --compile ${target:+--target=$target} \
  "node_modules/@agentclientprotocol/$name/dist/index.js" --outfile "out/$exe"
for pkg in node_modules/*/ node_modules/@*/*/; do
  for f in "$pkg"LICEN[CS]E* "$pkg"COPYING*; do
    [ -f "$f" ] || continue
    mkdir -p "out/licenses/${pkg#node_modules/}"
    cp "$f" "out/licenses/${pkg#node_modules/}"
  done
done
size=$(wc -c < "out/$exe")
[ "$size" -lt 157286400 ] || { echo "$exe is $size bytes: is a CLI bundled?" >&2; exit 1; }
if find out -name claude -o -name claude.exe -o -name codex -o -name codex.exe | grep -q .; then
  echo "A provider's CLI is in the build." >&2
  exit 1
fi
(cd out && COPYFILE_DISABLE=1 tar -czf ../out.tar.gz -- *)
