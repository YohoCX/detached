#!/bin/sh
# Builds the packages and starts this Mac's Detached.app with --self-test through LaunchServices, as Finder does.
# Extra arguments go to `open`, e.g. --env SHELL=/usr/bin/false to break the PATH probe.
set -eu
cd "$(dirname "$0")/.."

scripts/package.sh

arch=x64
[ "$(uname -m)" = arm64 ] && arch=arm64
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# `open` hands its own environment to the app; Finder gives launchd's PATH and the account's shell.
shell=$(dscl . -read "/Users/$USER" UserShell | sed 's/^UserShell: //')
env -i HOME="$HOME" USER="$USER" LOGNAME="$USER" SHELL="$shell" \
  TMPDIR="$(getconf DARWIN_USER_TEMP_DIR)" PATH=/usr/bin:/bin:/usr/sbin:/sbin \
  open -n -W "$@" "target/package/$arch/Detached.app" --args --self-test "$tmp/report.json"
if [ ! -f "$tmp/report.json" ]; then
  echo "FAIL: the app wrote no report." >&2
  exit 1
fi
cat "$tmp/report.json"
echo
if grep -q '^  "ok": true' "$tmp/report.json"; then
  echo "PASS"
else
  echo "FAIL" >&2
  exit 1
fi
