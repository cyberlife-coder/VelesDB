# E-commerce Recommendation Engine with VelesDB

> **Difficulty: Advanced** | Showcases: Vector search (HNSW), hybrid search (Vector + BM25), metadata filtering, co-purchase relationships

A comprehensive example demonstrating VelesDB's **Vector + Hybrid + Metadata filtering** capabilities for building a product recommendation system.

## What This Example Demonstrates

| Capability | Usage | Benefit |
|------------|-------|---------|
| **Vector Search** | Product embeddings for semantic similarity | "Find products similar to what I'm viewing" |
| **Hybrid Search** | Vector + BM25 tag matching via RRF fusion | Combines semantic and keyword relevance |
| **Metadata Filter** | Price, category, brand, stock, ratings (post-filter) | "Only show in-stock items under $500 with 4+ stars" |
| **Co-purchase Lookup** | `related_products` stored per product | "People who bought this also bought..." |

## Data Model

### Products (5,000 items)
```
Product struct fields (src/main.rs):

  id: u64                 (unique identifier)
  name: String            ("TechPro Premium Smartphones 42")
  category: String        ("Electronics")
  subcategory: String     ("Smartphones")
  brand: String           ("TechPro")
  price: f64              (599.99)
  rating: f32             (4.5)
  review_count: u32       (1234)
  in_stock: bool          (true)
  stock_quantity: u32     (50)
  tags: Vec<String>       (["electronics", "smartphones", "premium", "top-rated"])
  related_products: Vec<u64>  (co-purchase IDs, 2-5 per product)
```

Embeddings are generated externally via `generate_product_embedding()` (128 dimensions) and stored as the `Point` vector. The `Product` struct itself does not hold the embedding.

### Co-purchase Relationships

Each product has a `related_products` field containing 2-5 random product IDs, simulating co-purchase patterns. These are stored as metadata in the VelesDB payload and looked up directly -- no separate graph collection is used.

## Prerequisites

- **Rust 1.89+** with Cargo
- **Node.js 18+** and npm (required only for E2E tests)

## Running the Example

```bash
cd examples/ecommerce_recommendation
cargo run --release
```

### Expected Output

```
==================================================================
     VelesDB E-commerce Recommendation Engine Demo
     Vector + Graph-like + MultiColumn Combined Power
==================================================================

--- Step 1: Generating E-commerce Data ---

  Generated 5000 products
  Generated NNNNN co-purchase relationships
  Time: ...

--- Step 2: Building Vector Index (Product Embeddings) ---

  Indexed 5000 product vectors (128 dimensions)
  Stored 11 metadata fields per product
  Time: ...

--- Step 3: Recommendation Queries ---

  User is viewing: <product name> (ID: 42)
  [... detailed query results for 4 query types ...]

--- Performance Summary ---

  Products indexed:          5000
  Co-purchase relations:    NNNNN
  Vector dimensions:          128
  Metadata fields/product:     11

  Demo completed! VelesDB powers your recommendations.
```

Note: The exact banner uses Unicode box-drawing characters. Step counts, relation totals, and timings vary per run.

## Query Examples

### Query 1: Pure Vector Similarity
Find products semantically similar to the current product.

```rust
// Generate embedding for the sample product
let query_embedding = generate_product_embedding(sample_product, 128);

// Search the VelesDB collection
let results = collection.search(&query_embedding, 10)?;
```

`collection.search()` takes a `&[f32]` query vector and a `k` count, returning `Vec<SearchResult>` where each result has a `.point` (with `.payload`) and a `.score`.

### Query 2: Vector + Post-Filter
Find similar products that are in-stock and under $500. The code first runs a vector search, then filters results in Rust:

```rust
let filtered_results: Vec<_> = results
    .iter()
    .filter(|r| {
        if let Some(p) = &r.point.payload {
            let in_stock = p.get("in_stock").and_then(|v| v.as_bool()).unwrap_or(false);
            let price = p.get("price").and_then(|v| v.as_f64()).unwrap_or(f64::MAX);
            in_stock && price < 500.0
        } else {
            false
        }
    })
    .take(5)
    .collect();
```

Note: This example applies filters as a post-processing step on vector search results, not via VelesQL execution.

### Query 3: Co-purchase Lookup
Find products frequently bought together, using the `related_products` metadata:

```rust
let related_ids: &Vec<u64> = &sample_product.related_products;
// Look up each related product from the in-memory product list
for &related_id in related_ids.iter().take(5) {
    if let Some(product) = products.iter().find(|p| p.id == related_id) {
        // display product
    }
}
```

This is a direct metadata lookup, not a VelesDB graph traversal.

### Query 4: Hybrid Search (Vector + BM25)
Engine-level hybrid search combining vector similarity (60%) and BM25 tag matching (40%) via RRF fusion, then post-filtered by business rules.

```rust
// Build a text query from product tags for BM25 signal
let tag_query = sample_product.tags.join(" ");

// Hybrid search: engine-level RRF fusion of vector + BM25
// alpha=0.6 means 60% vector signal, 40% BM25 signal
let hybrid_candidates = collection
    .hybrid_search(&query_embedding, &tag_query, 20, Some(0.6))?;

// Post-filter by business rules
let final_recommendations: Vec<_> = hybrid_candidates
    .into_iter()
    .filter_map(|result| {
        products.iter().find(|p| p.id == result.point.id).and_then(|p| {
            if p.in_stock && p.rating >= 4.0 && p.price < price_threshold {
                Some((p, result.score))
            } else {
                None
            }
        })
    })
    .collect();
```

`collection.hybrid_search()` signature: `(&[f32], &str, usize, Option<f32>) -> Result<Vec<SearchResult>>`.

## Performance Characteristics

| Metric | Value |
|--------|-------|
| Products indexed | 5,000 |
| Vector dimensions | 128 |
| Co-purchase relations | ~17,000-20,000 (seeded RNG) |
| Metadata fields/product | 11 |

Latency numbers depend on hardware and are printed at runtime via `Instant::elapsed()`. They are not deterministic. On an i9-14900KF, typical values observed during development:

| Query | Typical Latency | Notes |
|-------|----------------|-------|
| Vector search | ~100-300us | HNSW search + payload retrieval |
| Post-filter | ~1-10us | In-memory filter on vector results |
| Co-purchase lookup | ~1-10us | Direct metadata access |
| Hybrid search | ~200-500us | Vector + BM25 + RRF fusion + post-filter |

### Performance Context

VelesDB's raw HNSW benchmark is **47.0us** for 10K/768D vectors (k=10). The demo latencies are higher because:

- The demo includes payload deserialization and result construction
- Hybrid search adds BM25 indexing overhead and RRF fusion
- Timings include all post-processing, not just the HNSW kernel

## Architecture

```
+----------------------------------------------------------+
|                 E-commerce Application                    |
|----------------------------------------------------------|
|                                                          |
|  +--------------+  +--------------+  +--------------+    |
|  |   Vector     |  |   Hybrid     |  |  Metadata    |    |
|  |   Index      |  |   Search     |  |  Post-filter |    |
|  |  (HNSW)      |  | (Vec + BM25) |  |  (in-memory) |    |
|  +------+-------+  +------+-------+  +------+-------+    |
|         |                 |                 |             |
|         +--------+--------+-----------------+             |
|                  |                                        |
|          +-------v-------+                                |
|          |   VelesDB     |                                |
|          |  Query Engine |                                |
|          +---------------+                                |
|                                                          |
+----------------------------------------------------------+
```

## Real-World Applications

This pattern is ideal for:

- **E-commerce**: Product recommendations, "Similar items", "Frequently bought together"
- **Media Streaming**: Content recommendations based on viewing history + genre similarity
- **Social Networks**: Friend suggestions combining profile similarity + mutual connections
- **Job Portals**: Job matching using skills embeddings + company network
- **Real Estate**: Property recommendations by features + location proximity

## Customization

### Change Vector Dimensions
```rust
// Use the Database API to create a collection with different dimensions
db.create_collection("products", 384, DistanceMetric::Cosine)?;
```

### Add Graph Relationships via VelesDB Graph Collections
For true graph traversal (not just metadata co-purchase lists), use VelesDB's `GraphCollection`:
```rust
let edge = GraphEdge::new(id, source_id, target_id, "SIMILAR_CATEGORY")?;
```

### Custom Scoring Weights
```rust
// In hybrid_search, alpha controls vector vs BM25 weight
// alpha=0.6 means 60% vector, 40% BM25
let results = collection.hybrid_search(&embedding, &text, 20, Some(0.6))?;
```

## E2E Tests (Playwright)

The example includes Playwright E2E tests validating:

- **Data generation**: 5000 products, co-purchase relationships
- **Query execution**: All 4 query types complete successfully
- **Performance**: All queries under 10ms threshold
- **Output format**: Graph query syntax, performance summary metrics

```bash
# Install dependencies
npm install

# Run all tests
npm test

# View HTML report
npm run test:report
```

## Related Documentation

- [VelesDB README](../../README.md) - Main documentation
- [VelesQL Specification](../../docs/VELESQL_SPEC.md) - SQL query syntax
- [Concurrency Model](../../docs/CONCURRENCY_MODEL.md) - Concurrency and locking model
- [Examples Overview](../README.md) - All available examples

## License

MIT License
