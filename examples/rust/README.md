# VelesDB Multi-Model Search (Rust)

> **Difficulty: Intermediate** | Showcases: Vector search, VelesQL queries, hybrid search (vector + BM25), text search, ORDER BY similarity

Demonstrates VelesDB's multi-model query capabilities in a single Rust binary: vector similarity, VelesQL with filters, hybrid search, and full-text search.

## What It Does

1. Creates a `documents` collection (384-dim, cosine) and inserts 5 sample documents
2. **Basic vector search** -- find nearest neighbors by embedding
3. **VelesQL with filter** -- SQL-like query restricting results to `category = 'programming'`
4. **ORDER BY similarity** -- VelesQL query sorting by descending similarity score
5. **Hybrid search** -- combine vector similarity with BM25 keyword matching ("rust")
6. **Text search** -- pure BM25 keyword search for "programming"

## Prerequisites

- Rust 1.90+ with Cargo

## How to Run

```bash
cd examples/rust
cargo run --bin multimodel_search
```

## Expected Output

```
=== VelesDB Multi-Model Search Example ===

Inserted 5 documents

--- Example 1: Basic Vector Search ---
  ID: 1, Score: 0.1499, Title: Introduction to Rust
  ID: 5, Score: 0.0867, Title: Building Search Engines
  ID: 4, Score: 0.0676, Title: Machine Learning with Rust

--- Example 2: VelesQL with Similarity ---
  Found 2 results with category='programming'
    ID: 1, Score: 0.1499
    ID: 4, Score: 0.0676

--- Example 3: ORDER BY Similarity ---
  Results ordered by similarity:
    ID: 1, Score: 0.1499
    ID: 5, Score: 0.0867
    ID: 4, Score: 0.0676

--- Example 4: Hybrid Search ---
  Hybrid search results (vector + text 'rust'):
    ID: 1, Score: 0.0167, Title: Introduction to Rust
    ID: 4, Score: 0.0162, Title: Machine Learning with Rust
    ID: 5, Score: 0.0115, Title: Building Search Engines
    ID: 2, Score: 0.0111, Title: Vector Databases Explained
    ID: 3, Score: 0.0109, Title: Graph Algorithms in Practice

--- Example 5: Text Search ---
  Text search results for 'programming':
    ID: 1, Score: 1.2321, Title: Introduction to Rust
    ID: 4, Score: 0.8337, Title: Machine Learning with Rust

=== Example Complete ===
```

The embeddings are generated from fixed seeds, so the result order and IDs are deterministic; the exact score digits may differ by a small amount across platforms.

## VelesDB Features Demonstrated

| Feature | Where |
|---------|-------|
| `Database::open()` | Opens a temporary database |
| `create_collection()` | 384-dim cosine collection |
| `upsert()` | Batch insert with JSON payloads |
| `search()` | K-nearest-neighbor vector search |
| `Parser::parse()` + `execute_query()` | VelesQL with filters and ORDER BY |
| `hybrid_search()` | Vector + BM25 fusion |
| `text_search()` | Pure BM25 keyword search |

## License

MIT License
