#!/usr/bin/env bash
# The JSON Schema of the pinned Codex release (subset.json's "codex"), written by that release's own
# `codex app-server generate-json-schema` on Hangar's build machine (scripts/remote), and brought
# back into a new local folder whose path is printed. Codex is not installed on the person's PC, and
# this never installs it there. With --generate, generate.mjs then runs on the folder.
#
#   scripts/codex-protocol/fetch-schema.sh [--generate]
#
# On the build machine everything lives in /root/codex-protocol: a Node of its own (official build,
# checked against nodejs.org's SHA-256 list) and an npm prefix per Codex version; nothing global,
# no login (the schema needs none), an empty CODEX_HOME. RBUILD=<path to rbuild> uses another copy
# of the build tool than this checkout's scripts/remote/rbuild.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
RBUILD="${RBUILD:-$ROOT/scripts/remote/rbuild}"
GENERATE=0
for arg in "$@"; do
  case "$arg" in
    --generate) GENERATE=1 ;;
    *) echo "usage: scripts/codex-protocol/fetch-schema.sh [--generate]" >&2; exit 2 ;;
  esac
done
if [ ! -x "$RBUILD" ]; then
  echo "fetch-schema: no build tool at $RBUILD (RBUILD=<path to scripts/remote/rbuild>)" >&2
  exit 2
fi

VERSION="$(node -p "require(process.argv[1]).codex" "$HERE/subset.json")"
FLAGS=()
if [ "$(node -p "Boolean(require(process.argv[1]).experimental)" "$HERE/subset.json")" = true ]; then
  FLAGS=(--experimental)
fi
OUT="$(mktemp -d "${TMPDIR:-/tmp}/codex-schema-$VERSION.XXXXXX")"

# Run as is on the build machine: nothing in it expands here.
# shellcheck disable=SC2016
REMOTE='
set -euo pipefail
version="$1"
experimental="${2:-}"
base=/root/codex-protocol
mkdir -p "$base"
cd "$base"
if [ ! -x node/bin/node ]; then
  v=$(curl -fsSL https://nodejs.org/dist/index.json | python3 -c "import json, sys; print(next(x[\"version\"] for x in json.load(sys.stdin) if x[\"lts\"] and x[\"version\"].startswith(\"v24.\")))")
  curl -fsSL "https://nodejs.org/dist/$v/node-$v-linux-x64.tar.xz" -o node.tar.xz
  curl -fsSL "https://nodejs.org/dist/$v/SHASUMS256.txt" | grep " node-$v-linux-x64.tar.xz\$" \
    | sed "s/node-$v-linux-x64.tar.xz/node.tar.xz/" | sha256sum -c - >&2
  rm -rf node && mkdir node && tar -xJf node.tar.xz -C node --strip-components=1 && rm node.tar.xz
fi
export PATH="$base/node/bin:$PATH"
if [ ! -x "$base/$version/node_modules/.bin/codex" ]; then
  npm install --no-audit --no-fund --prefix "$base/$version" "@openai/codex@$version" >&2
fi
codex="$base/$version/node_modules/.bin/codex"
work=$(mktemp -d -p "$base" run.XXXXXX)
trap "rm -rf \"$work\"" EXIT
mkdir "$work/home"
CODEX_HOME="$work/home" "$codex" app-server generate-json-schema $experimental --out "$work/schema" >&2
"$codex" --version > "$work/schema/codex-version.txt"
tar -C "$work/schema" -czf - . | base64 -w0
'

echo "fetch-schema: codex $VERSION on the build machine…" >&2
"$RBUILD" ssh bash -s -- "$VERSION" "${FLAGS[@]}" <<<"$REMOTE" | base64 -d | tar -xzf - -C "$OUT"
if [ "$(cat "$OUT/codex-version.txt")" != "codex-cli $VERSION" ]; then
  echo "fetch-schema: the build machine answered $(cat "$OUT/codex-version.txt"), not codex-cli $VERSION" >&2
  exit 1
fi
echo "$OUT"
if [ "$GENERATE" = 1 ]; then
  node "$HERE/generate.mjs" --schema "$OUT"
fi
