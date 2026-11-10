#!/bin/sh
# Starts a new worktree's build from the worktree that built last: an APFS clone, no disk used
# until they differ.
set -e
here=$(git rev-parse --show-toplevel)
if [ "$(uname)" != Darwin ]; then echo "warm: macOS only, nothing done"; exit; fi
if [ -e "$here/target/debug" ]; then echo "warm: target/debug is already here"; exit; fi
src=$(git worktree list --porcelain | sed -n 's/^worktree //p' | while read -r w; do
  if [ "$w" != "$here" ] && [ -d "$w/target/debug/deps" ]; then
    echo "$(stat -f %m "$w/target/debug/deps") $w"
  fi
done | sort -rn | sed -n '1s/^[0-9]* //p')
if [ -z "$src" ]; then echo "warm: no other worktree has built"; exit; fi
mkdir -p "$here/target"
cp -cRp "$src/target/debug" "$here/target/debug"
# Cargo judges freshness by file times, so our own crates could pass for the source's: build them anew.
rm -rf "$here"/target/debug/.fingerprint/detached-*
echo "warm: target/debug cloned from $src"
