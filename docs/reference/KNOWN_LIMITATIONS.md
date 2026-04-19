# Known Limitations

This document lists the **internal technical limitations** of VelesDB Core. These are distinct from the product-level scope boundaries in the [README "Known Limitations" section](../../README.md#known-limitations) (single-writer, no distributed replication, WASM hop-limit, etc.). Each entry below is either:

- Tracked by a GitHub issue and scheduled for resolution, OR
- An explicit design approximation whose trade-off is documented in the source and covered by regression tests.

None of the items below is a correctness bug. They are transparency notes so operators, integrators, and code reviewers understand the bounds of the current implementation.

---

## Query planner / Cost-Based Optimizer

### 1. Cost magnitude shift after `ANALYZE`

**Status**: documented trade-off. Source: `crates/velesdb-core/src/velesql/explain/node_stats.rs` (`COST_UNIT_TO_MS = 0.001`, with `TODO` noting empirical calibration).

Once a collection has been analyzed, `EXPLAIN.estimated_cost_ms` is derived from the calibrated `CostEstimator` (real histogram-based selectivity + I/O / CPU weights). Before `ANALYZE`, the same query uses the legacy heuristic (fixed coefficients).

These two code paths produce values in different magnitude ranges:

| Example (10 K rows, VectorSearch `ef=100`, `k=10`) | Cost reported |
|---|---|
| Before `ANALYZE` (heuristic) | ≈ 0.1 ms |
| After `ANALYZE` (calibrated, `COST_UNIT_TO_MS = 0.001`) | ≈ 2.2 ms |

The ratio (~22×) is **not** a regression; it reflects that the calibrated path counts more operations per unit (probe visits, comparisons, I/O page reads) whereas the heuristic uses rule-of-thumb constants directly. Users comparing `EXPLAIN` output across an `ANALYZE` boundary should expect this jump.

**Resolution path**: pin `COST_UNIT_TO_MS` empirically via a micro-benchmark that times a known plan shape on reference hardware, then rescale the constant so pre/post-`ANALYZE` costs align at the same operating point. Not blocker for correctness — both paths rank the same plan shape consistently within their own range.

### 2. Plan cache is not invalidated by `ANALYZE`

**Status**: pre-existing. Tracked by [issue #608](https://github.com/cyberlife-coder/VelesDB/issues/608). Source: `crates/velesdb-core/src/database/query_engine.rs` (plan cache key uses `write_generation`).

Running `ANALYZE` on a collection updates `CollectionStats` (histograms, calibrated factors) but does **not** bump `write_generation`. Cached plans built by `populate_plan_cache` therefore continue to be served with their pre-`ANALYZE` cost estimates until the next data modification.

**User impact**: `EXPLAIN` on a query that was cached before `ANALYZE` will report stale `estimated_cost_ms` and potentially a stale `filter_strategy`. The execution result itself is unchanged — the cache stores plan shape, not cost.

**Workaround**: force a cache bump by performing any write to the collection (e.g. an idempotent `UPSERT` of an existing point) after `ANALYZE`. The proper fix is tracked as an `analyze_generation` counter in the cache key.

### 3. `build_plan_with_stats` does not thread indexed-field set

**Status**: pre-existing. Tracked by [issue #607](https://github.com/cyberlife-coder/VelesDB/issues/607). Source: `crates/velesdb-core/src/database/query_engine.rs` (`HashSet::new()` passed unconditionally).

`Database::build_plan_with_stats` constructs an empty `HashSet` for the `indexed_fields` argument of `QueryPlan::from_select_with_stats`. As a consequence, `IndexLookup` plan nodes are never generated through this code path — only `VectorSearch`, `TableScan`, and `Filter` nodes appear in the plan tree, and the cost estimator cannot apply the `IndexLookup` cost discount.

**User impact**: `EXPLAIN` for queries that should benefit from a secondary index on a metadata column shows a `TableScan` in the plan tree instead of an `IndexLookup`. Execution behaviour is unaffected (the executor consults the index registry directly); only the plan shape visible in `EXPLAIN` is impacted.

### 4. Post-filter cost uses a fixed-fraction approximation

**Status**: documented approximation. Tracked by [issue #609](https://github.com/cyberlife-coder/VelesDB/issues/609). Source: `crates/velesdb-core/src/velesql/explain/filter_strategy.rs` (`POSTFILTER_TOPK_COST_FRACTION = 0.01`).

In `resolve_filter_strategy`, the post-filter cost is modelled as `estimate_filter_cost_from_selectivity(selectivity).total() * 0.01`. The real cost is proportional to `k` (top-k HNSW results) rather than to `total * selectivity * 0.01`. The approximation overestimates the post-filter cost by up to ~5× for very large collections with high selectivity (e.g. `total = 10_000`, `sel = 0.5`, `k = 10` → model 50·C, real 10·C).

**User impact**: the CBO's post-filter cost number is directionally correct (cheaper than pre-filter when HNSW dominates) but not physically meaningful. In all tested regimes the HNSW term dominates so strategy decisions are unaffected, but a future replacement with a proper `k * cpu_tuple_cost` model would remove the approximation. Covered by `test_filter_strategy_switches_on_selectivity` and `test_prefilter_accounts_for_full_table_scan` BDD tests.

### 5. CBO `choose_hybrid_strategy` not integrated for pure-`SELECT` hybrid queries

**Status**: partial integration. Tracked by [issue #467](https://github.com/cyberlife-coder/VelesDB/issues/467) (scope-reduced). Source: `crates/velesdb-core/src/collection/search/query/mod.rs:16` (TODO comment).

The calibrated CBO is fully wired for `MATCH` queries (via `MatchQueryPlanner::plan`). For pure-`SELECT` hybrid queries (vector + WHERE + optional text), `QueryPlanner::choose_hybrid_strategy` and `PlanGenerator` are defined but not called by `execute_query`. The `filter_strategy` reported in `EXPLAIN` is still driven by `resolve_filter_strategy` (the pipeline used by this PR series) and remains accurate; the missing piece is the deeper strategy-enumeration layer that compares multiple candidate plans (`PlanGenerator::CandidatePlan`).

**User impact**: `MATCH` queries benefit from the full CBO; pure-`SELECT` hybrid queries benefit from calibrated filter-strategy selection but not from multi-candidate plan enumeration. Covered by `test_filter_strategy_switches_on_selectivity` + `test_prefilter_accounts_for_full_table_scan`.

### 6. Filter-strategy fallback threshold is runtime-tunable (default 0.1)

**Status**: resolved (configurable). Source: `crates/velesdb-core/src/velesql/explain/filter_strategy.rs` (`DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD = 0.1`, `AtomicU64` runtime state).

When no calibrated `CollectionStats` is available (collection never analyzed, SDK path without collection handle), `resolve_filter_strategy` falls back to `selectivity > threshold → PostFilter`. The threshold defaults to `0.1` to keep the ~50 pre-existing `EXPLAIN` tests green (backward-compat anchor), but is tunable at runtime via `velesdb_core::velesql::set_fallback_selectivity_threshold(value)` (lock-free `AtomicU64`, validates `[0.0, 1.0]`). Once stats are present, the cost-based comparison (pre-filter vs post-filter with recall guardrail at `selectivity >= 0.5`) takes over.

**User impact**: for unanalyzed collections, operators can tune the fallback threshold for workloads where the calibrated pathway is unavailable without recompiling. Running `ANALYZE` on the collection still switches the decision to the calibrated pathway documented by BDD tests `test_filter_strategy_switches_on_selectivity` and `test_filter_strategy_respects_ef_search`.

---

## Full-text search

### 7. BM25 cold-start triggers an O(N) rebuild

**Status**: open. Tracked by [issue #389](https://github.com/cyberlife-coder/VelesDB/issues/389). Source: `crates/velesdb-core/src/collection/core/lifecycle.rs:189-244` (`rebuild_bm25_index()` invoked on every `Database::open`).

The BM25 inverted index is not persisted. On `Database::open` it is rebuilt in-memory by scanning every document in the collection. For a 100 K-document corpus this adds a few hundred milliseconds at startup; for multi-million-document corpora the cold-start penalty is proportionally larger.

**User impact**: higher-than-expected latency on the first query after process start for text-heavy workloads. Steady-state query latency is unaffected.

**Resolution path**: implement `Bm25Snapshot` with `save()` / `load()` mirroring the HNSW graph persistence pattern (`hnsw.bin`), plus an incremental update layer that keeps the on-disk snapshot in sync with write operations.

---

## Reading this document

Each entry states:

- **Status**: open / partial / documented / pre-existing.
- **Source**: the file or line referenced in code.
- **User impact**: what an operator or integrator actually sees.
- **Resolution path or workaround** where applicable.

For product-level scope boundaries (single-writer, no replication, RBAC scope, WASM hop-limit, benchmark infrastructure), see the [README "Known Limitations" section](../../README.md#known-limitations).
