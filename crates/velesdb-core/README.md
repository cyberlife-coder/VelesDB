# velesdb-core

[![Crates.io](https://img.shields.io/crates/v/velesdb-core.svg)](https://crates.io/crates/velesdb-core)
[![Documentation](https://docs.rs/velesdb-core/badge.svg)](https://docs.rs/velesdb-core)
[![License](https://img.shields.io/badge/license-ELv2-blue)](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/cyberlife-coder/VelesDB/ci.yml?branch=main)](https://github.com/cyberlife-coder/VelesDB/actions)

High-performance vector database engine written in Rust.

## Features

- **Blazing Fast**: HNSW index with explicit SIMD (4x faster than auto-vectorized)
- **Hybrid Search**: Combine vector similarity + BM25 full-text search with RRF fusion
- **Persistent Storage**: Memory-mapped files for efficient disk access
- **Multiple Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
- **ColumnStore Filtering**: 122x faster than JSON filtering at scale
- **VelesQL**: SQL-like query language with MATCH support for full-text search
- **Bulk Operations**: Optimized batch insert with parallel HNSW indexing

## Installation

```bash
cargo add velesdb-core
```

## Quick Start

```rust
use velesdb_core::{Database, DistanceMetric, Point};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new database
    let db = Database::open("./my_vectors")?;

    // Create a collection with 384-dimensional vectors
    let collection = db.create_collection("documents", 384, DistanceMetric::Cosine)?;

    // Insert vectors with metadata
    let points = vec![
        Point::new(1, vec![0.1; 384], Some(json!({"title": "Hello World", "category": "greeting"}))),
        Point::new(2, vec![0.2; 384], Some(json!({"title": "Rust Programming", "category": "tech"}))),
    ];
    collection.upsert(&points)?;

    // Vector similarity search
    let query = vec![0.15; 384];
    let results = collection.search(&query, 5)?;

    for result in results {
        println!("ID: {}, Score: {:.4}", result.point.id, result.score);
    }

    // Hybrid search (vector + full-text)
    let hybrid_results = collection.hybrid_search(
        &query,
        "rust programming",
        5,
        Some(0.7) // 70% vector, 30% text
    )?;

    Ok(())
}
```

## Distance Metrics

| Metric | Use Case |
|--------|----------|
| `Cosine` | Text embeddings, normalized vectors |
| `Euclidean` | Image features, spatial data |
| `DotProduct` | When vectors are pre-normalized |
| `Hamming` | Binary vectors, hash comparisons |
| `Jaccard` | Set similarity, sparse vectors |

## Performance

### Vector Operations (768D)

| Operation | Time | Throughput |
|-----------|------|------------|
| Dot Product | **~39 ns** | 26M ops/sec |
| Euclidean Distance | **~49 ns** | 20M ops/sec |
| Cosine Similarity | **~81 ns** | 12M ops/sec |
| Hamming (Binary) | **~6 ns** | 164M ops/sec |

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
- Memory efficient with SQ8 quantization (4x reduction)

> 📊 **Benchmark kit:** See [benchmarks/](../../benchmarks/) for reproducible tests.

## License

Elastic License 2.0 (ELv2)

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
