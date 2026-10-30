# Core: public API map

Where to find what in `velesdb_core`. This is an import map, not a signature
reference — full signatures, argument names and return types are generated on
[docs.rs/velesdb-core](https://docs.rs/velesdb-core) and are the authority.

Moved out of `crates/velesdb-core/README.md` to keep that file under the
400-line documentation budget.

---

## Database, collections, points

```rust
use velesdb_core::{
    Database,            // database handle (open, create/get collections, execute_query)
    VectorCollection,    // vector collection (typed handle)
    GraphCollection,     // graph collection (typed handle)
    MetadataCollection,  // metadata-only collection (typed handle)
    AnyCollection,       // type-erased handle: is_*/as_*/into_* plus shared ops
    Point,               // id + vector + optional JSON payload
    SearchResult,        // point + score
    ScoredResult,        // id + score (payload-free fast path)
    DistanceMetric,      // Cosine, Euclidean, DotProduct, Hamming, Jaccard
    StorageMode,         // Full, SQ8, Binary, ProductQuantization, RaBitQ
    Error, Result,       // error types
};
```

`Collection` itself is `pub(crate)` since v1.13 — the typed split above is the
public API.

## Graph engine

```rust
use velesdb_core::{
    GraphEdge, GraphNode, EdgeType, NodeType, GraphSchema,
    TraversalConfig, TraversalPath, TraversalResult,
};
```

## VelesQL

```rust
use velesdb_core::velesql::Parser;   // parse a VelesQL statement
// then run it with Database::execute_query
```

## Sparse vectors and fusion

```rust
use velesdb_core::sparse_index::SparseVector; // (index, weight) pairs
use velesdb_core::FusionStrategy;             // RRF, RelativeScore, ...
use velesdb_core::FusionParams;               // FusionStrategy::from_name(name, params): the name grammar
```

## Streaming ingestion

```rust
use velesdb_core::collection::streaming::{
    StreamIngester,     // bounded-channel ingestion pipeline
    StreamingConfig,    // buffer size, batch size, flush interval
    BackpressureError,  // BufferFull, NotConfigured, DrainTaskDead
};
```

## Agent memory (requires the `persistence` feature)

```rust
use velesdb_core::agent::{
    AgentMemory,        // unified interface (semantic + episodic + procedural)
    SemanticMemory,     // long-term knowledge
    EpisodicMemory,     // event timeline with temporal queries
    ProceduralMemory,   // learned patterns with reinforcement
    ProcedureMatch,     // recall result with confidence and steps
    EvictionConfig,     // TTL and eviction policy
    ExpireResult,       // auto_expire() report
    SnapshotManager,    // versioned snapshot persistence
    TemporalIndex,      // B-tree temporal index, O(log N) time queries
    BoundedRelations,   // relations_bounded() result: edges + truncated
};
```

`SemanticMemory::query_with_quality`, `query_filtered_with_quality` and
`query_excluding_with_quality` (7.1.0) take a `SearchQuality` per call; see
[Agent Memory: explicit quality](./AGENT_MEMORY.md#semantic-search-at-an-explicit-quality).

## Indexes

```rust
use velesdb_core::{
    HnswIndex,       // HNSW index
    HnswParams,      // index parameters
    SearchQuality,   // Fast, Balanced, Accurate, Perfect, Custom, Adaptive, AutoTune
    VectorIndex,     // index trait
    IndexInfo,                // VectorCollection::list_indexes() entry
    OrderByIndexSuggestion,   // VectorCollection::order_by_index_advice() entry
    OrderByIndexState,        // Missing | BuiltButUncovered
};
use velesdb_core::api_types::{
    parse_search_mode,        // "fast" | ... | "custom:<ef>" | "adaptive:<min>:<max>" -> SearchQuality
    mode_to_search_quality,   // lenient form: None instead of Err
    validate_ef_search,       // [MIN_EF_SEARCH, MAX_EF_SEARCH] = [16, 4096]
    MIN_EF_SEARCH, MAX_EF_SEARCH,
};
use velesdb_core::collection::auto_reindex::{AutoReindexConfig, AutoReindexManager, DivergenceCheck};
use velesdb_core::collection::streaming::{AsyncIndexBuilderConfig, DeferredIndexerConfig};
```

Usage: [Search modes](./SEARCH_MODES.md#when-the-two-phases-run) for the
mode parser, [Core: operations API](./CORE_OPERATIONS_API.md#index-introspection-and-maintenance)
for introspection, `apply_advanced_config` and auto-reindex.

## Control plane (observer, governed reads)

```rust
use velesdb_core::{
    DatabaseObserver,      // hooks; every method has a default
    AccessDecision,        // Allow | Deny(Error) | AllowWithScope(AccessScope)
    AccessScope,           // tenant hint + VelesQL filter AND-ed into the read
    QueryAccessContext,    // collection, operation, principal, tenant_hint
    QueryOperationKind,    // VectorSearch, TextSearch, HybridSearch, GraphTraversal, Select
    GatedRead,             // Dense | Text | Hybrid, for Database::gated_search
};
// Database::open_with_observer, Database::gated_search, Database::authorize_read
```

Usage: [Core: operations API](./CORE_OPERATIONS_API.md#control-plane-observer).

## Guard-rails and validation

```rust
use velesdb_core::QueryLimits;                                 // builder + Database::update_guardrails
use velesdb_core::guardrails::{CircuitBreaker, RateLimiter, GuardRailViolation};
use velesdb_core::{
    validate_collection_name, validate_dimension, validate_dimension_match,
    MAX_COLLECTION_NAME_LENGTH, MIN_DIMENSION, MAX_DIMENSION,
    hash_id, hash_id_bytes, hash_edge_id,                      // canonical FNV-1a ids
};
```

Usage: [Core: operations API](./CORE_OPERATIONS_API.md#guard-rails).

## Query plan cache

```rust
use velesdb_core::cache::{
    CompiledPlanCache,  // two-tier LRU cache for compiled plans
    PlanCacheMetrics,   // hit/miss counters with hit_rate()
    PlanKey,            // deterministic key (query hash + write generation)
};
```

## Filtering

```rust
use velesdb_core::{Filter, Condition};
use velesdb_core::velesql::{
    decide_filter_strategy,              // pre- vs post-filter, shared by executor and EXPLAIN
    set_fallback_selectivity_threshold,  // process-wide, default 0.1
    FilterDecisionMode, FilterStrategy,
    QueryPlanner, ExecutionStrategy,     // hybrid MATCH + NEAR strategy choice
};
```

## Column store

```rust
use velesdb_core::{ColumnStore, ColumnType, ColumnValue, ColumnStoreError, BatchUpsertResult};
use velesdb_core::column_store::{GeoDistanceParams, GeoBboxParams, CompareOp};
```

Usage: [Core: operations API](./CORE_OPERATIONS_API.md#column-store).

## Quantization

```rust
use velesdb_core::{QuantizedVector, BinaryQuantizedVector, QuantizationConfig};
```

## Metrics

```rust
// Retrieval quality (no feature needed).
use velesdb_core::{
    recall_at_k, precision_at_k, mrr, ndcg_at_k,
    hit_rate, mean_average_precision, average_metrics,
};
// Latency and operational counters, Prometheus text export.
use velesdb_core::{
    compute_latency_percentiles, LatencyStats,
    OperationalMetrics, DurationHistogram, TraversalMetrics, GuardRailsMetrics,
};
use velesdb_core::metrics::global_guardrails_metrics;
// Database::graph_metrics_prometheus(), Database::match_metrics_prometheus(),
// GraphCollection::metrics()
```

Usage: [Core: operations API](./CORE_OPERATIONS_API.md#observability); the
server's `/metrics` families: [REST reference](../reference/api-reference.md#get-metrics).

## See also

- [velesdb-core README](../../crates/velesdb-core/README.md)
- [Core: operations API](./CORE_OPERATIONS_API.md) — usage for the control plane, guard-rails, introspection and metrics
- [docs.rs/velesdb-core](https://docs.rs/velesdb-core) — generated API reference

---

Last updated: 2026-10-29 · Applies to: velesdb-core 8.0.0
