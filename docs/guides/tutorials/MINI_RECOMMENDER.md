# Build a Mini Recommender with VelesDB

> **Time**: ~15 minutes | **Level**: Beginner to Intermediate

This hands-on tutorial walks you through building a product recommendation engine using VelesDB's vector similarity search and metadata filtering.

## What You'll Learn

- Store product embeddings for semantic similarity
- Search for similar products with vector kNN
- Filter recommendations by category and price
- Parse VelesQL queries (SQL-like syntax with vector extensions)
- Aggregate analytics on your data

## Prerequisites

- Rust 1.83+ with Cargo
- Basic Rust knowledge (or follow along with the provided code)

---

## Step 1: Project Setup

Create a new Rust project:

```bash
cargo new mini_recommender
cd mini_recommender
```

Add dependencies to `Cargo.toml`:

```toml
[package]
name = "mini_recommender"
version = "0.1.0"
edition = "2021"

[dependencies]
velesdb-core = "1.11"
serde_json = "1.0"
tempfile = "3.10"
```

---

## Step 2: Data Model

Our recommender uses product embeddings for similarity search:

| Entity | Fields | Embedding |
|--------|--------|-----------|
| **Product** | id, title, category, price | 128-dim description embedding |

### Sample Data

```json
{
  "products": [
    {"id": 101, "title": "Wireless Headphones Pro", "category": "electronics", "price": 79.99},
    {"id": 102, "title": "Bluetooth Speaker", "category": "electronics", "price": 49.99},
    {"id": 103, "title": "Running Shoes X1", "category": "sports", "price": 129.99},
    {"id": 104, "title": "Yoga Mat Premium", "category": "sports", "price": 39.99},
    {"id": 105, "title": "Smart Watch", "category": "electronics", "price": 199.99},
    {"id": 106, "title": "Coffee Maker Deluxe", "category": "home", "price": 89.99},
    {"id": 107, "title": "Fitness Tracker", "category": "electronics", "price": 59.99},
    {"id": 108, "title": "Camping Tent 4P", "category": "sports", "price": 249.99}
  ]
}
```

---

## Step 3: Initialize Database and Ingest Data

```rust
use velesdb_core::{Database, DistanceMetric, Point};
use serde_json::json;
use tempfile::TempDir;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use temp directory for demo (use persistent path in production)
    let temp_dir = TempDir::new()?;
    let db = Database::open(temp_dir.path())?;

    // Create products collection with 128-dim embeddings
    db.create_collection("products", 128, DistanceMetric::Cosine)?;
    let products = db.get_vector_collection("products").ok_or("Collection not found")?;

    // Sample products with mock embeddings
    let product_data = [
        (101u64, "Wireless Headphones Pro", "electronics", 79.99f64),
        (102, "Bluetooth Speaker", "electronics", 49.99),
        (103, "Running Shoes X1", "sports", 129.99),
        (104, "Yoga Mat Premium", "sports", 39.99),
        (105, "Smart Watch", "electronics", 199.99),
        (106, "Coffee Maker Deluxe", "home", 89.99),
        (107, "Fitness Tracker", "electronics", 59.99),
        (108, "Camping Tent 4P", "sports", 249.99),
    ];

    let points: Vec<Point> = product_data
        .iter()
        .map(|(id, title, category, price)| {
            let embedding = generate_embedding(*id);
            Point::new(
                *id,
                embedding,
                Some(json!({
                    "title": title,
                    "category": category,
                    "price": price
                })),
            )
        })
        .collect();

    products.upsert(points)?;
    println!("Ingested {} products", products.len());

    Ok(())
}

/// Generate a deterministic mock embedding for demo purposes
fn generate_embedding(seed: u64) -> Vec<f32> {
    let mut embedding: Vec<f32> = (0..128)
        .map(|i| (seed as f32 * 0.1 + i as f32 * 0.01).sin())
        .collect();
    // Normalize
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut embedding {
            *x /= norm;
        }
    }
    embedding
}
```

**Run it:**

```bash
cargo run
# Output: Ingested 8 products
```

---

## Step 4: Basic Similarity Search

Find products similar to one a user liked:

```rust
fn find_similar_products(
    products: &velesdb_core::VectorCollection,
    liked_product_id: u64,
    top_k: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get the liked product's embedding
    let liked = products
        .get(&[liked_product_id])
        .into_iter()
        .next()
        .flatten()
        .ok_or("Product not found")?;

    // Search for similar products
    let results = products.search(&liked.vector, top_k + 1)?;

    println!("Products similar to ID {liked_product_id}:");
    for result in results.iter().skip(1) { // Skip self-match
        if let Some(payload) = &result.point.payload {
            println!(
                "  - {} (score: {:.3}) - ${:.2}",
                payload["title"].as_str().unwrap_or("?"),
                result.score,
                payload["price"].as_f64().unwrap_or(0.0)
            );
        }
    }

    Ok(())
}
```

**Usage:**

```rust
// In main():
find_similar_products(&products, 101, 3)?;
```

**Output:**

```
Products similar to ID 101:
  - Fitness Tracker (score: 0.998) - $59.99
  - Smart Watch (score: 0.995) - $199.99
  - Bluetooth Speaker (score: 0.993) - $49.99
```

Scores may differ slightly depending on your platform.

---

## Step 5: Filtered Recommendations

Add metadata filters to narrow results:

```rust
fn recommend_in_category(
    products: &velesdb_core::VectorCollection,
    user_preferences: &[f32],
    category: &str,
    max_price: f64,
    top_k: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    // Search with similarity
    let results = products.search(user_preferences, 20)?;

    // Filter by category and price (post-filter for demo)
    let filtered: Vec<_> = results
        .iter()
        .filter(|r| {
            if let Some(payload) = &r.point.payload {
                let cat = payload["category"].as_str().unwrap_or("");
                let price = payload["price"].as_f64().unwrap_or(f64::MAX);
                cat == category && price <= max_price
            } else {
                false
            }
        })
        .take(top_k)
        .collect();

    println!("\nRecommendations in '{category}' under ${max_price:.2}:");
    for result in filtered {
        if let Some(payload) = &result.point.payload {
            println!(
                "  - {} (score: {:.3}) - ${:.2}",
                payload["title"].as_str().unwrap_or("?"),
                result.score,
                payload["price"].as_f64().unwrap_or(0.0)
            );
        }
    }

    Ok(())
}
```

**Usage:**

```rust
// User preferences embedding (mock: similar to electronics)
let alice_prefs = generate_embedding(100);
recommend_in_category(&products, &alice_prefs, "electronics", 100.0, 3)?;
```

**Output:**

```
Recommendations in 'electronics' under $100.00:
  - Wireless Headphones Pro (score: 0.969) - $79.99
  - Fitness Tracker (score: 0.967) - $59.99
  - Bluetooth Speaker (score: 0.962) - $49.99
```

Scores may differ slightly depending on your platform.

---

## Step 6: VelesQL Queries

Use `VelesQL` for declarative queries:

```rust
use velesdb_core::velesql::Parser;

fn demo_velesql_queries() {
    println!("\nVelesQL Query Parsing:");

    let queries = [
        (
            "Similarity search",
            "SELECT id, title FROM products WHERE similarity(embedding, $pref) > 0.7 LIMIT 5",
        ),
        (
            "Filtered search",
            "SELECT * FROM products WHERE similarity(embedding, $q) > 0.6 \
             AND category = 'electronics' AND price < 100 \
             ORDER BY similarity(embedding, $q) DESC LIMIT 10",
        ),
        (
            "Aggregation",
            "SELECT category, COUNT(*) FROM products \
             WHERE similarity(embedding, $q) > 0.5 \
             GROUP BY category ORDER BY COUNT(*) DESC",
        ),
    ];

    for (name, query) in queries {
        match Parser::parse(query) {
            Ok(_) => println!("  {name}: parses correctly"),
            Err(e) => println!("  {name}: {e:?}"),
        }
    }
}
```

---

## Step 7: Analytics Aggregations

Analyze your recommendation data:

```rust
use std::collections::HashMap;

fn analyze_catalog(products: &velesdb_core::VectorCollection) {
    println!("\nCatalog Analytics:");

    let mut category_counts: HashMap<String, usize> = HashMap::new();
    let mut category_totals: HashMap<String, f64> = HashMap::new();

    for id in 101u64..=108 {
        if let Some(point) = products.get(&[id]).into_iter().next().flatten() {
            if let Some(payload) = &point.payload {
                let cat = payload["category"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let price = payload["price"].as_f64().unwrap_or(0.0);

                *category_counts.entry(cat.clone()).or_default() += 1;
                *category_totals.entry(cat).or_default() += price;
            }
        }
    }

    println!("\n  Category    | Count | Avg Price");
    println!("  ------------|-------|----------");

    let mut cats: Vec<_> = category_counts.keys().collect();
    cats.sort();

    for cat in cats {
        let count = category_counts.get(cat).unwrap_or(&0);
        let avg = category_totals.get(cat).unwrap_or(&0.0) / *count as f64;
        println!("  {cat:11} | {count:5} | ${avg:.2}");
    }
}
```

**Output:**

```
Catalog Analytics:

  Category    | Count | Avg Price
  ------------|-------|----------
  electronics |     4 | $97.49
  home        |     1 | $89.99
  sports      |     3 | $139.99
```

---

## Complete Example

The full working code is available at:

```
examples/mini_recommender/main.rs
```

Run it:

```bash
cd examples/mini_recommender
cargo run
```

---

## Next Steps

1. **Real Embeddings**: Replace mock embeddings with actual text embeddings from:
   - OpenAI `text-embedding-3-small`
   - Sentence Transformers
   - Cohere Embed

2. **Graph Relations**: Use the Graph API for user-product relationships:
   ```rust
   db.graph().add_edge(user_id, product_id, "LIKED", None)?;
   ```

3. **Hybrid Search**: Combine vector similarity with graph traversal for better recommendations

4. **Production**: See [USE_CASES.md](../USE_CASES.md) for 10 production-ready patterns

---

## Summary

| Capability | VelesDB Feature |
|------------|-----------------|
| Semantic Search | `similarity()`, `NEAR` |
| Filtering | `WHERE category = 'x'` |
| Sorting | `ORDER BY similarity() DESC` |
| Aggregations | `GROUP BY`, `COUNT(*)` |
| Graph Relations | Graph API |

**VelesDB** makes it easy to build recommendation systems that combine the best of vector search and graph databases in a single, embedded solution.

---

*Questions? See the [VelesDB Documentation](https://github.com/cyberlife-coder/VelesDB) or join our community.*
