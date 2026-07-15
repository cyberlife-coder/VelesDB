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

**Strategy realization on SELECT (audit F-2.15 — RESOLVED, #1390)**: on the NEAR + metadata-filter SELECT path (`execution_paths.rs`, `dispatch_vector_with_strategy`), the `ExecutionStrategy` the planner selects is executed as three **physically distinct** plans:

- `VectorFirst` — filtered HNSW search (`search_with_filter_and_opts`).
- `GraphFirst` — a full metadata scan scored by vector similarity (`scan_and_score_by_vector`), physically distinct from the HNSW path.
- `Parallel` — the GraphFirst scan and the VectorFirst HNSW branch run **concurrently** via `rayon::join`, then merge by best-score-per-id. On `MATCH` (`match_dispatch.rs`, `execute_match_parallel`), `Parallel` likewise runs its GraphFirst and VectorFirst legs concurrently via `rayon::join` and merges by `node_id`.

The concurrent Parallel result set is identical to the former sequential one (both legs are read-only; the merge is order-insensitive); the shared EXPLAIN counters remain the sum of both legs (atomic `fetch_add`). The other SELECT arms (similarity() threshold, pure NEAR, metadata-only, SELECT *) each have a single sensible physical plan and deliberately ignore the strategy — see the `dispatch_vector_query` match arms for the per-arm rationale. SELECT graph predicates still use their own anchored pre-filter (`graph_prefilter.rs`) independently of `ExecutionStrategy`. Covered by `test_parallel_strategy_returns_best_score_union_of_both_branches`, the forced-`GraphFirst`/`VectorFirst` dispatch tests, and `parallel_counters_sum_both_legs`.

**User impact**: `MATCH` queries use the full CBO via `MatchQueryPlanner::plan`. SELECT queries (including ORDER BY similarity + filter) now use calibrated strategy and over-fetch selection. Covered by `test_cbo_forces_vector_first_for_order_by_similarity_with_selective_filter` + `test_cbo_calibrated_path_still_works_without_order_by_similarity` + `test_filter_strategy_switches_on_selectivity`.

### 3. Filter-strategy fallback threshold is runtime-tunable (default 0.1)

**Status**: resolved (configurable). Source: `crates/velesdb-core/src/velesql/explain/filter_strategy.rs` (`DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD = 0.1`, `AtomicU64` runtime state).

When no calibrated `CollectionStats` is available (collection never analyzed, SDK path without collection handle), `resolve_filter_strategy` falls back to `selectivity > threshold → PostFilter`. The threshold defaults to `0.1` to keep the ~50 pre-existing `EXPLAIN` tests green (backward-compat anchor), but is tunable at runtime via `velesdb_core::velesql::set_fallback_selectivity_threshold(value)` (lock-free `AtomicU64`, validates `[0.0, 1.0]`). Once stats are present, the cost-based comparison (pre-filter vs post-filter with recall guardrail at `selectivity >= 0.5`) takes over.

**User impact**: for unanalyzed collections, operators can tune the fallback threshold for workloads where the calibrated pathway is unavailable without recompiling. Running `ANALYZE` on the collection still switches the decision to the calibrated pathway documented by BDD tests `test_filter_strategy_switches_on_selectivity` and `test_filter_strategy_respects_ef_search`.

### Indexed metadata `Eq` query falls back to full scan above 50× the limit (audit F-4.7)

**Status**: documented heuristic. Source: `crates/velesdb-core/src/collection/search/query/metadata_query.rs` (`execute_indexed_metadata_query`).

For a metadata query whose `WHERE` reduces to an indexed `Eq`, the executor uses the secondary index directly. If the index reports more than `max(execution_limit × 50, 1000)` matching ids, it abandons the index and does a full filtered scan instead. The `50×` factor (with a 1000-id floor) is an intentional but **arbitrary** cutoff: past that fan-out, materializing and post-filtering the id list from the index is empirically slower than a straight scan, but the exact break-even point is workload-dependent and has not been calibrated per collection.

**User impact**: none on correctness (results are identical either way). On a low-selectivity `Eq` over a very large collection, the query may take the scan path even when a different constant would have kept the index path. The factor is a compile-time constant today; making it cost-model-driven (like the filter-strategy threshold above) is a possible follow-up.

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

### 5. No macOS Intel (x86_64) wheel on PyPI

**Status**: open, no ETA. Source: `.github/workflows/release.yml` `publish-pypi-wheels` matrix.

The `macos-13` (Intel x86_64) entry was added briefly in v1.14.4 (PR #738) but the GitHub-hosted `macos-13` runner availability proved unreliable: one v1.14.4 publish attempt left the wheel-build job queued for over 9 hours without a runner being assigned, blocking the rest of the release pipeline. The entry was removed in v1.14.5 to keep the release pipeline reliable.

**User impact**: Intel Mac users have three options.

1. **Recommended**: install via the macOS aarch64 wheel under Rosetta 2 — `arch -arm64 pip install velesdb`. Performance is within ~3-5% of native on Intel Macs running macOS 12+ with Rosetta 2.
2. **Build from source**: `cargo install velesdb-cli` (or `pip install velesdb --no-binary :all:` with a working Rust toolchain) produces a native x86_64 binary.
3. Use the Linux x86_64 wheel inside Docker / Lima / Multipass.

**Resolution path**: tracked for v1.15.0+. Candidate outcomes:

- (a) Provision a self-hosted `macos-13` runner via a paid CI provider with reliable Intel-Mac capacity.
- (b) Wait for GitHub-hosted `macos-13` queue times to stabilize and re-add the matrix entry.
- (c) Drop x86_64 macOS wheel support officially — Apple stopped shipping new Intel Macs in 2023, and Rosetta 2 covers existing devices.

A measurable decision will be made when one of: download counts on `manylinux2014_x86_64.whl` from macOS user-agents drops below 5%/month, OR a self-hosted runner is funded.

---

## Hardening limits (security / OOM / DoS guard-rails)

These are **intentional hard limits** introduced by the core hardening effort to
bound resource use against corrupt files and adversarial queries. They are not
bugs; they are the documented ceilings of the current implementation.

### 6. VelesQL query length and nesting depth

**Status**: resolved (hard limit). Source: `crates/velesdb-core/src/velesql/parser/prescan.rs` (`MAX_NESTING_DEPTH = 64`).

Before a query reaches the `pest` parser, a single O(n) pre-scan rejects any
query that:

- exceeds the configured `max_query_length`, or
- has an effective parse-recursion depth (open `()`/`[]` brackets plus a leading
  `NOT NOT …` run) greater than **64**.

The pre-scan exists because `pest` builds the full recursive parse tree before
any Rust-level guard runs, so a deeply nested query (~thousands of levels) would
otherwise overflow the native stack and abort the process. Quoted strings,
backtick/double-quoted identifiers, and `--` comments are skipped so the guard
never false-positives on literal bracket content.

**User impact**: legitimate hand-written queries nest a handful of levels and
are unaffected; programmatically generated queries must keep bracket/NOT nesting
at or below 64.

### 7. GROUP BY group-count ceiling

**Status**: resolved (server-side hard ceiling). Source: `crates/velesdb-core/src/collection/search/query/aggregation/having.rs` (`DEFAULT_MAX_GROUPS = 10_000`, `SERVER_MAX_GROUPS_CEILING = 1_000_000`).

A GROUP BY query retains at most `DEFAULT_MAX_GROUPS` (10,000) groups by default.
A query may use `WITH (max_groups = N)` (or `group_limit`) to **lower** its group
budget, but `N` is always **clamped down** to the server-side ceiling of
**1,000,000** — a query can never raise the memory ceiling. Exceeding the
effective limit returns a "Too many groups" error rather than growing unbounded.

### 8. `NOT similarity()` scan cap

**Status**: resolved (hard limit). Source: `crates/velesdb-core/src/collection/search/query/similarity_filter.rs` (`NOT_SIMILARITY_MAX_SCAN = 5_000_000`).

A `NOT similarity(...)` predicate has no index acceleration and must full-scan
the collection. It is now a hard guard-rail (not just a warning): if the
collection holds more than **5,000,000** vectors the query is **rejected** with
guidance to add a selective metadata filter or use a positive `similarity()`
predicate (which is index-accelerated).

### 9. Bounded query-result materialization

**Status**: resolved (bounded memory). Source: `crates/velesdb-core/src/collection/search/query/` (set_operations, parallel_traversal, similarity_filter), `database/query_engine.rs`, `database/query_join.rs`.

Result materialization for top-k scans, JOIN, parallel graph traversal, and
set operations (UNION/INTERSECT) is bounded by the effective LIMIT via bounded
top-k rather than collect-all-then-truncate. Results are identical to the
unbounded path; only peak memory is bounded. Intermediate operators that can
legitimately drop rows fall back to the conservative server-side ceiling — this
includes a scalar (non-`similarity()`) `ORDER BY ... LIMIT k`, which must rank
the full matching set before truncating, so it fetches exhaustively rather than
bounding the fetch at `k`. (Capping the fetch at `k` first was the
`ORDER BY`-before-sort defect fixed 2026-06-14; the bounded==unbounded identity
above now holds for scalar `ORDER BY` as well, at the cost of an exhaustive
fetch for that one operator.) The `similarity()`-ordered HNSW path stays bounded
top-k — it is pre-sorted by score, so truncation is correct without an
exhaustive fetch and recall is unaffected.

The O(n) cost of that exhaustive scalar-`ORDER BY` fetch is removed by the
ordered-index pushdown (EPIC-081, `docs/planning/CORE_PARITY_REMEDIATION.md`):
when the single `ORDER BY` field has a fully-covering secondary index and the
query has no WHERE/JOIN/graph/similarity, the engine serves the top-k from the
index in O(log n + k) (`create_index(field)` to opt in) — ~89 ms → ~0.013 ms for
that 50k-row query — with identical results to the exhaustive path. Queries
without a covering index keep the exhaustive scalar-`ORDER BY` behaviour above.
The `create_index(field)` opt-in is persisted (recorded in `config.json` and
rebuilt from the stored payloads on open), so the fast path keeps firing after a
process restart instead of silently reverting to the exhaustive scan.

### 10. Configuration range caps

**Status**: resolved (validated in every loader and on open; the `limits.*`
fields are additionally enforced at runtime since 2026-06-14). Source:
`crates/velesdb-core/src/config_validation.rs` (range checks, called from
`Config` loaders and `Database::open_with_config`);
`crates/velesdb-core/src/collection/{types,payload_size,core/crud,core/bulk_import,core/graph_api,search/vector,search/vector_filter}.rs`
and `database/collection_ops.rs` (runtime enforcement).

`VelesConfig::validate()` now runs in every config loader (`load`,
`load_from_path`, `from_toml`) **and** on `open_with_config`. Each capacity/limit
field is range-checked against a hard ceiling:

| Field | `0` means | Hard ceiling |
|-------|-----------|--------------|
| `limits.max_vectors_per_collection` | rejected | 10,000,000,000 |
| `limits.max_collections` | rejected | 1,000,000 |
| `limits.max_payload_size` | rejected | 1 GiB (1,073,741,824) |
| `search.query_timeout_ms` | disabled | 24 h (86,400,000 ms) |
| `hnsw.max_layers` | auto | 64 |
| `storage.mmap_cache_mb` | rejected | 1 TiB (1,048,576 MiB) |
| `server.workers` | auto (CPU count) | 4,096 |

An out-of-range value fails the loader/open with `ConfigError::InvalidValue`
rather than being silently accepted (which previously allowed `0` = DoS or
absurdly large = unbounded). The per-client `RateLimiter` map is also bounded
with sampled eviction so a client cycling `client_id` values cannot OOM the
limiter.

Beyond range validation, all five `limits.*` fields are now **enforced at
runtime** (2026-06-14): `max_dimensions` / `max_collections` at collection
creation, and `max_vectors_per_collection` / `max_payload_size` /
`max_perfect_mode_vectors` at the cold ingest/search boundary inside the
`Collection` (off the hot path), covering the Point upsert, zero-copy raw
bulk, graph node-write, and filtered/unfiltered search paths. An operation
that would exceed a cap is rejected with `Error::GuardRail` (`VELES-027`)
naming the actual value and the `limits.<field>` to raise; the engine never
silently clamps. Two intentional scoping notes: (1) `max_vectors_per_collection`
is a conservative O(1) pre-count (`stored + batch`) that treats every incoming
point as net-new, so a collection exactly at the cap may reject a pure in-place
update batch — raise the cap to update at the limit; (2) vector-less graph node
writes do not increment the vector count, so `max_vectors_per_collection` does
not apply to pure-graph node ingest (only `max_payload_size` does there).

### 11. `AllocGuard` per-allocation ceiling

**Status**: resolved (backstop). Source: `crates/velesdb-core/src/alloc_guard.rs` (`DEFAULT_ALLOC_BYTE_LIMIT = 1 TiB`).

Every raw aligned allocation is capped at a process-wide per-allocation ceiling
of **1 TiB**, configurable at runtime via `set_alloc_byte_limit`. This is a
deliberately high *backstop* against arithmetic-wrapped or pathological sizes; it
is far above any single contiguous buffer VelesDB legitimately allocates, so it
never rejects a real index. Primary defense against untrusted sizes is the
per-artifact load-time validation (file-length-bounded counts).

---

## Hybrid search / fusion

### `FUSION(maximum)` / `FUSION(average)` mix raw score scales (BM25 dominates)

**Status**: documented design choice (not a bug). Sources: `crates/velesdb-core/src/fusion/strategy.rs` (`fuse_maximum`, `fuse_average` — no normalization; `fuse_rsf` → `min_max_normalize`), `crates/velesdb-core/src/collection/search/text_fusion.rs` (routes `maximum`/`average`/`rsf` to score-level fusion of the raw vector-similarity and BM25 streams).

**Symptom**: in a hybrid query fusing a vector branch with a text branch — e.g.
`WHERE vector NEAR $v AND content MATCH '...' USING FUSION(strategy = 'maximum')`
(or `'average'`) — the fused ranking is dominated by the text (BM25) results,
and the vector branch has little or no visible influence.

**Why**: `maximum` and `average` operate on the **raw** per-branch scores with
no normalization. Vector similarity is bounded (cosine ∈ [0, 1]) while BM25 is
unbounded (routinely > 1, often 5–20 on longer queries), so under `maximum` the
BM25 score almost always wins, and under `average` it dwarfs the vector
contribution. These strategies are designed for branches whose scores share a
scale (e.g. two dense-vector branches over the same metric); the engine
deliberately does not second-guess the caller by normalizing behind their back.

**Recommendation**: for mixed-scale hybrids (vector + BM25), use
`FUSION(strategy = 'rrf')` — rank-based, therefore insensitive to score scale —
or `FUSION(strategy = 'rsf', dense_weight = ..., sparse_weight = ...)`, which
min-max normalizes each branch to [0, 1] before the weighted sum. Reserve
`maximum`/`average` for branches with commensurable scores. See the
scale-mixing caveat in [`VELESQL_SPEC.md` → USING FUSION](../VELESQL_SPEC.md#using-fusion----hybrid-search-v20).

---

## Ecosystem / interop

### 12. String → u64 point-ID hashing differs across components

**Status**: documented trade-off (intentional). Sources: `integrations/common/src/velesdb_common/ids.py` (`stable_hash_id`, the single source shared by LangChain, LlamaIndex, **and** Haystack since 2026-06-14); `integrations/haystack/src/haystack_velesdb/document_store.py` (now imports `stable_hash_id` — the previous forked `_str_id_to_int` copy was removed); `crates/velesdb-migrate/src/pipeline_points.rs` (`stable_point_id`).

VelesDB point IDs are `u64`. Components that ingest documents keyed by an
arbitrary string derive the numeric ID with **two intentionally different**
hash strategies — every Python integration now shares one, and `velesdb-migrate`
keeps its deliberately distinct one:

| Component | Function | Strategy |
|-----------|----------|----------|
| LangChain / LlamaIndex / Haystack | `velesdb_common.ids.stable_hash_id` (shared) | SHA-256 of the UTF-8 string, top 8 bytes, sign bit cleared → positive 63-bit ID |
| `velesdb-migrate` | `stable_point_id` | numeric strings parsed directly to `u64`; non-numeric strings hashed via FNV-1a |

These do not agree. The `velesdb-migrate` strategy is deliberately distinct: it
parses numeric IDs verbatim so a source row keyed `"12345"` maps to point
`12345`, and its FNV-1a fallback is frozen for **checkpoint-resumable**
migrations (changing it would re-key already-inserted points and corrupt a
resumed run — see the stability note in the source).

**User impact**: the *same* logical document can land under **different point
IDs** depending on the ingestion path. A corpus loaded via `velesdb-migrate`
and the same corpus loaded via the LangChain/LlamaIndex/Haystack vector store
will not share point IDs, so cross-referencing or de-duplicating across the two
paths by point ID is not reliable. Pick a single ingestion path per collection,
or map on a payload field (e.g. a stored `source_id`) rather than on the
numeric point ID.

---

## Tooling / test coverage

### 13. VelesQL executor conformance (core, WASM, CLI)

**Status**: resolved (2026-06-20). Sources: `crates/velesdb-core/tests/velesql_executor_conformance.rs`, `crates/velesdb-cli/tests/velesql_executor_conformance.rs`, `crates/velesdb-wasm/src/velesql_executor_conformance_tests.rs` + the shared `conformance/velesql_executor_cases.json`; parser layer `crates/velesdb-{wasm,cli}/tests/velesql_parser_conformance.rs`; server REST-contract layer `crates/velesdb-server/tests/velesql_conformance_tests.rs`.

The shared VelesQL conformance fixtures come in layers. The
`velesql_parser_cases.json` layer (does this query parse?) is checked across
**core, WASM, and CLI**. The `velesql_contract_cases.json` layer (does this
query *execute* and return the contracted result/error shape over REST?) is
exercised at the **server** runtime. The `velesql_executor_cases.json` layer
(does this query produce the exact result **rows / counts / ordering**?) is now
run by all three executors — **core, CLI, and WASM** — with goldens derived from
`velesdb-core` as the source of truth.

**Architecture note**: the three executors are *not* the same code path. The CLI
delegates SELECT execution to `velesdb-core` (`Database::execute_query`), so it
inherits core behaviour directly. WASM runs its **own** independent SELECT/ORDER
BY pipeline (`velesql_select` / `velesql_orderby`), sharing only the AST/parser
with core — which is exactly why a per-runtime executor golden has real value:
it pins the WASM result set against core rather than assuming shared-executor
equivalence.

**Coverage** (`conformance/velesql_executor_cases.json`, cases X001–X010 plus
the B001 regression lock): scalar string-equality and integer-range WHERE
filters, conjunctive (AND) filters, single- and multi-column ORDER BY in mixed
directions, the deterministic ascending-id tie-break, and bounded top-k
(`ORDER BY ... LIMIT k`, both ASC and DESC). A result-shape divergence specific
to the WASM or CLI surface now fails CI rather than going unnoticed.

### Large production files vs the NLOC/complexity gate (audit F-5.13)

**Status**: documented (governance clarification). Sources: [`QUALITY_BAR.md`](../../QUALITY_BAR.md) Gates 5–6, [`.codacy.yml`](../../.codacy.yml) `engines.lizard.exclude_paths`.

The enforced Codacy metric is **per-function**: cyclomatic complexity ≤ 8 and function NLOC ≤ 50. There is no hard file-length gate — a file can be large yet fully compliant if it is a set of small functions or is comment-dense. So raw line count is not, by itself, a violation.

Several production files exceed ~900 raw lines (e.g. `simd_native/x86_avx512.rs` ≈ 1469 raw / ~944 NLOC — dominated by per-intrinsic `// SAFETY:` blocks; `tauri-plugin-velesdb/src/{types,commands}.rs`; `velesdb-python/src/{agent.rs, collection/search.rs}`; `velesql/.../match_dispatch.rs`; `velesdb-server/src/config.rs`). They fall into two buckets:

- **Covered by an exclusion**: the tauri-plugin files via the `crates/tauri-plugin-velesdb/src/**` lizard glob.
- **Compliant without an exclusion**: the rest — their individual functions stay within CC ≤ 8 / NLOC ≤ 50, which is why `main` (3.10.0) ships Codacy-green. They are intentionally **not** added to `.codacy.yml`, because a blanket file exclusion would suppress genuine future per-function findings in them.

The rule of thumb when adding a large file: only list it under `engines.lizard.exclude_paths` (with a rationale) if a **specific function** legitimately exceeds the per-function budget; never to silence file size alone.

---

## Bindings / SDK architecture

### 14. `MobileGraphStore` is a deliberate in-memory graph fork

**Status**: documented (by design). Source: `crates/velesdb-mobile/src/graph.rs` (`MobileGraphStore`); core counterparts `crates/velesdb-core/src/collection/graph/edge.rs` (`EdgeStore`) and `crates/velesdb-core/src/collection/graph_collection.rs` (`GraphCollection`). Decision recorded in `docs/planning/CORE_PARITY_REMEDIATION.md` (T4).

The mobile SDK's `MobileGraphStore` is a self-contained, purely in-memory graph
engine (node/edge maps plus BFS/DFS/degree/label helpers) rather than a delegate
to `velesdb-core`'s graph runtime. This is an intentional design decision, **not**
an accidental rewrite: mobile needs a RAM-only graph with no filesystem path,
WAL, or on-disk payloads, whereas core's `GraphCollection` persists nodes as JSON
payloads and requires a path. Core today exposes an in-memory `EdgeStore` (edges
+ traversal) but has **no in-memory `GraphNode`-object store** (label + properties
+ vector CRUD), so the node half of `MobileGraphStore` has no core API to delegate
to.

What *is* single-sourced is the record types: `MobileGraphStore` is pinned to
core via `From<velesdb_core::GraphNode / GraphEdge / TraversalResult>` conversions
(`graph.rs`), so any field drift in the core types is a compile error in mobile —
the type-shadowing risk (the in-scope half of T4) is closed.

**User impact**: none functionally — the mobile graph API behaves as documented.
The architectural caveat is that mobile graph semantics are maintained
independently of core's graph engine. Full delegation is gated on core first
shipping an in-memory `GraphStore` API (node + edge CRUD, label query, cascade
remove, BFS/DFS); until then, copying core's graph engine into the MIT-licensed
mobile crate would violate the Core License boundary, so the fork is the correct
boundary-preserving choice.

---

## Reading this document

Each entry states:

- **Status**: open / partial / documented / resolved / pre-existing.
- **Source**: the file or line referenced in code.
- **User impact**: what an operator or integrator actually sees.
- **Resolution path or workaround** where applicable.

For product-level scope boundaries (single-writer, no replication, RBAC scope, WASM hop-limit, benchmark infrastructure), see the [README "Known Limitations" section](../../README.md#known-limitations).
