#!/usr/bin/env bash
set -euo pipefail

SERVER_FILE="crates/velesdb-server/src/main.rs"
README_FILE="README.md"

if [[ ! -f "$SERVER_FILE" ]]; then
  echo "ERROR: missing $SERVER_FILE"
  exit 1
fi

if [[ ! -f "$README_FILE" ]]; then
  echo "ERROR: missing $README_FILE"
  exit 1
fi

mapfile -t routes < <(
  grep -oE '\.route\("([^"]+)"' "$SERVER_FILE" \
    | sed -E 's/\.route\("([^"]+)"/\1/' \
    | sort -u
)

missing=()
for route in "${routes[@]}"; do
  # /metrics is feature-gated; README documents it in optional section.
  if ! grep -Fq "\`$route\`" "$README_FILE"; then
    missing+=("$route")
  fi
done

if (( ${#missing[@]} > 0 )); then
  echo "README is missing runtime routes:"
  for m in "${missing[@]}"; do
    echo "  - $m"
  done
  exit 1
fi

if ! grep -Fq '`/query/explain`' "$README_FILE"; then
  echo "README must document /query/explain"
  exit 1
fi

if ! grep -Fq '`/aggregate`' "$README_FILE"; then
  echo "README must document /aggregate"
  exit 1
fi

echo "Doc contract check passed: README includes all runtime routes from main router."
