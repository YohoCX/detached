#!/bin/sh
# Installs or updates Detached from the latest GitHub release; run it again to update:
#   curl -fsSL https://github.com/YohoCX/detached/releases/latest/download/install.sh | sh
set -eu

url=${DETACHED_URL:-https://github.com/YohoCX/detached/releases/latest/download/Detached-macos.zip}
dir=${DETACHED_DIR:-/Applications}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ "$(uname -s)" = Darwin ] || { echo "Detached runs on macOS only." >&2; exit 1; }
echo "Downloading $url"
curl -fL --progress-bar "$url" -o "$tmp/Detached.zip"
ditto -x -k "$tmp/Detached.zip" "$tmp"
codesign --verify --deep --strict "$tmp/Detached.app"
# The app is not notarized: a quarantined copy (e.g. a browser download) would be blocked.
xattr -dr com.apple.quarantine "$tmp/Detached.app"

if pgrep -qf "Detached.app/Contents/MacOS/detached"; then
  echo "Quitting Detached."
  osascript -e 'quit app id "dev.wdevs.detached"'
fi
mkdir -p "$dir"
rm -rf "$dir/Detached.app"
mv "$tmp/Detached.app" "$dir/"
echo "Installed $dir/Detached.app"
