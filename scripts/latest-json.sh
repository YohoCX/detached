#!/bin/sh
# Prints the latest.json that crates/core/src/update.rs reads: version $1, and per platform the package's URL under
# $2 with the signature from its .minisig in directory $3. The universal 0.5.0 picks its key by the arch it runs on.
set -eu
jq -n --arg version "$1" --arg url "$2" \
  --rawfile arm "$3/Detached-macos-arm64.zip.minisig" \
  --rawfile intel "$3/Detached-macos-x64.zip.minisig" \
  --rawfile windows "$3/Detached-windows-x64-setup.exe.minisig" \
  --rawfile linux "$3/Detached-linux-x86_64.AppImage.minisig" '
  def package($file; $signature): {url: "\($url)/\($file)", $signature};
  {$version, platforms: {
    "macos-aarch64": package("Detached-macos-arm64.zip"; $arm),
    "macos-x86_64": package("Detached-macos-x64.zip"; $intel),
    "windows-x86_64": package("Detached-windows-x64-setup.exe"; $windows),
    "linux-x86_64": package("Detached-linux-x86_64.AppImage"; $linux)}}'
