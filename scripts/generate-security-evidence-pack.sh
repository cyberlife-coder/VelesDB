#!/usr/bin/env bash
set -euo pipefail

OUT="docs/reference/security-evidence-pack.md"
DATE_UTC="$(date -u +"%Y-%m-%d %H:%M UTC")"

ADVISORIES="N/A"
if command -v cargo >/dev/null 2>&1 && cargo deny --version >/dev/null 2>&1; then
  if cargo deny check advisories >/tmp/velesdb_advisories.log 2>&1; then
    ADVISORIES="pass"
  else
    ADVISORIES="fail (see /tmp/velesdb_advisories.log)"
  fi
fi

UNSAFE_COUNT="$(rg -n "\bunsafe\b" crates/velesdb-core/src | wc -l | tr -d ' ')"
FUZZ_TARGETS="$(rg --files fuzz | wc -l | tr -d ' ')"

cat > "$OUT" <<MD
# Security Evidence Pack

Generated: $DATE_UTC

## Snapshot

- Dependency advisories check: **$ADVISORIES**
- Unsafe usage occurrences in \`crates/velesdb-core/src\`: **$UNSAFE_COUNT**
- Fuzzing corpus/targets files under \`fuzz/\`: **$FUZZ_TARGETS**

## Checklist

- [ ] Review advisories output and attach CI artifact.
- [ ] Review unsafe diff versus previous release tag.
- [ ] Run fuzz campaign and attach crash-free duration evidence.
- [ ] Attach SBOM / supply-chain attestation artifact.
- [ ] Confirm telemetry and data handling defaults in release notes.

## Commands

\`\`\`bash
cargo deny check advisories
rg -n "\\bunsafe\\b" crates/velesdb-core/src
cargo fuzz list
\`\`\`
MD

echo "Wrote $OUT"
