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
    AnyCollection,       // type-erased handle from Database::get_any_collection
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
};
```

## Indexes

```rust
use velesdb_core::{
    HnswIndex,       // HNSW index
    HnswParams,      // index parameters
    SearchQuality,   // Fast, Balanced, Accurate, Perfect, Custom, Adaptive
    VectorIndex,     // index trait
};
```

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
```

## Quantization

```rust
use velesdb_core::{QuantizedVector, BinaryQuantizedVector, QuantizationConfig};
```

## Retrieval-quality metrics

```rust
use velesdb_core::{recall_at_k, precision_at_k, mrr, ndcg_at_k};
```

## See also

- [velesdb-core README](../../crates/velesdb-core/README.md)
- [docs.rs/velesdb-core](https://docs.rs/velesdb-core) — generated API reference

---

Last updated: 2026-07-25 · Applies to: velesdb-core 5.2.0
