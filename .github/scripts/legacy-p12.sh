#!/usr/bin/env bash
# Re-packs a base64 .p12 so macOS `security import` can read it, and prints
# the result as base64 on stdout.
#
# `security import` (used by Tauri to set up its signing keychain) only
# understands .p12 files protected with the classic algorithms: SHA-1 MAC and
# 3DES (or RC2) encryption. Certificates exported by current macOS Keychain
# Access or OpenSSL 3 use PBES2/AES-256 with a SHA-256 MAC, and the import
# fails with "MAC verification failed during PKCS12 import (wrong password?)".
# Re-encrypting with 3DES + SHA-1 works for both old and new exports.
#
# Input:  P12_BASE64 (the .p12, base64) and P12_PASSWORD in the environment.
# Output: the re-packed .p12, base64 on one line, protected by the same
#         password. Nothing secret is written to stderr; the unencrypted key
#         only ever exists in a private temp dir that is removed on exit.
set -euo pipefail

: "${P12_BASE64:?P12_BASE64 is not set}"
: "${P12_PASSWORD?P12_PASSWORD is not set}"
openssl="${OPENSSL:-openssl}"

umask 077
work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/p12.XXXXXX")
trap 'rm -rf "$work"' EXIT

printf '%s' "$P12_BASE64" | "$openssl" base64 -d -A -out "$work/in.p12"

# OpenSSL 3 reads RC2-encrypted (old) exports only with the legacy provider.
read_p12() {
  "$openssl" pkcs12 -in "$work/in.p12" -passin env:P12_PASSWORD -nodes -out "$work/bundle.pem" "$@" 2>"$work/err"
}
if ! read_p12 && ! read_p12 -legacy; then
  # openssl's messages name the failing check (e.g. "Mac verify error"),
  # never key material or the password.
  echo "error: could not read the certificate (.p12): wrong password or not a PKCS#12 file" >&2
  sed 's/^/  openssl: /' "$work/err" >&2
  exit 1
fi

"$openssl" pkcs12 -export -in "$work/bundle.pem" -passout env:P12_PASSWORD \
  -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
  -out "$work/out.p12"

# Algorithms only (no bag attributes, keys or certificates), for the log.
"$openssl" pkcs12 -in "$work/out.p12" -passin env:P12_PASSWORD -info -noout 2>&1 \
  | grep -E '^(MAC|PKCS7 Encrypted data|Shrouded Keybag):' >&2 || true

"$openssl" base64 -A -in "$work/out.p12"
echo
