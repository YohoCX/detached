#!/bin/sh
# Creates a self-signed "Workbench Dev" code-signing identity once, so every build carries the
# same signature and Keychain access survives rebuilds. Usage: scripts/sign-setup.sh [keychain]
set -eu

NAME="Workbench Dev"
KEYCHAIN="${1:-$HOME/Library/Keychains/login.keychain-db}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

has_identity() {
  security find-identity "$@" -p codesigning "$KEYCHAIN" | grep -q "\"$NAME\""
}

if has_identity; then
  echo "Identity \"$NAME\" already exists in $KEYCHAIN."
else
  cat >"$tmp/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
EOF
  # macOS LibreSSL: `security import` cannot read OpenSSL 3's default PKCS#12 encryption.
  /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$tmp/cert.cnf" \
    -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null
  /usr/bin/openssl pkcs12 -export -name "$NAME" -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
    -passout pass:workbench -out "$tmp/id.p12"
  security import "$tmp/id.p12" -k "$KEYCHAIN" -P workbench -T /usr/bin/codesign >/dev/null
  echo "Created identity \"$NAME\" in $KEYCHAIN."
fi

if has_identity -v; then
  echo "\"$NAME\" is trusted for code signing."
else
  security find-certificate -c "$NAME" -p "$KEYCHAIN" >"$tmp/trust.pem"
  echo "Trusting \"$NAME\" for code signing; macOS asks for your password."
  security add-trusted-cert -p codeSign -k "$KEYCHAIN" "$tmp/trust.pem"
  echo "Trusted. The first build may ask to let codesign use the key: choose Always Allow."
fi
