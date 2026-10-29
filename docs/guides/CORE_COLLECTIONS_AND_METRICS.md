# Core: collections, distance metrics, storage modes

Reference for the `velesdb-core` collection model — what a collection is, how
the distance metric is chosen, what a payload may contain, how to trade memory
for recall with quantized storage modes, and how bulk ingestion and durability
behave.

Moved out of `crates/velesdb-core/README.md` to keep that file under the
400-line documentation budget.

> The snippets below are function bodies: they use `?`, so paste them inside
> `fn main() -> Result<(), Box<dyn std::error::Error>> { ... Ok(()) }`.

---

## The collection model

VelesDB is **not** a relational database. Each vector collection has:

- **one vector column** with a fixed dimension,
- **one distance metric**, immutable after creation,
- a **JSON payload** (`serde_json::Value`) per point, optional.

```rust
use velesdb_core::{Database, DistanceMetric};

let db = Database::open("./data")?;

// Cosine metric, for text embeddings
db.create_collection("documents", 768, DistanceMetric::Cosine)?;

// Hamming metric, for binary fingerprints
db.create_collection("fingerprints", 256, DistanceMetric::Hamming)?;

// The metric is fixed: to use a different one, create another collection.
```

`Database::get_vector_collection(name)` returns `Option<VectorCollection>` — it
yields `None` when the name refers to a graph or metadata-only collection.
`Database::get_any_collection(name)` returns the type-erased `AnyCollection`
and checks the vector → graph → metadata registries in that order.

### Working through `AnyCollection`

`AnyCollection` is an enum over the three typed handles. The operations every
kind supports dispatch without a `match` — `name`, `config`, `point_count`,
`is_empty`, `flush`, `get`, `upsert`, the graph-edge calls (`add_edge`,
`remove_edge`, `get_outgoing_edges`, `edge_exists`, `max_edge_id`),
`execute_query_str`, `execute_aggregate` and `diagnostics`; the typed handle is
one call away when you need a kind-specific method.

```rust
use std::collections::HashMap;
use velesdb_core::velesql::Parser;
use velesdb_core::Database;

let db = Database::open("./data")?;
let any = db.get_any_collection("documents").ok_or("collection not found")?;

println!("{} holds {} points", any.name(), any.point_count());
let diag = any.diagnostics();
println!("search ready: {}, index: {:?}", diag.search_ready, diag.index_health);

let params = HashMap::new();
let rows = any.execute_query_str("SELECT * FROM documents LIMIT 5", &params)?;
let query = Parser::parse("SELECT category, COUNT(*) FROM documents GROUP BY category")?;
let counts = any.execute_aggregate(&query, &params)?; // serde_json::Value

// Check, borrow, or take the typed handle.
assert!(any.is_vector() && !any.is_graph() && !any.is_metadata());
if let Some(vectors) = any.as_vector() {
    let hits = vectors.search(&[0.1_f32; 768], 10)?;
}
let vectors = any
    .into_vector()
    .map_err(|other| format!("{} is not a vector collection", other.name()))?;
```

`as_vector` / `as_graph` / `as_metadata` borrow (`Option<&T>`, with `_mut`
forms), and `into_vector` / `into_graph` / `into_metadata` consume, handing the
`AnyCollection` back in `Err` on a mismatch. The enum is `#[non_exhaustive]`,
so a `match` on it needs a wildcard arm. `execute_query_str` and
`execute_aggregate` run on the collection itself: a database observer's read
gate is consulted by `Database::execute_query`, not here (see
[Core: operations API](./CORE_OPERATIONS_API.md#control-plane-observer)).

## Distance metrics

All five metrics are variants of `DistanceMetric`:

```rust
use velesdb_core::DistanceMetric;

let cosine = DistanceMetric::Cosine;        // text embeddings (normalized)
let euclidean = DistanceMetric::Euclidean;  // image features, spatial data
let dot = DistanceMetric::DotProduct;       // pre-normalized vectors, MIPS
let hamming = DistanceMetric::Hamming;      // binary vectors, fingerprints, LSH
let jaccard = DistanceMetric::Jaccard;      // set similarity, sparse tags
```

| Metric | Use case | Score interpretation |
|--------|----------|---------------------|
| `Cosine` | Text embeddings | Higher = more similar |
| `Euclidean` | Spatial data | Lower = more similar |
| `DotProduct` | MIPS, pre-normalized | Higher = more similar |
| `Hamming` | Binary vectors | Lower = more similar |
| `Jaccard` | Set similarity | Higher = more similar |

### Common embedding dimensions

The `dimension` argument must match your embedding model's output size exactly.

| Model | Dimension | Metric |
|-------|-----------|--------|
| OpenAI `text-embedding-3-small` | 1536 | Cosine |
| OpenAI `text-embedding-3-large` | 3072 | Cosine |
| Sentence-Transformers `all-MiniLM-L6-v2` | 384 | Cosine |
| Cohere `embed-english-v3.0` | 1024 | Cosine |
| BAAI `bge-large-en-v1.5` | 1024 | Cosine |
| CLIP (image + text) | 512 or 768 | Cosine |

## Payload (metadata) format

Metadata is stored as JSON. Any valid JSON structure is accepted.

```rust
use serde_json::json;
use velesdb_core::Point;

let vector = vec![0.1_f32; 768];

// Flat payload
let point1 = Point::new(1, vector.clone(), Some(json!({
    "title": "Hello World",
    "category": "greeting",
    "views": 1500,
    "published": true
})));

// Nested payload
let point2 = Point::new(2, vector.clone(), Some(json!({
    "title": "Rust Guide",
    "author": { "name": "Alice", "email": "alice@example.com" },
    "tags": ["rust", "programming", "tutorial"],
    "stats": { "views": 5000, "likes": 120 }
})));

// No payload
let point3 = Point::without_payload(3, vector);
```

## Storage modes (quantization at creation time)

`create_collection_with_options` selects the on-disk representation. The
metric and the storage mode must agree (binary storage pairs with `Hamming`).

```rust
use velesdb_core::{Database, DistanceMetric, StorageMode};

let db = Database::open("./data")?;

// SQ8: 4x memory reduction, small recall loss
db.create_collection_with_options(
    "sq8_collection", 768, DistanceMetric::Cosine, StorageMode::SQ8)?;

// Binary: 32x memory reduction, larger recall loss (IoT / edge)
db.create_collection_with_options(
    "binary_collection", 768, DistanceMetric::Hamming, StorageMode::Binary)?;

// Product Quantization: variable compression
db.create_collection_with_options(
    "pq_collection", 768, DistanceMetric::Cosine, StorageMode::ProductQuantization)?;

// RaBitQ: randomized binary quantization
db.create_collection_with_options(
    "rabitq_collection", 768, DistanceMetric::Cosine, StorageMode::RaBitQ)?;
```

Compression ratios, recall trade-offs, training (`TRAIN QUANTIZER`), OPQ and
the SIMD distance kernels are documented in
[Quantization](./QUANTIZATION.md).

## Bulk ingestion

For high-throughput import (what is measured, and at which dimension, is in
[Core performance](./CORE_PERFORMANCE.md)):

```rust
use velesdb_core::{Database, DistanceMetric, Point};

let db = Database::open("./data")?;
db.create_collection("bulk_test", 768, DistanceMetric::Cosine)?;
let collection = db
    .get_vector_collection("bulk_test")
    .ok_or("collection not found")?;

let points: Vec<Point> = (0..10_000)
    .map(|i| Point::without_payload(i, vec![0.1; 768]))
    .collect();

// Bulk insert with parallel HNSW indexing
let inserted = collection.upsert_bulk(&points)?;
println!("Inserted {inserted} vectors");

// Explicit durability barrier
collection.flush()?;
```

`upsert_bulk` is not just a loop over `upsert`: it runs turbo/fast batch modes,
parallel HNSW indexing, graduated `ef_construction` (VAMANA 3-phase), and
lock-free entry-point reads (a small lock serializes its rare promotions).

## Durability semantics

- `store` / `upsert` update in-memory and WAL state, optimized for throughput.
- `flush()` is the explicit durability barrier for crash-consistent
  persistence. Call it when you need the data to survive a crash.
- Destructor-based cleanup is best-effort and must **not** be treated as a
  commit boundary.

## See also

- [velesdb-core README](../../crates/velesdb-core/README.md)
- [Core VelesQL reference](./CORE_VELESQL_REFERENCE.md)
- [Core performance](./CORE_PERFORMANCE.md)
- [Tuning guide](./TUNING_GUIDE.md) — HNSW parameters
- [Search modes](./SEARCH_MODES.md) — Fast / Balanced / Accurate / Perfect / Adaptive
- [Core: operations API](./CORE_OPERATIONS_API.md) — index introspection, guard-rails, observer, metrics

---

Last updated: 2026-10-29 · Applies to: velesdb-core 7.1.0
