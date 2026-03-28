# velesdb-core

[![Crates.io](https://img.shields.io/crates/v/velesdb-core.svg)](https://crates.io/crates/velesdb-core)
[![Documentation](https://docs.rs/velesdb-core/badge.svg)](https://docs.rs/velesdb-core)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0-blue)](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/cyberlife-coder/VelesDB/ci.yml?branch=main)](https://github.com/cyberlife-coder/VelesDB/actions)

High-performance vector database engine written in Rust.

## Features

- **Blazing Fast**: Native HNSW with AVX-512/AVX2/NEON SIMD (47.0µs search at 768D, 19.8ns dot product 768D)
- **Adaptive Search**: Two-phase ef_search that auto-escalates only for hard queries (2-4x faster median)
- **Hybrid Search**: Combine vector similarity + BM25 full-text search with RRF fusion
- **Sparse Vectors**: Named sparse vector indexes with DAAT MaxScore search and RRF/RSF fusion
- **Streaming Inserts**: Bounded-channel ingestion with backpressure and insert-and-search via delta buffer
- **Agent Memory SDK**: Semantic, Episodic, and Procedural memory with TTL, snapshots, and reinforcement
- **Query Plan Cache**: Two-tier LRU cache with write-generation invalidation for repeated queries
- **Persistent Storage**: Memory-mapped files for efficient disk access
- **Multiple Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
- **ColumnStore Filtering**: Up to 130x faster than JSON filtering at scale (integer equality, 100K rows; string equality up to 75x)
- **VelesQL**: SQL-like query language with MATCH support for graph pattern queries
- **Bulk Operations**: Optimized batch insert with turbo/fast modes and parallel HNSW indexing
- **Quantization**: SQ8 (4x), Binary (32x), Product Quantization (8-32x), RaBitQ compression

## Installation

```bash
cargo add velesdb-core
```

## Quick Start

```rust
use velesdb_core::{Database, DistanceMetric, Point, StorageMode};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new database
    let db = Database::open("./my_vectors")?;

    // Create a collection with 384-dimensional vectors (Cosine similarity)
    db.create_collection("documents", 384, DistanceMetric::Cosine)?;

    // Get the collection handle
    let collection = db.get_vector_collection("documents")
        .ok_or("Collection not found")?;

    // Insert vectors with metadata (upsert takes ownership)
    let points = vec![
        Point::new(1, vec![0.1; 384], Some(json!({"title": "Hello World", "category": "greeting"}))),
        Point::new(2, vec![0.2; 384], Some(json!({"title": "Rust Programming", "category": "tech"}))),
    ];
    collection.upsert(points)?;

    // Vector similarity search
    let query = vec![0.15; 384];
    let results = collection.search(&query, 5)?;

    for result in results {
        println!("ID: {}, Score: {:.4}", result.point.id, result.score);
    }

    // Hybrid search (vector + full-text with RRF fusion)
    let hybrid_results = collection.hybrid_search(
        &query,
        "rust programming",
        5,
        Some(0.7) // 70% vector, 30% text
    )?;

    // BM25 full-text search only
    let text_results = collection.text_search("rust programming", 10)?;

    // Fast search (IDs + scores only, no payload retrieval)
    let fast_results = collection.search_ids(&query, 10)?;
    for result in fast_results {
        println!("ID: {}, Score: {:.4}", result.id, result.score);
    }

    Ok(())
}
```

## Distance Metrics

All 5 metrics are available via `DistanceMetric` enum:

```rust
use velesdb_core::DistanceMetric;

// Text embeddings (normalized vectors)
let cosine = DistanceMetric::Cosine;

// Image features, spatial data
let euclidean = DistanceMetric::Euclidean;

// Pre-normalized vectors, MIPS
let dot = DistanceMetric::DotProduct;

// Binary vectors, fingerprints, LSH
let hamming = DistanceMetric::Hamming;

// Set similarity, sparse vectors, tags
let jaccard = DistanceMetric::Jaccard;
```

| Metric | Use Case | Score Interpretation |
|--------|----------|---------------------|
| `Cosine` | Text embeddings | Higher = more similar |
| `Euclidean` | Spatial data | Lower = more similar |
| `DotProduct` | MIPS, pre-normalized | Higher = more similar |
| `Hamming` | Binary vectors | Lower = more similar |
| `Jaccard` | Set similarity | Higher = more similar |

### Common Embedding Dimensions

| Model | Dimension | Metric |
|-------|-----------|--------|
| OpenAI `text-embedding-3-small` | 1536 | Cosine |
| OpenAI `text-embedding-3-large` | 3072 | Cosine |
| Sentence-Transformers `all-MiniLM-L6-v2` | 384 | Cosine |
| Cohere `embed-english-v3.0` | 1024 | Cosine |
| BAAI `bge-large-en-v1.5` | 1024 | Cosine |
| CLIP (image+text) | 512 or 768 | Cosine |

The `dimension` parameter must match your embedding model's output size exactly.

## Bulk Operations

For high-throughput import (3.8K-6.4K vectors/sec at Collection level with persistence, 768D):

```rust
use velesdb_core::{Database, DistanceMetric, Point};

let db = Database::open("./data")?;
db.create_collection("bulk_test", 768, DistanceMetric::Cosine)?;
let collection = db.get_vector_collection("bulk_test")
    .ok_or("Collection not found")?;

// Generate 10,000 vectors
let points: Vec<Point> = (0..10_000)
    .map(|i| Point::without_payload(i, vec![0.1; 768]))
    .collect();

// Bulk insert with parallel HNSW indexing
let inserted = collection.upsert_bulk(&points)?;
println!("Inserted {} vectors", inserted);

// Explicit flush for durability (optional)
collection.flush()?;
```

### Durability semantics

- `store`/`upsert` update in-memory/WAL state for performance.
- `flush()` is the explicit durability barrier for crash-consistent persistence.
- Destructor-based cleanup is best-effort and should not be used as a commit boundary.

## Memory-Efficient Storage (Quantization)

```rust
use velesdb_core::{Database, DistanceMetric, StorageMode};

let db = Database::open("./data")?;

// SQ8: 4x memory reduction, ~1% recall loss
db.create_collection_with_options(
    "sq8_collection",
    768,
    DistanceMetric::Cosine,
    StorageMode::SQ8
)?;

// Binary: 32x memory reduction, ~10-15% recall loss (IoT/Edge)
db.create_collection_with_options(
    "binary_collection",
    768,
    DistanceMetric::Hamming,
    StorageMode::Binary
)?;

// Product Quantization: variable compression
db.create_collection_with_options(
    "pq_collection",
    768,
    DistanceMetric::Cosine,
    StorageMode::ProductQuantization
)?;

// RaBitQ: randomized binary quantization
db.create_collection_with_options(
    "rabitq_collection",
    768,
    DistanceMetric::Cosine,
    StorageMode::RaBitQ
)?;
```

## Performance

### Vector Operations (768D)

| Operation | Time | Throughput |
|-----------|------|------------|
| Dot Product | **19.8 ns** | 38.8 Gelem/s |
| Euclidean Distance | **22.5 ns** | 34.1 Gelem/s |
| Cosine Similarity | **33.1 ns** | 23.2 Gelem/s |
| Hamming Distance | **35.8 ns** | — |
| Jaccard Similarity | **35.1 ns** | — |

*Measured March 27, 2026 on Intel Core i9-14900KF, 64GB DDR5, Rust 1.92.0, AVX2, `--release`, sequential on idle machine.*

### System Benchmarks (10K vectors, 768D)

| Benchmark | Result |
|-----------|--------|
| **HNSW Search** | **47.0 µs** (k=10, Balanced mode) |
| **VelesQL Cache Hit** | **1.08 µs** (~926K QPS) |
| **Sparse Search** | **813 µs** (MaxScore DAAT) |
| **Recall@10 (Accurate)** | **100%** |

*Measured March 27, 2026 on Intel Core i9-14900KF, 64GB DDR5, Rust 1.92.0, AVX2, `--release`, sequential on idle machine.*

### Key Performance Features

- Search latency: **47.0µs** for 10K/768D vectors (k=10)
- Insert throughput: **3.8-7x faster** than pgvector (10K-100K vectors, Docker benchmark v0.7.3, [benchmark](../../benchmarks/README.md))
- ColumnStore filtering: up to 130x faster than JSON scanning at scale (integer equality, 100K rows)

### Recall by Configuration (Native Rust, Criterion)

| Config | Mode | ef_search | Recall@10 | Latency P50 | Status |
|--------|------|-----------|-----------|-------------|--------|
| **10K/128D** | Balanced | 128 | **98.8%** | 57µs | ✅ |
| **10K/128D** | Accurate (ef=512) | 512 | **99.9%** | 130µs | ✅ |
| **10K/128D** | Perfect (ef=4096) | 4096 | **100%** | 200µs | ✅ |
| **10K/128D** | Adaptive | 32-512 | **95%+** | ~40µs (easy) | ✅ |

> *Latency P50 = median over 100 queries. Measured March 27, 2026. The headline "47.0µs" is for 10K/768D Balanced — higher dimensions use SIMD more efficiently. 128D benchmarks above are worst-case for recall measurement.*

> 📊 **Benchmark kit:** See [benchmarks/](../../benchmarks/) for reproducible tests.

## Understanding Collections & Metrics

### Metric is Set at Collection Level

VelesDB is **not** a relational database. Each collection has:
- **ONE vector column** with a fixed dimension
- **ONE distance metric** (immutable after creation)
- **JSON metadata** (payload) for each point

```rust
// Create collection with Cosine metric (for text embeddings)
db.create_collection("documents", 768, DistanceMetric::Cosine)?;

// Create collection with Hamming metric (for binary vectors)
db.create_collection("fingerprints", 256, DistanceMetric::Hamming)?;

// The metric is fixed - you cannot change it after creation
// To use a different metric, create a new collection
```

### Metadata (Payload) Format

Metadata is stored as **JSON** (`serde_json::Value`). Any valid JSON structure is supported:

```rust
use serde_json::json;

// Simple flat metadata
let point1 = Point::new(1, vector, Some(json!({
    "title": "Hello World",
    "category": "greeting",
    "views": 1500,
    "published": true
})));

// Nested metadata
let point2 = Point::new(2, vector, Some(json!({
    "title": "Rust Guide",
    "author": {
        "name": "Alice",
        "email": "alice@example.com"
    },
    "tags": ["rust", "programming", "tutorial"],
    "stats": {
        "views": 5000,
        "likes": 120
    }
})));

// No metadata
let point3 = Point::without_payload(3, vector);
```

### Querying with VelesQL

VelesQL is a SQL-like query language. The distance metric is **always** the one defined at collection creation.

> **JOIN runtime limit:** `JOIN ... USING (...)` currently supports **one column only**.  
> Multi-column `USING (a, b, ...)` is parsed but rejected at execution time.

```sql
-- Vector similarity search
SELECT * FROM docs WHERE VECTOR NEAR [0.1, 0.2, ...] LIMIT 5;

-- With parameter (for API)
SELECT * FROM docs WHERE VECTOR NEAR $query LIMIT 10;

-- Full-text search (BM25)
SELECT * FROM docs WHERE content MATCH 'rust programming' LIMIT 10;

-- Hybrid (vector + text)
SELECT * FROM docs 
WHERE VECTOR NEAR $query AND content MATCH 'rust'
LIMIT 5;
```

### Querying Metadata

Metadata fields can be filtered with standard SQL operators:

```sql
-- Equality
SELECT * FROM docs WHERE category = 'tech' LIMIT 10;

-- Comparison operators
SELECT * FROM docs WHERE views > 1000 LIMIT 10;
SELECT * FROM docs WHERE price >= 50 AND price <= 200 LIMIT 10;

-- String patterns
SELECT * FROM docs WHERE title LIKE '%rust%' LIMIT 10;

-- IN list
SELECT * FROM docs WHERE category IN ('tech', 'science', 'ai') LIMIT 10;

-- BETWEEN (inclusive)
SELECT * FROM docs WHERE score BETWEEN 0.5 AND 1.0 LIMIT 10;

-- NULL checks
SELECT * FROM docs WHERE author IS NOT NULL LIMIT 10;

-- Combine vector + metadata filters
SELECT * FROM docs 
WHERE VECTOR NEAR [0.1, 0.2, ...] 
AND category = 'tech' 
AND views > 100
LIMIT 5;
```

### WITH Clause (Query Options)

Override search parameters on a per-query basis:

```sql
-- Set search mode
SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10
WITH (mode = 'accurate');

-- Set ef_search and timeout
SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10
WITH (ef_search = 512, timeout_ms = 5000);
```

| Option | Type | Description |
|--------|------|-------------|
| `mode` | string | fast, balanced, accurate, perfect, adaptive |
| `ef_search` | integer | HNSW ef_search (higher = better recall) |
| `timeout_ms` | integer | Query timeout in milliseconds |
| `rerank` | boolean | Enable result reranking |

### Available Filter Operators

| Operator | SQL Syntax | Example |
|----------|------------|---------|
| Equal | `=` | `category = 'tech'` |
| Not Equal | `!=` or `<>` | `status != 'draft'` |
| Greater Than | `>` | `views > 1000` |
| Greater or Equal | `>=` | `price >= 50` |
| Less Than | `<` | `score < 0.5` |
| Less or Equal | `<=` | `rating <= 3` |
| IN | `IN (...)` | `tag IN ('a', 'b')` |
| BETWEEN | `BETWEEN ... AND` | `age BETWEEN 18 AND 65` |
| LIKE | `LIKE` | `name LIKE '%john%'` |
| IS NULL | `IS NULL` | `email IS NULL` |
| IS NOT NULL | `IS NOT NULL` | `phone IS NOT NULL` |
| Full-text | `MATCH` | `content MATCH 'rust'` |

## Sparse Vector Search

VelesDB supports sparse vectors (e.g., SPLADE, BM25 term weights) alongside dense embeddings.
You can store named sparse vectors per point, search them independently, or combine dense+sparse
results using Reciprocal Rank Fusion (RRF).

### Upserting points with sparse vectors

```rust
use std::collections::BTreeMap;
use velesdb_core::{Database, DistanceMetric, Point};
use velesdb_core::sparse_index::SparseVector;

let db = Database::open("./data")?;
db.create_collection("docs", 768, DistanceMetric::Cosine)?;
let collection = db.get_vector_collection("docs")
    .ok_or("Collection not found")?;

// Build a sparse vector from (term_index, weight) pairs
let sparse = SparseVector::new(vec![
    (42, 1.2),   // term 42, weight 1.2
    (187, 0.8),  // term 187, weight 0.8
    (1024, 0.3),
]);

// Attach named sparse vectors to a point
let mut sparse_map = BTreeMap::new();
sparse_map.insert("".to_string(), sparse); // "" = default sparse index

let point = Point::with_sparse(
    1,
    vec![0.1; 768],                          // dense embedding
    Some(serde_json::json!({"title": "My doc"})),
    Some(sparse_map),
);
collection.upsert(vec![point])?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Sparse-only search (DAAT MaxScore)

The sparse search engine uses a DAAT (Document-At-A-Time) MaxScore algorithm for fast
top-k retrieval by inner product. It automatically falls back to linear scan for
high-coverage queries.

```rust
# use velesdb_core::sparse_index::SparseVector;
// Build a query with term weights
let query = SparseVector::new(vec![(42, 1.0), (187, 0.5)]);

// Search the default sparse index for top-5 results
let results = collection.sparse_search(&query, 5, "")?;
for result in &results {
    println!("ID: {}, Score: {:.4}", result.point.id, result.score);
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Hybrid dense+sparse with RRF fusion

Combine dense vector search (HNSW) with sparse term matching. Both branches run
in parallel via rayon, then results are fused using Reciprocal Rank Fusion (RRF) or
Relative Score Fusion (RSF).

```rust
# use velesdb_core::sparse_index::SparseVector;
# use velesdb_core::FusionStrategy;
let dense_query = vec![0.15; 768];
let sparse_query = SparseVector::new(vec![(42, 1.0), (187, 0.5)]);

// RRF fusion with default k=60
let strategy = FusionStrategy::rrf_default();
let results = collection.hybrid_sparse_search(
    &dense_query,
    &sparse_query,
    10,         // top-k
    "",         // default sparse index
    &strategy,
)?;

for result in &results {
    println!("ID: {}, Fused score: {:.4}", result.point.id, result.score);
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

You can also use `RelativeScore` fusion for explicit weight control:

```rust
# use velesdb_core::FusionStrategy;
// 70% dense, 30% sparse (validated constructor)
let strategy = FusionStrategy::relative_score(0.7, 0.3)?;
```

### Fusion types and parameters

| Type | Path | Description |
|------|------|-------------|
| `SparseVector` | `velesdb_core::sparse_index` | Sorted `(u32 index, f32 weight)` pairs; deduplicates and filters zeros on construction |
| `FusionStrategy` | `velesdb_core` | `RRF { k }`, `RelativeScore { dense_weight, sparse_weight }` |
| `ScoredDoc` | `velesdb_core::sparse_index` | Raw sparse search result: `doc_id: u64`, `score: f32` |

| Method | On | Description |
|--------|-----|-------------|
| `sparse_search(query, k, index_name)` | `VectorCollection` | Sparse search on the given index (`""` for default) |
| `hybrid_sparse_search(dense, sparse, k, index_name, strategy)` | `VectorCollection` | Dense + sparse with fusion |

## Streaming Inserts

For high-throughput, continuously arriving data (IoT sensors, live embeddings, log streams),
`StreamIngester` provides a bounded-channel ingestion pipeline with automatic micro-batch
flushing and backpressure signaling.

### Basic usage

```rust,no_run
use velesdb_core::collection::streaming::{StreamIngester, StreamingConfig};
use velesdb_core::Point;

// Configure the pipeline
let config = StreamingConfig {
    buffer_size: 10_000,     // channel capacity (backpressure threshold)
    batch_size: 128,         // flush every 128 points
    flush_interval_ms: 50,   // or every 50ms, whichever comes first
};

// `collection` is a Collection obtained from db.get_vector_collection(...)
let ingester = StreamIngester::new(collection, config);

// Send points — returns immediately
let point = Point::new(1, vec![0.1; 384], None);
match ingester.try_send(point) {
    Ok(()) => { /* accepted */ }
    Err(e) => eprintln!("Backpressure: {e}"),
}

// Gracefully drain remaining points before shutdown
ingester.shutdown().await;
```

### Backpressure

`try_send` is non-blocking. When the bounded channel is at capacity, it returns
`BackpressureError::BufferFull` -- the caller should retry after a short delay or
drop the point. If the background drain task exits unexpectedly, `DrainTaskDead` is
returned.

### Delta buffer (insert-and-search)

During an HNSW rebuild, newly inserted vectors are not yet in the index. The delta buffer
accumulates these vectors and merges them into search results via brute-force scan, so
freshly inserted data is searchable immediately without waiting for the rebuild to complete.

```rust,ignore
// The delta buffer is managed automatically by the streaming pipeline.
// When active, search results transparently include delta-buffered vectors.
let results = collection.search(&query, 10)?;
// ^ includes both HNSW-indexed and delta-buffered vectors
```

## Agent Memory Patterns

The Agent Memory SDK provides three memory subsystems designed for AI agent workloads:
chatbots, RAG pipelines, and autonomous learning agents. Each memory type is backed by
VelesDB collections with vector similarity search, TTL-based expiration, and snapshot
persistence.

### Initialization

```rust,no_run
use std::sync::Arc;
use velesdb_core::Database;
use velesdb_core::agent::AgentMemory;

let db = Arc::new(Database::open("./agent_data")?);
let memory = AgentMemory::new(Arc::clone(&db))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Semantic Memory (long-term knowledge)

Stores facts as vector embeddings for similarity-based retrieval. Use this for RAG
knowledge bases, persistent world knowledge, or any data your agent should "know" long-term.

```rust,ignore
// Store a fact
let embedding = vec![0.1; 384]; // from your embedding model
memory.semantic().store(1, "Paris is the capital of France", &embedding)?;

// Query by similarity
let query_embedding = vec![0.12; 384];
let results = memory.semantic().query(&query_embedding, 5)?;
for (id, score, content) in &results {
    println!("[{score:.3}] {content}");
}
```

### Episodic Memory (event timeline)

Records events with timestamps for temporal and similarity-based retrieval. Use this
for conversation history, user interaction logs, or any time-sequenced data.

```rust,ignore
// Record an event
let timestamp = 1710000000_i64; // Unix timestamp
let embedding = vec![0.2; 384];
memory.episodic().record(1, "User asked about French geography", timestamp, Some(&embedding))?;

// Retrieve recent events
let recent = memory.episodic().recent(10, None)?;
for (id, description, ts) in &recent {
    println!("[{ts}] {description}");
}

// Recall similar events
let results = memory.episodic().recall_similar(&query_embedding, 5)?;
```

### Procedural Memory (learned patterns)

Stores action sequences with confidence scoring and reinforcement learning. Use this
for agents that learn from experience -- task automation, decision-making, or any
workflow where past success/failure should influence future behavior.

```rust,ignore
// Learn a procedure
let steps = vec!["parse query".into(), "search index".into(), "format results".into()];
let embedding = vec![0.3; 384];
memory.procedural().learn(1, "answer_question", &steps, Some(&embedding), 0.8)?;

// Recall matching procedures (min confidence 0.5)
let matches = memory.procedural().recall(&query_embedding, 5, 0.5)?;
for m in &matches {
    println!("{} (confidence: {:.2}): {:?}", m.name, m.confidence, m.steps);
}

// Reinforce after success/failure
memory.procedural().reinforce(1, true)?;  // increases confidence
memory.procedural().reinforce(1, false)?; // decreases confidence
```

### TTL, eviction, and snapshots

```rust,ignore
// Set TTL on individual entries
memory.set_semantic_ttl(1, 3600);  // expires in 1 hour
memory.set_episodic_ttl(2, 86400); // expires in 24 hours

// Run periodic expiration
let stats = memory.auto_expire()?;
println!("Expired: {} semantic, {} episodic", stats.semantic_expired, stats.episodic_expired);

// Evict low-confidence procedures
let evicted = memory.evict_low_confidence_procedures(0.3)?;

// Snapshot and restore
let memory = memory
    .with_snapshots("./snapshots", 5)  // keep last 5 snapshots
    .with_eviction_config(EvictionConfig::default());

let version = memory.snapshot()?;
memory.load_snapshot_version(version)?;
```

### When to use each memory type

| Memory Type | Use Case | Example |
|-------------|----------|---------|
| **Semantic** | Persistent knowledge that rarely changes | RAG knowledge base, world facts, documentation |
| **Episodic** | Time-sequenced events and interactions | Chat history, user sessions, audit logs |
| **Procedural** | Learned behaviors that improve over time | Task automation, decision trees, API call patterns |

### Agent Memory types

| Type | Description |
|------|-------------|
| `AgentMemory` | Unified interface; holds `SemanticMemory`, `EpisodicMemory`, `ProceduralMemory` |
| `SemanticMemory` | `store(id, content, embedding)`, `query(embedding, k)` returns `Vec<(id, score, content)>` |
| `EpisodicMemory` | `record(id, description, timestamp, embedding)`, `recent(limit, since)`, `recall_similar(embedding, k)` |
| `ProceduralMemory` | `learn(id, name, steps, embedding, confidence)`, `recall(embedding, k, min_confidence)`, `reinforce(id, success)` |
| `ProcedureMatch` | Result struct: `id`, `name`, `steps: Vec<String>`, `confidence: f32`, `score: f32` |


| `EvictionConfig` | `consolidation_age_threshold: u64`, `min_confidence_threshold: f32`, `max_entries_per_cycle: usize` |
| `SnapshotManager` | `new(dir, max_snapshots)` -- versioned state persistence with automatic rotation |
| `ExpireResult` | Returned by `auto_expire()`: `semantic_expired`, `episodic_expired`, `episodic_consolidated` counts |

Default embedding dimension is **384** (configurable via `AgentMemory::with_dimension(db, dim)`).

## Query Plan Cache

VelesDB automatically caches compiled query plans in a two-tier LRU cache (L1 lock-free +
L2 LRU). Repeated queries skip parsing and planning entirely when the cache key matches.

### How it works

- **Automatic**: The cache is enabled by default on every `Database` instance. No configuration
  required.
- **Write-generation invalidation**: Each collection tracks a monotonic write generation counter.
  When data is inserted, updated, or deleted, the generation increments. Cached plans whose
  key includes a stale generation are automatically bypassed -- no explicit invalidation needed.
- **LRU eviction**: The cache has bounded capacity. Least-recently-used plans are evicted when
  the cache is full.

### Inspecting cache behavior with EXPLAIN

The `EXPLAIN` output includes `cache_hit` and `plan_reuse_count` fields that show whether
a query plan was served from the cache:

```sql
EXPLAIN SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10;
```

```json
{
  "query": "SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10",
  "query_type": "SELECT",
  "collection": "docs",
  "plan": [
    { "step": 1, "operation": "VectorSearch", "description": "HNSW search k=10 ef=100", "estimated_rows": 10 }
  ],
  "estimated_cost": {
    "uses_index": true,
    "index_name": "Hnsw",
    "selectivity": 0.001,
    "complexity": "O(log N)"
  },
  "features": {
    "has_vector_search": true,
    "has_filter": false,
    "has_order_by": false,
    "has_group_by": false,
    "has_aggregation": false,
    "has_join": false,
    "has_fusion": false,
    "limit": 10,
    "offset": null
  },
  "cache_hit": true,
  "plan_reuse_count": 42
}
```

- `cache_hit: true` -- the plan was found in cache (parsing and planning were skipped).
- `cache_hit: false` -- cache miss; a fresh plan was compiled and inserted into the cache.
- `plan_reuse_count` -- how many times this cached plan has been reused across all callers.

### Cache metrics

```rust,ignore
let metrics = db.plan_cache().metrics();
println!("Hit rate: {:.1}%", metrics.hit_rate() * 100.0);
println!("Hits: {}, Misses: {}", metrics.hits(), metrics.misses());
```

### Cache types and parameters

| Type | Path | Description |
|------|------|-------------|
| `CompiledPlanCache` | `velesdb_core::cache` | Two-tier cache (L1 lock-free DashMap + L2 LRU). Default: 1K L1 / 10K L2 entries |
| `PlanKey` | `velesdb_core::cache` | Cache key: `query_hash: u64`, `schema_version: u64`, `collection_generations: SmallVec<[u64; 4]>` |
| `CompiledPlan` | `velesdb_core::cache` | Cached plan: `plan: QueryPlan`, `referenced_collections: Vec<String>`, `reuse_count: AtomicU64` |
| `PlanCacheMetrics` | `velesdb_core::cache` | `hits()`, `misses()`, `hit_rate() -> f64` (ratio 0.0--1.0) |

| Method | On | Description |
|--------|-----|-------------|
| `plan_cache()` | `Database` | Returns `&CompiledPlanCache` |
| `plan_cache().metrics()` | `CompiledPlanCache` | Returns `&PlanCacheMetrics` |
| `plan_cache().stats()` | `CompiledPlanCache` | Returns `LockFreeCacheStats` (L1/L2 sizes, hit counts) |

## Public API Reference

```rust
// Core types
use velesdb_core::{
    Database,           // Database instance
    Collection,         // Vector collection
    Point,              // Vector with metadata
    DistanceMetric,     // Cosine, Euclidean, DotProduct, Hamming, Jaccard
    StorageMode,        // Full, SQ8, Binary, ProductQuantization, RaBitQ
    Error, Result,      // Error types
};

// Sparse vectors and fusion
use velesdb_core::sparse_index::SparseVector; // Sparse vector (indices + weights)
use velesdb_core::FusionStrategy;             // RRF, RelativeScore, Average, Maximum, Weighted

// Streaming ingestion
use velesdb_core::collection::streaming::{
    StreamIngester,     // Bounded-channel ingestion pipeline
    StreamingConfig,    // Buffer size, batch size, flush interval
    BackpressureError,  // BufferFull, NotConfigured, DrainTaskDead
};

// Agent memory
use velesdb_core::agent::{
    AgentMemory,        // Unified memory interface (semantic + episodic + procedural)
    SemanticMemory,     // Long-term knowledge storage
    EpisodicMemory,     // Event timeline with temporal queries
    ProceduralMemory,   // Learned patterns with reinforcement
    ProcedureMatch,     // Recall result with confidence and steps
    EvictionConfig,     // TTL and eviction policies
    SnapshotManager,    // Versioned snapshot persistence
    TemporalIndex,      // B-tree temporal index for O(log N) time queries
};

// Index types
use velesdb_core::{
    HnswIndex,          // HNSW index
    HnswParams,         // Index parameters
    SearchQuality,      // Fast, Balanced, Accurate, Perfect, Custom, Adaptive
};

// Query plan cache
use velesdb_core::cache::{
    CompiledPlanCache,  // Two-tier LRU cache for compiled query plans
    PlanCacheMetrics,   // Hit/miss counters with hit_rate()
    PlanKey,            // Deterministic cache key (query hash + write generation)
};

// Filtering
use velesdb_core::{Filter, Condition};

// Quantization
use velesdb_core::{QuantizedVector, BinaryQuantizedVector, QuantizationConfig};

// Metrics
use velesdb_core::{recall_at_k, precision_at_k, mrr, ndcg_at_k};
```

## License

VelesDB Core License 1.0

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
