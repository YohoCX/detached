#!/bin/sh
# Installs or updates Detached from the latest GitHub release; run it again to update:
#   curl -fsSL https://github.com/YohoCX/detached/releases/latest/download/install.sh | sh
set -eu

# A function, so a truncated download runs nothing.
main() {
  url=${DETACHED_URL:-https://github.com/YohoCX/detached/releases/latest/download/Detached-macos.zip}
  dir=${DETACHED_DIR:-/Applications}
  [ "$(uname -s)" = Darwin ] || { echo "Detached runs on macOS only." >&2; exit 1; }
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT

  echo "Downloading $url"
  curl -fL --progress-bar "$url" -o "$tmp/Detached.zip"
  ditto -x -k "$tmp/Detached.zip" "$tmp"
  codesign --verify --deep --strict "$tmp/Detached.app"
  # Not notarized: a quarantined copy would be blocked by Gatekeeper.
  xattr -dr com.apple.quarantine "$tmp/Detached.app"

  if pgrep -qf "Detached.app/Contents/MacOS/detached"; then
    echo "Quitting Detached."
    osascript -e 'quit app id "dev.wdevs.detached"'
  fi
  mkdir -p "$dir"
  rm -rf "$dir/Detached.app"
  mv "$tmp/Detached.app" "$dir/"
  echo "Installed $dir/Detached.app"
}

main
