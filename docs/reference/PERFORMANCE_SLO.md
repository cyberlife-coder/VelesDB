# VelesDB Performance SLO

Last updated: 2026-08-08

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

## v1.7.2 Optimization Notes (historical)

> Historical record of the v1.7.2 optimization pass; kept for context. The
> figures below reflect the baselines of that era and are superseded by the
> current CI baseline (`benchmarks/baseline.json`). Details of these
> optimizations live in `docs/guides/TUNING_GUIDE.md` and `docs/BENCHMARKS.md`.

v1.7.2 included three internal optimizations that improved SLO metrics:
- **Partial sort** (#373) — HNSW search uses O(ef + k log k) instead of O(ef log ef)
- **Batch fast-path** (#375) — pure-insert workloads skip DashMap write lock overhead
- **Upsert lock contention fix** — `Collection::upsert()` restructured into a 3-phase pipeline with read lock on HNSW (replacing write lock) and batch I/O. Primarily affects insert SLO: upsert throughput gap vs `upsert_bulk()` dropped from ~19x to ~1x on local benchmarks (i9-14900KF, 10K/384D).

At the time, local validation (i9-14900KF) confirmed no regression against the then-current CI baseline:
- `smoke_insert/10k_128d`: 8.36s (CI baseline: 9.0s, threshold: 25%)
- `smoke_search/10k_128d_k10`: 191.73 µs (local machine — CI baseline not directly comparable)

## Governance Rules

- Product promises in README/site must align with measured and reproducible benchmarks.
- Any change to benchmark methodology must update this file and baseline in the same PR.
