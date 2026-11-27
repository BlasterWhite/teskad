#!/usr/bin/env bash
# Signs the files the updater downloads, leaving <file>.sig beside each.
#
#   UPDATER_SIGNING_KEY=... UPDATER_SIGNING_KEY_PASSWORD=... packaging/sign.sh <file>...
#
# The private key is the one `cargo packager signer generate` made, kept in the
# repository secrets UPDATER_SIGNING_KEY and UPDATER_SIGNING_KEY_PASSWORD (the
# password is not empty; docs/native/PACKAGING.md, "The updater's signing key").
# Its public half is compiled into every install as hangar_update::PUBKEY, and
# an install only accepts what that key signed.
#
# `cargo packager signer sign` writes what the feed needs: a minisign signature
# of the file's BLAKE2b hash, with the file's name in its trusted comment, the
# whole .sig text base64-encoded. It reads the key and its password from
# CARGO_PACKAGER_SIGN_PRIVATE_KEY and CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD,
# which are set below for it alone, never put on a command line.
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <file>..." >&2
  exit 2
fi

key="${UPDATER_SIGNING_KEY:-}"
if [ -z "$key" ]; then
  echo "no signing key: set UPDATER_SIGNING_KEY (without it the release would be refused by every install)" >&2
  exit 1
fi
password="${UPDATER_SIGNING_KEY_PASSWORD:-}"
if [ -z "$password" ]; then
  echo "no password: set UPDATER_SIGNING_KEY_PASSWORD (the key is generated with a non-empty one)" >&2
  exit 1
fi

for file in "$@"; do
  CARGO_PACKAGER_SIGN_PRIVATE_KEY="$key" \
    CARGO_PACKAGER_SIGN_PRIVATE_KEY_PASSWORD="$password" \
    cargo packager signer sign "$file"
  test -s "$file.sig"
done
