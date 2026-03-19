# VelesDB Performance SLO

Last updated: 2026-03-19

This file defines measurable performance objectives used as CI regression gates.

## Scope

- Engine: `velesdb-core`
- Workload baseline: `crates/velesdb-core/benches/smoke_test.rs`
- Dataset profile: `10k vectors, 128 dimensions`
- Baseline environment: **GitHub Actions `ubuntu-latest` (2-core AMD)** — re-baselined 2026-03-12
  (Previous baseline v1.5.1 was measured on a local i9-14900KF Windows 11 machine, which is
  ~1.75–2× faster than CI runners; baselines are now CI-authoritative as of v1.5.2.)

## SLO Targets (Smoke)

| Metric | Target | Source |
|--------|--------|--------|
| insert mean (`smoke_insert/10k_128d`) | no regression > 25% vs baseline | `benchmarks/baseline.json` |
| search mean (`smoke_search/10k_128d_k10`) | no regression > 15% vs baseline | `benchmarks/baseline.json` |

> **Note:** Insert uses a wider 25% threshold because it is IO-bound on shared CI runners
> with high variance (7.86–10.42s across 5 runs). Search is CPU-bound and stable at 15%.

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
