# velesdb-core

[![Crates.io](https://img.shields.io/crates/v/velesdb-core.svg)](https://crates.io/crates/velesdb-core)
[![Documentation](https://docs.rs/velesdb-core/badge.svg)](https://docs.rs/velesdb-core)
[![License](https://img.shields.io/badge/license-ELv2-blue)](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/cyberlife-coder/VelesDB/ci.yml?branch=main)](https://github.com/cyberlife-coder/VelesDB/actions)

High-performance vector database engine written in Rust.

## Features

- **Blazing Fast**: Native HNSW with AVX-512/AVX2/NEON SIMD (71µs search, 66ns distance)
- **Hybrid Search**: Combine vector similarity + BM25 full-text search with RRF fusion
- **Persistent Storage**: Memory-mapped files for efficient disk access
- **Multiple Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
- **ColumnStore Filtering**: 122x faster than JSON filtering at scale
- **VelesQL**: SQL-like query language with MATCH support for full-text search
- **Bulk Operations**: Optimized batch insert with parallel HNSW indexing
- **Quantization**: SQ8 (4x) and Binary (32x) memory compression

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
    let collection = db.get_collection("documents")
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
    let text_results = collection.text_search("rust programming", 10);

    // Fast search (IDs + scores only, no payload retrieval)
    let fast_results = collection.search_ids(&query, 10)?;
    for (id, score) in fast_results {
        println!("ID: {id}, Score: {score:.4}");
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

## Bulk Operations

For high-throughput import (3,300+ vectors/sec):

```rust
use velesdb_core::{Database, DistanceMetric, Point};

let db = Database::open("./data")?;
db.create_collection("bulk_test", 768, DistanceMetric::Cosine)?;
let collection = db.get_collection("bulk_test").unwrap();

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

// Binary: 32x memory reduction, ~5-10% recall loss (IoT/Edge)
db.create_collection_with_options(
    "binary_collection",
    768,
    DistanceMetric::Hamming,
    StorageMode::Binary
)?;
```

## Performance

### Vector Operations (768D)

| Operation | Time | Throughput |
|-----------|------|------------|
| Dot Product | **~36 ns** | 28M ops/sec |
| Euclidean Distance | **~46 ns** | 22M ops/sec |
| Cosine Similarity | **~93 ns** | 11M ops/sec |
| Hamming Distance | **~6 ns** | 164M ops/sec |
| Jaccard Similarity | **~160 ns** | 6M ops/sec |

### End-to-End Benchmark (10k vectors, 768D)

| Metric | pgvectorscale | VelesDB | Speedup |
|--------|---------------|---------|---------|
| **Ingest** | 22.3s | **3.0s** | 7.4x |
| **Search Latency** | 52.8ms | **4.0ms** | 13x |
| **Throughput** | 18.9 QPS | **246.8 QPS** | 13x |

### Key Performance Features

- Search latency: **< 5ms** for 10k vectors
- Bulk import: **3,300 vectors/sec** with `upsert_bulk()`
- ColumnStore filtering: **122x faster** than JSON at 100k items

### Recall by Configuration (Native Rust, Criterion)

| Config | Mode | ef_search | Recall@10 | Latency P50 | Status |
|--------|------|-----------|-----------|-------------|--------|
| **10K/128D** | Balanced | 128 | **95.8%** | 0.88ms | ✅ |
| **10K/128D** | HighRecall | 1024 | **99.4%** | 3.0ms | ✅ |
| **10K/128D** | Perfect | 2048 | **100.0%** | 0.61ms | ✅ |
| **100K/768D** | HighRecall | 1024 | **97.0%** | 71.5ms | ✅ ≥95% |
| **100K/768D** | Perfect | 2048 | **100.0%** | 55.4ms | ✅ |

> *Latency P50 = median over 100 queries. ≥95% recall guaranteed for HighRecall mode.*

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
WITH (mode = 'high_recall');

-- Set ef_search and timeout
SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10
WITH (ef_search = 512, timeout_ms = 5000);
```

| Option | Type | Description |
|--------|------|-------------|
| `mode` | string | fast, balanced, accurate, high_recall, perfect |
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

## Public API Reference

```rust
// Core types
use velesdb_core::{
    Database,           // Database instance
    Collection,         // Vector collection
    Point,              // Vector with metadata
    DistanceMetric,     // Cosine, Euclidean, DotProduct, Hamming, Jaccard
    StorageMode,        // Full, SQ8, Binary
    Error, Result,      // Error types
};

// Index types
use velesdb_core::{
    HnswIndex,          // HNSW index
    HnswParams,         // Index parameters
    SearchQuality,      // Fast, Balanced, Accurate, HighRecall
};

// Filtering
use velesdb_core::{Filter, Condition};

// Quantization
use velesdb_core::{QuantizedVector, BinaryQuantizedVector};

// Metrics
use velesdb_core::{recall_at_k, precision_at_k, mrr, ndcg_at_k};
```

## License

Elastic License 2.0 (ELv2)

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
