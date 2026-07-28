#!/usr/bin/env bash
# generate_bindings.sh — build velesdb-mobile and generate Swift + Kotlin bindings.
#
# This is the crate README's "first success in 60 seconds", generalised: it
# picks the right shared-library extension for the host and works in both debug
# and release profiles. No device, no simulator, no Xcode project, no NDK — the
# bindings are generated from a plain host build.
#
# Usage, from the repository root (or anywhere — the script finds the root):
#   ./crates/velesdb-mobile/examples/generate_bindings.sh
#   PROFILE=release ./crates/velesdb-mobile/examples/generate_bindings.sh
#   OUT_DIR=/tmp/bindings ./crates/velesdb-mobile/examples/generate_bindings.sh
#
# The first cargo build compiles velesdb-core and takes several minutes on a
# cold cache. Everything after that is seconds.

set -euo pipefail

PROFILE="${PROFILE:-debug}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${HERE}/../../.." && pwd)"
OUT_DIR="${OUT_DIR:-${ROOT}/bindings}"

case "$PROFILE" in
  debug) CARGO_PROFILE_FLAGS="" ;;
  release) CARGO_PROFILE_FLAGS="--release" ;;
  *)
    echo "PROFILE must be 'debug' or 'release' (got '${PROFILE}')." >&2
    exit 1
    ;;
esac

# UniFFI library mode reads the compiled cdylib. Its name differs per host.
case "$(uname -s)" in
  Darwin) LIB_NAME="libvelesdb_mobile.dylib" ;;
  Linux) LIB_NAME="libvelesdb_mobile.so" ;;
  MINGW* | MSYS* | CYGWIN*) LIB_NAME="velesdb_mobile.dll" ;;
  *)
    echo "Unrecognised host '$(uname -s)'. Set LIB_NAME yourself and re-run." >&2
    exit 1
    ;;
esac

LIB_PATH="${ROOT}/target/${PROFILE}/${LIB_NAME}"

echo "==> Building velesdb-mobile (${PROFILE})"
cd "$ROOT"
# shellcheck disable=SC2086  # intentional word splitting: the flag may be empty
cargo build -p velesdb-mobile $CARGO_PROFILE_FLAGS

[ -f "$LIB_PATH" ] || {
  echo "Expected library not found: ${LIB_PATH}" >&2
  echo "The crate builds a cdylib; check that the build above actually succeeded." >&2
  exit 1
}
echo "    ${LIB_PATH}"

echo
echo "==> Generating Swift bindings"
# shellcheck disable=SC2086
cargo run -p velesdb-mobile --bin uniffi-bindgen $CARGO_PROFILE_FLAGS -- generate \
  --library "$LIB_PATH" \
  --language swift \
  --out-dir "${OUT_DIR}/swift"
ls -1 "${OUT_DIR}/swift"

echo
echo "==> Generating Kotlin bindings"
# shellcheck disable=SC2086
cargo run -p velesdb-mobile --bin uniffi-bindgen $CARGO_PROFILE_FLAGS -- generate \
  --library "$LIB_PATH" \
  --language kotlin \
  --out-dir "${OUT_DIR}/kotlin"
find "${OUT_DIR}/kotlin" -name '*.kt' -print

cat <<EOF

Done.
-----
Swift  : ${OUT_DIR}/swift/velesdb_mobile.swift
         plus velesdb_mobileFFI.h and velesdb_mobileFFI.modulemap — exactly
         three files, nothing else. Anything different means the --library
         path was wrong for this host.
Kotlin : ${OUT_DIR}/kotlin/uniffi/velesdb_mobile/velesdb_mobile.kt
         one file; note the package, uniffi.velesdb_mobile.

The generated sources are the authority on every name. Read them before
writing platform code — see ../swift/README.md and ../kotlin/README.md for
how to package them, and docs/guides/MOBILE_BUILD.md for the device targets.

Sanity-check the engine itself, with no mobile toolchain at all:

    cargo test -p velesdb-mobile
EOF
