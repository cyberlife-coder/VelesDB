# Phase E Benchmark Report (2026-02-20)

Scope: `velesdb-core` performance/stability validation for HNSW, VelesQL executor/parser,
and BM25/hybrid text path.

## Commands Executed

- `cargo bench --bench hnsw_benchmark -- --noplot`
- `cargo bench --bench velesql_execution_benchmark -- --noplot`
- `cargo bench --bench match_parser_benchmark -- --noplot`
- `cargo bench --bench bm25_benchmark -- --noplot`
- `cargo bench --bench recall_benchmark -- --noplot`

## Key Outcomes

### Recall quality target (10K/128D)

From `recall_benchmark` summary:

- Fast (`ef=64`): `92.2%`
- Balanced (`ef=128`): `98.8%`
- Accurate (`ef=256`): `100.0%`
- Perfect (`ef=2048`): `100.0%`

Status: recall objectives are met for the reference quality profile.

### Latency observations (current host run)

From `recall_hnsw/n10000_k10_*`:

- Fast: `~2.14 ms`
- Balanced: `~3.68 ms`
- Accurate: `~10.03 ms`
- Perfect: `~26.28 ms`

Note: these are benchmark-environment measurements and are not directly comparable to
site/README microsecond claims without strict hardware/methodology parity.

### Regression scan (criterion comparison)

- `hnsw_benchmark`: multiple search-path regressions flagged vs prior criterion baseline.
- `velesql_execution_benchmark`: mixed profile (vector-near regressions, substantial
  improvements on text/multicolumn paths).
- `bm25_benchmark`: search/hybrid regressions broadly flagged.
- `match_parser_benchmark`: mostly stable, localized regressions on
  `parse_relationship` and `parse_match_simple`.
- `scripts/compare_perf.py` against `benchmarks/baseline.json`:
  - `smoke_insert/10k_128d`: `+88.3%` (regression)
  - `smoke_search/10k_128d_k10`: `+107.1%` (regression)

## Triage Decision

- Smoke baseline regressions are above baseline gate and require remediation or
  baseline re-validation on a controlled reference host before `develop -> main`.
- Performance debt exists on hot search/text paths and must be tracked before
  `develop -> main`.

## Next Actions

1. Pin a dedicated benchmark host profile (CPU governor/affinity/no background load).
2. Refresh criterion baseline from controlled host after stabilization.
3. Investigate top hot-path regressions:
   - `hnsw_search_latency/top_k/*`
   - `collection_search/search_10k_top10`
   - `bm25_search/search_single_term/*`
   - `collection_hybrid_search/hybrid_search/*`
4. Attach root-cause and fix PRs before final release merge.
