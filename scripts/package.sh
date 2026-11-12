#!/bin/sh
# Builds the release packages for this OS into target/package, under the release asset names.
set -eu
cd "$(dirname "$0")/.."

version=0.11.8
[ "$(cargo packager --version 2>/dev/null)" = "cargo-packager $version" ] ||
  cargo install cargo-packager --version "$version" --locked
out=$PWD/target/package
rm -rf "$out"
# From crates/app: the packager looks for config files under the working directory.
package() { (cd crates/app && cargo packager --release -o "$out" "$@"); }

case "$(uname -s)" in
Darwin)
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release -p detached-app --target aarch64-apple-darwin --target x86_64-apple-darwin
  mkdir -p target/universal-apple-darwin/release
  lipo -create -output target/universal-apple-darwin/release/detached \
    target/aarch64-apple-darwin/release/detached target/x86_64-apple-darwin/release/detached
  package --target universal-apple-darwin -f app
  # Signed here, not by the packager, which always asks Apple for a secure timestamp.
  if security find-identity -v -p codesigning | grep -q '"Detached Dev"'; then
    identity="Detached Dev"
  else
    echo "warning: no valid \"Detached Dev\" identity (see make sign-setup): signing ad-hoc." >&2
    identity=-
  fi
  codesign --force --options runtime -s "$identity" "$out/Detached.app"
  ditto -c -k --keepParent "$out/Detached.app" "$out/Detached-macos.zip"
  for arch in arm64:aarch64 x64:x86_64; do
    dir=$out/${arch%:*}
    mkdir "$dir"
    # Cargo's detached.dSYM is a link to deps/detached-<hash>.dSYM.
    cp -RL "target/${arch#*:}-apple-darwin/release/detached.dSYM" "$dir/"
    ditto -c -k --keepParent "$dir/detached.dSYM" "$out/Detached-macos-${arch%:*}-dSYM.zip"
  done
  ;;
MINGW* | MSYS* | CYGWIN*)
  cargo build --release -p detached-app
  package -f nsis
  mv "$out"/*-setup.exe "$out/Detached-windows-x64-setup.exe"
  ;;
*)
  cargo build --release -p detached-app
  package -f deb,appimage
  mv "$out"/*.deb "$out/Detached-linux-amd64.deb"
  mv "$out"/*.AppImage "$out/Detached-linux-x86_64.AppImage"
  ;;
esac
ls -l "$out"
