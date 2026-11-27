#!/usr/bin/env bash
# Builds Teskad's Windows executables into target/dist, the way the release
# workflow does (.github/workflows/release.yml, "Build Hangar Mobile" and
# "Build"): Hangar Mobile first, since the app embeds dist-mobile/, then the
# app and the webview helper with the dist profile.
#
#   scripts/fork/build.sh            dist profile (fat LTO: slow, smallest, what releases ship)
#   scripts/fork/build.sh --release  release profile (several times faster to build)
#
# cmake and libclang (for whisper.cpp, dictation) are taken from Visual Studio
# when they are not on PATH already. Run from Git Bash.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

profile=dist
if [ "${1:-}" = "--release" ]; then profile=release; fi

pick() { for p in "$@"; do if [ -e "$p" ]; then echo "$p"; return 0; fi; done; return 1; }

if ! command -v cmake > /dev/null; then
  cmake_bin="$(dirname "$(pick \
    "/c/Program Files/Microsoft Visual Studio/18/Community/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe" \
    "/c/Program Files/Microsoft Visual Studio/2022/Community/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe" \
    "/c/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe")")" \
    || { echo "cmake not found: winget install Kitware.CMake" >&2; exit 1; }
  export PATH="$cmake_bin:$PATH"
fi

if [ -z "${LIBCLANG_PATH:-}" ]; then
  libclang="$(pick \
    "/c/Program Files/LLVM/bin/libclang.dll" \
    "/c/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/Llvm/x64/bin/libclang.dll" \
    "/c/Program Files/Microsoft Visual Studio/18/Community/VC/Tools/Llvm/x64/bin/libclang.dll")" \
    || { echo "libclang not found: winget install LLVM.LLVM" >&2; exit 1; }
  LIBCLANG_PATH="$(cygpath -w "$(dirname "$libclang")")"
  export LIBCLANG_PATH
fi

# The release's portable instruction set for whisper.cpp, so the build runs on
# any x86-64 machine with AVX2, not only this one.
export GGML_NATIVE=OFF GGML_SSE42=ON GGML_AVX=ON GGML_AVX2=ON GGML_FMA=ON GGML_F16C=ON GGML_BMI2=ON

echo "cmake:   $(command -v cmake) ($(cmake --version | head -1))"
echo "libclang: $LIBCLANG_PATH"
echo "profile: $profile"

if command -v pnpm > /dev/null; then
  pnpm install --frozen-lockfile
  pnpm mobile:build
else
  echo "pnpm not found: building without Hangar Mobile (phones get a 'missing' page)" >&2
fi

cargo build --profile "$profile" --locked -p hangar-app
cargo build --profile "$profile" --locked -p hangar-webview --features engine

echo
echo "Built: $root/target/$profile/teskad.exe and teskad-webview.exe"
