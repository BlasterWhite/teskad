#!/usr/bin/env bash
# Builds the macOS packages from the two dist builds cargo already made (the
# dist profile, which the workspace Cargo.toml explains):
#
#   Teskad_<version>_universal.dmg       what people download
#   Teskad_universal.app.tar.gz          what the updater downloads
#
#   packaging/macos/build.sh <out-dir>
#
# One universal app rather than two downloads: an Apple
# Silicon user who picks the Intel build still gets a working app. Needs, built
# first for both targets,
#
#   cargo build --profile dist --locked --target aarch64-apple-darwin \
#     -p hangar-app -p hangar-webview --features hangar-webview/engine
#   (and the same with --target x86_64-apple-darwin)
#
# the two Linux hangar-node files in <target dir>/node, and cargo-packager and jq. Signing and notarisation happen inside
# cargo-packager when the environment asks for them: APPLE_CERTIFICATE and
# APPLE_CERTIFICATE_PASSWORD, APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID, and a
# `signingIdentity` in the packager's configuration (the release workflow writes
# it in from its secret). Without them the app is unsigned.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <out-dir>" >&2
  exit 2
fi

source "$(dirname "${BASH_SOURCE[0]}")/../common.sh"
out="$(absolute_dir "$1")"
target_dir="$(cd "$repo" && cargo metadata --no-deps --format-version 1 | jq -r .target_directory)"

universal="$target_dir/universal-apple-darwin/dist"
mkdir -p "$universal"
for binary in teskad hangar-webview; do
  arm="$target_dir/aarch64-apple-darwin/dist/$binary"
  intel="$target_dir/x86_64-apple-darwin/dist/$binary"
  for built in "$arm" "$intel"; do
    if [ ! -x "$built" ]; then
      echo "$built is missing: build it first (see docs/native/PACKAGING.md)" >&2
      exit 1
    fi
  done
  lipo -create "$arm" "$intel" -output "$universal/$binary"
  lipo -info "$universal/$binary"
done

# hangar-node for Linux x86_64 and arm64, which this Mac uploads to a node:
# checked in target/node, from where the packager's `resources` put them in
# Contents/Resources (not Contents/MacOS: codesign refuses ELF files there).
stage_node_binaries "$target_dir"

# cargo-packager looks for the binaries in target/<--target>/<--profile>, and
# names the disk image after the target: Teskad_<version>_universal.dmg.
rm -rf "$out/.cargo-packager" "$out/Teskad.app"
(
  cd "$repo"
  cargo packager --profile dist --target universal-apple-darwin --formats app,dmg --out-dir "$out"
)
rm -rf "$out/.cargo-packager"

# What hangar-update downloads: the bundle in a gzipped tar with the .app as its
# one top-level folder, which it unpacks into place of the installed one. The
# AppleDouble files tar would otherwise add for extended attributes would sit
# inside the sealed bundle and break its signature.
COPYFILE_DISABLE=1 tar -C "$out" -czf "$out/Teskad_universal.app.tar.gz" Teskad.app

ls -la "$out"
