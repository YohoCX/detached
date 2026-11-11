#!/bin/sh
# Prints the latest.json that crates/core/src/update.rs reads: version $1, and per platform the package's URL under
# $2 with the signature from its .minisig in directory $3.
set -eu
jq -n --arg version "$1" --arg url "$2" \
  --rawfile mac "$3/Detached-macos.zip.minisig" \
  --rawfile windows "$3/Detached-windows-x64-setup.exe.minisig" \
  --rawfile linux "$3/Detached-linux-x86_64.AppImage.minisig" '
  def package($file; $signature): {url: "\($url)/\($file)", $signature};
  {$version, platforms: {
    "macos-aarch64": package("Detached-macos.zip"; $mac),
    "macos-x86_64": package("Detached-macos.zip"; $mac),
    "windows-x86_64": package("Detached-windows-x64-setup.exe"; $windows),
    "linux-x86_64": package("Detached-linux-x86_64.AppImage"; $linux)}}'
