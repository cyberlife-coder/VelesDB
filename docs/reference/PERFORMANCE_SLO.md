# VelesDB Performance SLO

Last updated: 2026-02-18

This file defines measurable performance objectives used as CI regression gates.

## Scope

- Engine: `velesdb-core`
- Workload baseline: `crates/velesdb-core/benches/smoke_test.rs`
- Dataset profile: `10k vectors, 128 dimensions`

## SLO Targets (Smoke)

| Metric | Target | Source |
|--------|--------|--------|
| insert mean (`smoke_insert/10k_128d`) | no regression > 15% vs baseline | `benchmarks/baseline.json` |
| search mean (`smoke_search/10k_128d_k10`) | no regression > 15% vs baseline | `benchmarks/baseline.json` |

## CI Enforcement

On `main` and `develop` pushes:

1. Run smoke benchmark:
   `cargo bench -p velesdb-core --bench smoke_test -- --noplot`
2. Export criterion result:
   `python3 scripts/export_smoke_criterion.py`
3. Compare against baseline:
   `python3 scripts/compare_perf.py --current benchmarks/results/latest.json --baseline benchmarks/baseline.json --threshold 15`

If threshold is exceeded, CI fails.

## Governance Rules

- Product promises in README/site must align with measured and reproducible benchmarks.
- Any change to benchmark methodology must update this file and baseline in the same PR.
