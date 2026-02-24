# Security Evidence Pack

Generated: 2026-02-24 22:32 UTC

## Snapshot

- Dependency advisories check: **N/A**
- Unsafe usage occurrences in `crates/velesdb-core/src`: **156**
- Fuzzing corpus/targets files under `fuzz/`: **6**

## Checklist

- [ ] Review advisories output and attach CI artifact.
- [ ] Review unsafe diff versus previous release tag.
- [ ] Run fuzz campaign and attach crash-free duration evidence.
- [ ] Attach SBOM / supply-chain attestation artifact.
- [ ] Confirm telemetry and data handling defaults in release notes.

## Commands

```bash
cargo deny check advisories
rg -n "\bunsafe\b" crates/velesdb-core/src
cargo fuzz list
```
