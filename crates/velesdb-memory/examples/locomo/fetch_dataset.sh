#!/usr/bin/env bash
# Fetch the LoCoMo research dataset (snap-research/locomo) into ./data.
# The dataset is NOT vendored: it is research data with its own terms, and at
# ~2.8 MB it would only bloat the crate. Run this once before the benchmark.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dest="$here/data/locomo10.json"
url="https://raw.githubusercontent.com/snap-research/locomo/main/data/locomo10.json"
mkdir -p "$here/data"
if [[ -s "$dest" ]]; then
  echo "locomo10.json already present ($(wc -c <"$dest") bytes) — skipping."
  exit 0
fi
echo "Downloading LoCoMo dataset → $dest"
curl -fsSL -o "$dest" "$url"
echo "Done ($(wc -c <"$dest") bytes)."
