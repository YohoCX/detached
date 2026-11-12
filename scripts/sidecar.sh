#!/bin/sh
# Builds the Claude sidecar and installs it where the app looks: plugins/claude in the config directory.
set -e
cd "$(dirname "$0")/../sidecars/claude"
bun install --omit=optional
case "$(uname)" in
  Darwin) base="$HOME/Library/Application Support" ;;
  *) base="${XDG_CONFIG_HOME:-$HOME/.config}" ;;
esac
dir="${DETACHED_CONFIG_DIR:-$base/com.w-devs.detached}/plugins/claude"
mkdir -p "$dir"
bun build --compile sidecar.ts --outfile "$dir/sidecar"
echo "installed $dir/sidecar"
