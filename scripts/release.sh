#!/bin/sh
# Tags v<version from src-tauri/tauri.conf.json>; .github/workflows/release.yml builds and publishes it.
set -eu
cd "$(dirname "$0")/.."

version=$(bun -p 'require("./src-tauri/tauri.conf.json").version')
[ -z "$(git status --porcelain)" ] || { echo "Commit your changes first." >&2; exit 1; }
git fetch -q origin
[ -n "$(git branch -r --contains HEAD)" ] || { echo "Push your commits first." >&2; exit 1; }

git tag "v$version"
git push origin "v$version"
echo "Building: https://github.com/YohoCX/detached/actions/workflows/release.yml"
