#!/usr/bin/env bash
# README <-> runtime route contract.
#
# Documentation *freshness* (date stamps, docs/README.md index coverage,
# hardcoded versions vs the Cargo manifests) is a separate, independently
# selectable guard: `scripts/check-doc-freshness.py`. Both are wired in
# .github/workflows/doc-contract.yml. This script stays route-only, and its
# blocking scope is unchanged, so `scripts/run-production-gates.sh` and
# `propagation-guard.yml` — which both call it — keep the exact pass/fail
# semantics they had before.
#
# Two scopes, on purpose:
#
#   * PINNED routes (`/query/explain`, `/aggregate`) — always blocking. This
#     is exactly what the script enforced before the router moved out of
#     main.rs, so restoring the full sweep cannot change the verdict on any
#     tree that was green yesterday.
#   * FULL sweep of every `.route("...")` in the server crate — governed by
#     DOC_CONTRACT_MODE, default `warn`. It is red on develop today (4 routes
#     were added without a README entry while the sweep was disarmed). Flip
#     the default to `strict` in the same commit that documents them:
#       /collections/{name}/compact  /collections/{name}/points/raw
#       /collections/{name}/stream/enable  /collections/{name}/vacuum
set -euo pipefail

DOC_CONTRACT_MODE="${DOC_CONTRACT_MODE:-warn}"
if [[ "$DOC_CONTRACT_MODE" != "strict" && "$DOC_CONTRACT_MODE" != "warn" ]]; then
  echo "ERROR: DOC_CONTRACT_MODE must be 'strict' or 'warn' (got '$DOC_CONTRACT_MODE')"
  exit 2
fi

# Source of truth for the runtime routes. This used to be the single file
# `crates/velesdb-server/src/main.rs`; the router was extracted to
# `src/routes.rs` and main.rs kept ZERO `.route("...")` calls, so the loop
# below iterated over an empty array and the gate passed vacuously for every
# route. Scan the whole server crate source instead, so moving the router
# again cannot silently disarm the check.
SERVER_SRC="crates/velesdb-server/src"
README_FILE="README.md"

if [[ ! -d "$SERVER_SRC" ]]; then
  echo "ERROR: missing $SERVER_SRC"
  exit 1
fi

if [[ ! -f "$README_FILE" ]]; then
  echo "ERROR: missing $README_FILE"
  exit 1
fi

mapfile -t routes < <(
  grep -rhoE '\.route\("([^"]+)"' --include='*.rs' "$SERVER_SRC" \
    | sed -E 's/\.route\("([^"]+)"/\1/' \
    | sort -u
)

# Guard against the failure mode described above: an empty route list means
# the extraction broke, not that the router is empty.
if (( ${#routes[@]} == 0 )); then
  echo "ERROR: no \`.route(\"...\")\` call found under $SERVER_SRC."
  echo "The extraction is broken (or the router moved again) — this check"
  echo "would otherwise pass without verifying anything."
  exit 1
fi

missing=()
for route in "${routes[@]}"; do
  # /metrics is feature-gated; README documents it in optional section.
  if ! grep -Fq "\`$route\`" "$README_FILE"; then
    missing+=("$route")
  fi
done

if (( ${#missing[@]} > 0 )); then
  if [[ "$DOC_CONTRACT_MODE" == "warn" ]]; then
    echo "WARNING: $README_FILE does not document ${#missing[@]} of the ${#routes[@]} runtime route(s):"
    for m in "${missing[@]}"; do
      echo "  - $m"
      echo "::warning file=$README_FILE::README does not document runtime route $m"
    done
    echo "(DOC_CONTRACT_MODE=warn — full-sweep findings are not failing the build.)"
  else
    echo "$README_FILE is missing ${#missing[@]} runtime route(s) declared under $SERVER_SRC:"
    for m in "${missing[@]}"; do
      echo "  - $m"
      echo "::error file=$README_FILE::README does not document runtime route $m"
    done
    echo
    echo "Document each route in $README_FILE inside backticks (e.g. \`/query/explain\`),"
    echo "or delete the route from the server if it is gone."
    exit 1
  fi
fi

# Always blocking, in every mode: the two differentiator endpoints. These were
# the only checks the script actually performed while the sweep was disarmed,
# so keeping them hard preserves the historical pass/fail semantics exactly.
pinned_missing=()
for required in '/query/explain' '/aggregate'; do
  if ! grep -Fq "\`$required\`" "$README_FILE"; then
    pinned_missing+=("$required")
  fi
done

if (( ${#pinned_missing[@]} > 0 )); then
  echo "$README_FILE must document these endpoints (always blocking):"
  for m in "${pinned_missing[@]}"; do
    echo "  - $m"
    echo "::error file=$README_FILE::README must document $m"
  done
  exit 1
fi

if (( ${#missing[@]} > 0 )); then
  echo "Doc contract check passed (blocking scope): the pinned endpoints are documented;"
  echo "${#missing[@]} of the ${#routes[@]} routes declared under $SERVER_SRC are still undocumented (warning above)."
else
  echo "Doc contract check passed: README documents all ${#routes[@]} runtime routes declared under $SERVER_SRC."
fi
