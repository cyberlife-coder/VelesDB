# velesdb-core

[![Crates.io](https://img.shields.io/crates/v/velesdb-core.svg)](https://crates.io/crates/velesdb-core)
[![Documentation](https://docs.rs/velesdb-core/badge.svg)](https://docs.rs/velesdb-core)
[![License](https://img.shields.io/crates/l/velesdb-core.svg)](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE)

High-performance vector database engine written in Rust.

## Features

- **Blazing Fast**: HNSW index with SIMD-optimized distance calculations
- **Persistent Storage**: Memory-mapped files for efficient disk access
- **Multiple Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
- **Metadata Filtering**: Filter search results by payload attributes
- **VelesQL**: SQL-like query language for vector operations

## Installation

```bash
cargo add velesdb-core
```

## Quick Start

```rust
use velesdb_core::{Database, DistanceMetric};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new database
    let db = Database::open("./my_vectors")?;

    // Create a collection with 384-dimensional vectors
    let collection = db.create_collection("documents", 384, DistanceMetric::Cosine)?;

    // Insert vectors with metadata
    collection.upsert(vec![
        (1, vec![0.1; 384], json!({"title": "Hello World", "category": "greeting"})),
        (2, vec![0.2; 384], json!({"title": "Rust Programming", "category": "tech"})),
    ])?;

    // Search for similar vectors
    let query = vec![0.15; 384];
    let results = collection.search(&query, 5)?;

    for result in results {
        println!("ID: {}, Score: {:.4}", result.id, result.score);
    }

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

- Search latency: **< 1ms** for 100k vectors
- Insert throughput: **> 50k vectors/sec**
- Memory efficient with quantization support

## License

Business Source License 1.1 (BSL-1.1)

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
