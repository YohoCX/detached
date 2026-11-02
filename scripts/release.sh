#!/bin/sh
# Builds a universal Detached.app signed with "Detached Dev" and publishes it with install.sh as
# GitHub release v<version from src-tauri/tauri.conf.json>.
# Every release must be signed with the same identity, or users' Keychain access breaks on update.
set -eu
cd "$(dirname "$0")/.."

version=$(node -p 'require("./src-tauri/tauri.conf.json").version')
out=src-tauri/target/universal-apple-darwin/release/bundle/macos

security find-identity -v -p codesigning | grep -q '"Detached Dev"' ||
  { echo 'No "Detached Dev" identity: run make sign-setup.' >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "Commit your changes first." >&2; exit 1; }
gh auth status >/dev/null

rustup target add aarch64-apple-darwin x86_64-apple-darwin
pnpm tauri build --bundles app --target universal-apple-darwin
ditto -c -k --keepParent "$out/Detached.app" "$out/Detached-macos.zip"

git tag "v$version"
git push origin "v$version"
gh release create "v$version" --repo YohoCX/detached --verify-tag --title "Detached $version" \
  --generate-notes "$out/Detached-macos.zip" scripts/install.sh
