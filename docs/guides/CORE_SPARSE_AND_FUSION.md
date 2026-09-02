# Core: sparse vectors and result fusion

`velesdb-core` stores named sparse vectors (SPLADE, BM25 term weights, tag
sets) next to dense embeddings on the same point. You can search them
independently, or fuse dense and sparse rankings with Reciprocal Rank Fusion
(RRF) or Relative Score Fusion (RSF).

Moved out of `crates/velesdb-core/README.md` to keep that file under the
400-line documentation budget.

> The snippets below are function bodies: they use `?`, so paste them inside
> `fn main() -> Result<(), Box<dyn std::error::Error>> { ... Ok(()) }`. The
> search snippets continue the `collection` binding created in the first one.

---

## Upserting points with sparse vectors

```rust
use std::collections::BTreeMap;
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::{Database, DistanceMetric, Point};

let db = Database::open("./data")?;
db.create_collection("docs", 768, DistanceMetric::Cosine)?;
let collection = db
    .get_vector_collection("docs")
    .ok_or("collection not found")?;

// A sparse vector is a list of (term_index, weight) pairs.
let sparse = SparseVector::new(vec![
    (42, 1.2),   // term 42, weight 1.2
    (187, 0.8),  // term 187, weight 0.8
    (1024, 0.3),
]);

// Sparse vectors are named; "" is the default index.
let mut sparse_map = BTreeMap::new();
sparse_map.insert(String::new(), sparse);

let point = Point::with_sparse(
    1,
    vec![0.1; 768],                                    // dense embedding
    Some(serde_json::json!({ "title": "My doc" })),    // payload
    Some(sparse_map),                                  // named sparse vectors
);
collection.upsert(vec![point])?;
```

## Sparse-only search (DAAT MaxScore)

The sparse engine uses a DAAT (Document-At-A-Time) MaxScore algorithm for fast
top-k retrieval by inner product, and falls back to a linear scan for
high-coverage queries.

```rust
use velesdb_core::sparse_index::SparseVector;
let query = SparseVector::new(vec![(42, 1.0), (187, 0.5)]);

// Top-5 on the default sparse index
let results = collection.sparse_search(&query, 5, "")?;
for result in &results {
    println!("ID: {}, Score: {:.4}", result.point.id, result.score);
}
```

## Hybrid dense + sparse with RRF fusion

Both branches run in parallel (rayon), then the two rankings are fused.

```rust
use velesdb_core::sparse_index::SparseVector;
use velesdb_core::FusionStrategy;
let dense_query = vec![0.15; 768];
let sparse_query = SparseVector::new(vec![(42, 1.0), (187, 0.5)]);

// RRF with the default k = 60
let strategy = FusionStrategy::rrf_default();
let results = collection.hybrid_sparse_search(
    &dense_query,
    &sparse_query,
    10,          // top-k
    "",          // default sparse index
    &strategy,
)?;

for result in &results {
    println!("ID: {}, Fused score: {:.4}", result.point.id, result.score);
}
```

Use `RelativeScore` when you want explicit weights instead of rank-based
fusion:

```rust
use velesdb_core::FusionStrategy;
// 70% dense, 30% sparse — the constructor validates the weights
let strategy = FusionStrategy::relative_score(0.7, 0.3)?;
```

## Types and methods

| Type | Path | Description |
|------|------|-------------|
| `SparseVector` | `velesdb_core::sparse_index` | Sorted `(u32 index, f32 weight)` pairs; deduplicates and drops zeros on construction |
| `FusionStrategy` | `velesdb_core` | `RRF { k }`, `RelativeScore { dense_weight, sparse_weight }` |
| `ScoredDoc` | `velesdb_core::sparse_index` | Raw sparse result: `doc_id: u64`, `score: f32` |

| Method | On | Description |
|--------|----|-------------|
| `sparse_search(query, k, index_name)` | `VectorCollection` | Sparse search on the named index (`""` = default) |
| `hybrid_sparse_search(dense, sparse, k, index_name, strategy)` | `VectorCollection` | Dense + sparse with fusion |

Full signatures live on [docs.rs](https://docs.rs/velesdb-core).

## See also

- [velesdb-core README](../../crates/velesdb-core/README.md)
- [Core VelesQL reference](./CORE_VELESQL_REFERENCE.md)
- [Core performance](./CORE_PERFORMANCE.md) — sparse search micro-benchmark

---

Last updated: 2026-07-25 · Applies to: velesdb-core 6.0.0
