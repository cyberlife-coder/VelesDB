# 🛒 E-commerce Recommendation Engine with VelesDB

> **Difficulty: Advanced** | Showcases: Vector search (HNSW), knowledge graph traversal, multi-column filtering, combined queries, VelesQL

A comprehensive example demonstrating VelesDB's **Vector + Graph + MultiColumn** combined capabilities for building a production-grade recommendation system.

## 🎯 What This Example Demonstrates

| Capability | Usage | Benefit |
|------------|-------|---------|
| **Vector Search** | Product embeddings for semantic similarity | "Find products similar to what I'm viewing" |
| **Knowledge Graph** | User behavior relationships (bought_together, viewed_also) | "People who bought this also bought..." |
| **Multi-Column Filter** | Price, category, brand, stock, ratings | "Only show in-stock items under $500 with 4+ stars" |
| **Combined Queries** | All three unified in microseconds | Production-ready recommendations |

## 📊 Data Model

### Products (5,000 items)
```
┌─────────────────────────────────────────────────────────────┐
│ Product                                                      │
├─────────────────────────────────────────────────────────────┤
│ id: u64                 (unique identifier)                  │
│ name: String            ("TechPro Premium Smartphones 42")   │
│ category: String        ("Electronics")                      │
│ subcategory: String     ("Smartphones")                      │
│ brand: String           ("TechPro")                          │
│ price: f64              (599.99)                             │
│ rating: f32             (4.5)                                │
│ review_count: u32       (1234)                               │
│ in_stock: bool          (true)                               │
│ stock_quantity: u32     (50)                                 │
│ embedding: [f32; 128]   (semantic vector)                    │
└─────────────────────────────────────────────────────────────┘
```

### Knowledge Graph (Relationships)
```
    ┌──────────┐                         ┌──────────┐
    │ Product  │───BOUGHT_TOGETHER──────▶│ Product  │
    │   (A)    │◀──────────────────────  │   (B)    │
    └──────────┘                         └──────────┘
         │                                    │
         │ VIEWED_ALSO                        │
         ▼                                    ▼
    ┌──────────┐                         ┌──────────┐
    │ Product  │                         │ Product  │
    │   (C)    │                         │   (D)    │
    └──────────┘                         └──────────┘
```

### User Behaviors (10,000+ events)
- **Viewed**: User viewed a product page
- **AddedToCart**: User added to shopping cart
- **Purchased**: User completed purchase

## Prerequisites

- **Rust 1.83+** with Cargo
- **Node.js 18+** and npm (required only for E2E tests)

## 🚀 Running the Example

```bash
cd examples/ecommerce_recommendation
cargo run --release
```

### Expected Output

```
╔══════════════════════════════════════════════════════════════════╗
║     VelesDB E-commerce Recommendation Engine Demo                ║
║     Vector + Graph + MultiColumn Combined Power                  ║
╚══════════════════════════════════════════════════════════════════╝

━━━ Step 1: Generating E-commerce Data ━━━
✓ Generated 5000 products
✓ Generated 15000+ user behaviors from 1000 users

━━━ Step 2: Building Vector Index (Product Embeddings) ━━━
✓ Indexed 5000 product vectors (128 dimensions)

━━━ Step 3: Building Knowledge Graph (User Behavior) ━━━
✓ Created 5000 product nodes
✓ Created 50000+ relationship edges

━━━ Step 4: Recommendation Queries ━━━
[... detailed query results ...]
```

## 🔍 Query Examples

### Query 1: Pure Vector Similarity
Find products semantically similar to the current product.

```rust
let results = collection.search(&query_embedding, 10)?;
```

### Query 2: Vector + Filter (VelesQL)
Find similar products that are in-stock and under $500.

```sql
SELECT * FROM products 
WHERE similarity(embedding, ?) > 0.7
  AND in_stock = true 
  AND price < 500
ORDER BY similarity DESC
LIMIT 10
```

### Query 3: Graph Traversal
Find products frequently bought together.

```cypher
MATCH (p:Product)-[:BOUGHT_TOGETHER]-(other:Product)
WHERE p.id = 42
RETURN other
LIMIT 10
```

### Query 4: Combined (Full Power)
Union of vector similarity + graph neighbors, filtered by business rules.

```rust
// Combine vector scores (60%) + graph proximity (40%)
for result in vector_results {
    combined_scores[result.id] += result.score * 0.6;
}
for neighbor in graph_neighbors {
    combined_scores[neighbor] += 0.4;
}

// Apply business rules filter
let recommendations = combined_scores
    .filter(|p| p.in_stock && p.rating >= 4.0 && p.price < threshold)
    .sort_by_score()
    .take(10);
```

## 📈 Performance Characteristics (Actual Results)

| Metric | Value |
|--------|-------|
| Products indexed | 5,000 |
| Vector dimensions | 128 |
| Co-purchase relations | ~20,000 |
| Metadata fields/product | 11 |
| **Vector search latency** | **187µs** |
| **Filtered search latency** | **55µs** |
| **Graph lookup latency** | **88µs** |
| **Combined query latency** | **202µs** |

### Performance Analysis

These results are **production-ready** and compare favorably to VelesDB's benchmarks:

| Comparison | Benchmark | E-commerce Demo | Analysis |
|------------|-----------|-----------------|----------|
| HNSW Search (10K, 768D) | 42.8µs | 187µs (5K, 128D) | ✅ Includes I/O + payload retrieval |
| Filter overhead | — | +55µs | ✅ Minimal (metadata in memory) |
| Graph lookup | — | 88µs | ✅ O(1) relationship access |

**Why slightly higher than raw benchmark?**
- Benchmark measures pure HNSW distance computation
- Demo includes: payload deserialization, result construction, I/O
- Real-world overhead is expected and acceptable

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    E-commerce Application                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
│  │   Vector     │  │  Knowledge   │  │  Multi-Col   │          │
│  │   Index      │  │    Graph     │  │   Filters    │          │
│  │  (HNSW)      │  │  (Adjacency) │  │  (B-Tree)    │          │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘          │
│         │                 │                 │                   │
│         └────────────┬────┴────────────────┘                   │
│                      │                                          │
│              ┌───────▼───────┐                                  │
│              │   VelesDB     │                                  │
│              │  Query Engine │                                  │
│              └───────────────┘                                  │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## 💡 Real-World Applications

This pattern is ideal for:

- **E-commerce**: Product recommendations, "Similar items", "Frequently bought together"
- **Media Streaming**: Content recommendations based on viewing history + genre similarity
- **Social Networks**: Friend suggestions combining profile similarity + mutual connections
- **Job Portals**: Job matching using skills embeddings + company network
- **Real Estate**: Property recommendations by features + location proximity

## 🔧 Customization

### Change Vector Dimensions
```rust
let config = CollectionConfig {
    dimension: 384,  // Use larger embeddings for better accuracy
    ..Default::default()
};
```

### Add More Relationship Types
```rust
let edge = Edge::new(
    id,
    source_id,
    target_id,
    RelationshipType::Custom("SIMILAR_CATEGORY".to_string()),
);
```

### Custom Scoring Weights
```rust
// Adjust weights based on your use case
const VECTOR_WEIGHT: f32 = 0.5;
const GRAPH_WEIGHT: f32 = 0.3;
const POPULARITY_WEIGHT: f32 = 0.2;
```

## 🧪 E2E Tests (Playwright)

The example includes comprehensive Playwright E2E tests validating:

- **Data generation**: 5000 products, ~20000 relationships
- **Query execution**: All 4 query types complete successfully  
- **Performance**: All queries under 10ms threshold
- **Output format**: VelesQL syntax, graph queries, metrics

```bash
# Install dependencies
npm install

# Run all tests
npm test

# View HTML report
npm run test:report
```

### Test Results

```
Running 15 tests using 1 worker
  ✓  should generate 5000 products
  ✓  should generate co-purchase relationships
  ✓  should execute Vector Similarity query (Query 1)
  ✓  should execute Vector + Filter query (Query 2)
  ✓  should execute Graph Lookup query (Query 3)
  ✓  should execute Combined query (Query 4)
  ✓  should complete demo successfully
  ✓  vector search should be under 10ms
  ✓  filtered search should be under 10ms
  ✓  graph lookup should be under 1ms
  ✓  combined query should be under 10ms
  ...
  15 passed (2.9s)
```

## 📚 Related Documentation

- [VelesDB README](../../README.md) - Main documentation
- [VelesQL Specification](../../docs/VELESQL_SPEC.md) - SQL query syntax
- [Concurrency Model](../../docs/CONCURRENCY_MODEL.md) - Concurrency and locking model
- [Examples Overview](../README.md) - All available examples

## 📄 License

MIT License
