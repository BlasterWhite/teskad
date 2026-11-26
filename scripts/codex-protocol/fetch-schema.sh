#!/usr/bin/env bash
# The JSON Schema of the pinned Codex release (subset.json's "codex"), written by that release's own
# `codex app-server generate-json-schema` into a new local folder whose path is printed. With
# --generate, generate.mjs then runs on the folder.
#
#   scripts/codex-protocol/fetch-schema.sh [--generate]
#
# Codex comes from npm into a throwaway prefix, removed on exit, as in the schema watch
# (.github/workflows/codex-schema-watch.yml): never installed globally, install scripts off, no
# login (the schema needs none), an empty CODEX_HOME. Needs node, npm and the network.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GENERATE=0
for arg in "$@"; do
  case "$arg" in
    --generate) GENERATE=1 ;;
    *) echo "usage: scripts/codex-protocol/fetch-schema.sh [--generate]" >&2; exit 2 ;;
  esac
done

VERSION="$(node -p "require(process.argv[1]).codex" "$HERE/subset.json")"
FLAGS=()
if [ "$(node -p "Boolean(require(process.argv[1]).experimental)" "$HERE/subset.json")" = true ]; then
  FLAGS=(--experimental)
fi
OUT="$(mktemp -d "${TMPDIR:-/tmp}/codex-schema-$VERSION.XXXXXX")"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/codex-install-$VERSION.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
mkdir "$WORK/home"

echo "fetch-schema: installing codex $VERSION into a throwaway prefix…" >&2
npm install --ignore-scripts --no-audit --no-fund --prefix "$WORK/npm" "@openai/codex@$VERSION" >&2
codex="$WORK/npm/node_modules/.bin/codex"
CODEX_HOME="$WORK/home" "$codex" app-server generate-json-schema "${FLAGS[@]}" --out "$OUT" >&2
CODEX_HOME="$WORK/home" "$codex" --version > "$OUT/codex-version.txt"
if [ "$(cat "$OUT/codex-version.txt")" != "codex-cli $VERSION" ]; then
  echo "fetch-schema: the installed codex answered $(cat "$OUT/codex-version.txt"), not codex-cli $VERSION" >&2
  exit 1
fi
echo "$OUT"
if [ "$GENERATE" = 1 ]; then
  node "$HERE/generate.mjs" --schema "$OUT"
fi
