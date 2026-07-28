#!/usr/bin/env bash
# serve.sh — static server for the browser examples in this directory.
#
# WebAssembly cannot be loaded from a file:// page, so the examples must be
# served over HTTP. Any static server works; this one has no dependencies
# beyond python3.
#
# The server is rooted at the CRATE directory (the parent of examples/), not at
# examples/ itself. That is deliberate: it keeps both module layouts reachable —
# examples/node_modules/@wiscale/velesdb-wasm/ for the published package, and
# pkg/ for a local `wasm-pack build`. A server rooted at examples/ could not
# serve ../pkg/ at all.
#
# Usage:
#   ./serve.sh          # http://localhost:8080/examples/01-quickstart/
#   PORT=9000 ./serve.sh

set -euo pipefail

PORT="${PORT:-8080}"
EXAMPLES="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$EXAMPLES")"

command -v python3 > /dev/null 2>&1 || {
  echo "python3 not found. Any static file server works, for example:" >&2
  echo "  npx --yes http-server -p ${PORT} \"${ROOT}\"" >&2
  exit 1
}

if [ ! -d "${EXAMPLES}/node_modules/@wiscale/velesdb-wasm" ] \
  && [ ! -f "${ROOT}/pkg/velesdb_wasm.js" ]; then
  echo "Warning: neither the published package nor a local build was found." >&2
  echo "  (cd \"${EXAMPLES}\" && npm install)                            # published package" >&2
  echo "  wasm-pack build crates/velesdb-wasm --target web --release   # local build" >&2
  echo "Serving anyway — the pages report the error themselves." >&2
  echo >&2
fi

echo "Serving ${ROOT} on http://localhost:${PORT}"
echo
echo "  http://localhost:${PORT}/examples/01-quickstart/"
echo "  http://localhost:${PORT}/examples/02-payload-filter/"
echo "  http://localhost:${PORT}/examples/03-indexeddb-persistence/"
echo "  http://localhost:${PORT}/examples/04-agent-memory/"
echo "  http://localhost:${PORT}/examples/05-velesql/"
echo
echo "Ctrl+C to stop."

cd "$ROOT"
exec python3 -m http.server "$PORT"
