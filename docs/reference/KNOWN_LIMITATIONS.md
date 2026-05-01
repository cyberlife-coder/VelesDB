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

### 2. Multi-candidate `PlanGenerator` enumeration not wired into `execute_query`

**Status**: partial integration (scope-reduced). Tracked by [issue #467](https://github.com/cyberlife-coder/VelesDB/issues/467). Source: `crates/velesdb-core/src/collection/query_cost/plan_generator.rs` (`PlanGenerator::CandidatePlan`).

`compute_cbo_strategy` in `collection/search/query/select_dispatch.rs` now routes SELECT queries through two calibrated planner entry points:

- `QueryPlanner::choose_hybrid_strategy` for queries carrying `ORDER BY similarity()` — forces `VectorFirst` to preserve HNSW natural ordering regardless of cost estimates.
- `QueryPlanner::choose_strategy_with_cbo_and_overfetch` for all other SELECT queries — calibrated I/O / CPU cost comparison across `VectorFirst` / `GraphFirst` / `Parallel`.

Both branches feed into the same `dispatch_vector_query` executor through the `(ExecutionStrategy, over_fetch: usize)` tuple.

**What remains open**: the deeper `PlanGenerator::CandidatePlan` enumeration (SeqScan, IndexScan, VectorSearch, GraphTraversal, hybrid combinations) is still not consumed by `execute_query`. The current two-path routing covers the operationally common cases — full multi-candidate enumeration would only change the decision when the cost landscape is non-trivially multimodal.

**User impact**: `MATCH` queries use the full CBO via `MatchQueryPlanner::plan`. SELECT queries (including ORDER BY similarity + filter) now use calibrated strategy and over-fetch selection. Covered by `test_cbo_forces_vector_first_for_order_by_similarity_with_selective_filter` + `test_cbo_calibrated_path_still_works_without_order_by_similarity` + `test_filter_strategy_switches_on_selectivity`.

### 3. Filter-strategy fallback threshold is runtime-tunable (default 0.1)

**Status**: resolved (configurable). Source: `crates/velesdb-core/src/velesql/explain/filter_strategy.rs` (`DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD = 0.1`, `AtomicU64` runtime state).

When no calibrated `CollectionStats` is available (collection never analyzed, SDK path without collection handle), `resolve_filter_strategy` falls back to `selectivity > threshold → PostFilter`. The threshold defaults to `0.1` to keep the ~50 pre-existing `EXPLAIN` tests green (backward-compat anchor), but is tunable at runtime via `velesdb_core::velesql::set_fallback_selectivity_threshold(value)` (lock-free `AtomicU64`, validates `[0.0, 1.0]`). Once stats are present, the cost-based comparison (pre-filter vs post-filter with recall guardrail at `selectivity >= 0.5`) takes over.

**User impact**: for unanalyzed collections, operators can tune the fallback threshold for workloads where the calibrated pathway is unavailable without recompiling. Running `ANALYZE` on the collection still switches the decision to the calibrated pathway documented by BDD tests `test_filter_strategy_switches_on_selectivity` and `test_filter_strategy_respects_ef_search`.

---

## Workspace scope

### 4. `velesdb-migrate` (12,108 LOC, 9 connectors) — to be reworked

**Status**: open, scheduled for rework decision in v1.15.0. Source: `crates/velesdb-migrate/` (workspace member).

The `velesdb-migrate` sub-crate ships a migration toolkit covering 9 source databases (Supabase, Qdrant, Pinecone, Weaviate, Milvus, ChromaDB, JSON/CSV, Elasticsearch, Redis). It is currently bundled in the workspace but is identified for **rework or extraction in a future release**: the current scope inflates the workspace surface (12k LOC, 9 third-party API surfaces) without a measured user base, and the connectors evolve at different cadences than the core engine.

**Decision criteria for v1.15.0** (per ROADMAP.md Horizon 2):

- crates.io download counts for `velesdb-migrate` over the last 90 days
- GitHub stars / watchers attributable to migration tooling
- Open issues count specifically scoped to migration connectors

**User impact**: until the rework decision lands, the crate is maintained on a best-effort basis. Users depending on it should pin to the v1.14.x line. No migration tooling will be removed or moved during the v1.14.x line — this is purely a forward-looking transparency note.

**Resolution path**: tracked for v1.15.0 evaluation; the candidate outcomes are (a) keep + invest, (b) extract to separate `velesdb-migrate` repository under the same org, or (c) archive with documented sunset window. The decision will be made in a separate planning issue once the criteria above are measurable.

---

## Reading this document

Each entry states:

- **Status**: open / partial / documented / pre-existing.
- **Source**: the file or line referenced in code.
- **User impact**: what an operator or integrator actually sees.
- **Resolution path or workaround** where applicable.

For product-level scope boundaries (single-writer, no replication, RBAC scope, WASM hop-limit, benchmark infrastructure), see the [README "Known Limitations" section](../../README.md#known-limitations).
