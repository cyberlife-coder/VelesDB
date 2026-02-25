#!/usr/bin/env bash
set -euo pipefail

if ! command -v curl >/dev/null 2>&1; then
  echo "❌ curl is required"
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "❌ jq is required"
  exit 1
fi

EXPECTED_VERSION="${1:-}"
shift || true

if [ -z "$EXPECTED_VERSION" ]; then
  echo "Usage: $0 <expected_version> <crate1> [crate2 ... crateN]"
  exit 1
fi

if [ "$#" -eq 0 ]; then
  echo "❌ At least one crate name must be provided"
  exit 1
fi

failures=0

for crate in "$@"; do
  echo "🔎 Checking $crate"

  response=$(curl -fsSL --retry 3 --retry-delay 2 --retry-connrefused \
    -H "User-Agent: VelesDB-release-check/1.0" \
    -H "Accept: application/json" \
    "https://crates.io/api/v1/crates/${crate}")
  max_version=$(printf '%s' "$response" | jq -r '.crate.max_version')

  if [ "$max_version" != "$EXPECTED_VERSION" ]; then
    echo "❌ $crate max_version=$max_version (expected $EXPECTED_VERSION)"
    failures=$((failures + 1))
  else
    echo "✅ $crate max_version=$max_version"
  fi
done

if [ "$failures" -gt 0 ]; then
  echo "❌ Version check failed for $failures crate(s)."
  exit 1
fi

echo "✅ All crates are published at version $EXPECTED_VERSION"
