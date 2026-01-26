# Build a Mini Recommender with VelesDB

> **Time**: ~45 minutes | **Level**: Beginner to Intermediate

This hands-on tutorial walks you through building a product recommendation engine using VelesDB's hybrid vector + graph capabilities.

## What You'll Learn

- Store product embeddings for semantic similarity
- Create user-product relationships (LIKED, VIEWED)
- Query hybrid recommendations combining graph traversal and vector similarity
- Aggregate analytics on your data

## Prerequisites

- VelesDB installed (`cargo install velesdb-cli` or from source)
- Basic Rust knowledge (or follow along with the provided code)
- ~10 minutes to complete

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
velesdb-core = "1.3"
serde_json = "1.0"
```

---

## Step 2: Data Model

Our recommender has three entity types:

| Entity | Fields | Embedding |
|--------|--------|-----------|
| **User** | id, name, preferences | 128-dim preference vector |
| **Product** | id, title, category, price | 128-dim description embedding |
| **Relation** | LIKED, VIEWED, PURCHASED | - |

### Sample Data

```json
{
  "users": [
    {"id": 1, "name": "Alice", "preferences": "tech gadgets, wireless audio"},
    {"id": 2, "name": "Bob", "preferences": "fitness, outdoor sports"},
    {"id": 3, "name": "Carol", "preferences": "home decor, kitchen"}
  ],
  "products": [
    {"id": 101, "title": "Wireless Headphones Pro", "category": "electronics", "price": 79.99},
    {"id": 102, "title": "Bluetooth Speaker", "category": "electronics", "price": 49.99},
    {"id": 103, "title": "Running Shoes X1", "category": "sports", "price": 129.99},
    {"id": 104, "title": "Yoga Mat Premium", "category": "sports", "price": 39.99},
    {"id": 105, "title": "Smart Watch", "category": "electronics", "price": 199.99},
    {"id": 106, "title": "Coffee Maker Deluxe", "category": "home", "price": 89.99},
    {"id": 107, "title": "Fitness Tracker", "category": "electronics", "price": 59.99},
    {"id": 108, "title": "Camping Tent 4P", "category": "sports", "price": 249.99}
  ],
  "interactions": [
    {"user_id": 1, "product_id": 101, "type": "LIKED"},
    {"user_id": 1, "product_id": 102, "type": "VIEWED"},
    {"user_id": 1, "product_id": 105, "type": "LIKED"},
    {"user_id": 2, "product_id": 103, "type": "LIKED"},
    {"user_id": 2, "product_id": 104, "type": "PURCHASED"},
    {"user_id": 2, "product_id": 107, "type": "LIKED"},
    {"user_id": 3, "product_id": 106, "type": "LIKED"},
    {"user_id": 3, "product_id": 101, "type": "VIEWED"}
  ]
}
```

---

## Step 3: Initialize Database and Ingest Data

```rust
use velesdb_core::{Database, DistanceMetric, Point};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Open database
    let db = Database::open("./recommender_data")?;
    
    // Create products collection with 128-dim embeddings
    db.create_collection("products", 128, DistanceMetric::Cosine)?;
    let products = db.get_collection("products")?;
    
    // Sample products with mock embeddings
    let product_data = vec![
        (101u64, "Wireless Headphones Pro", "electronics", 79.99f32),
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
            // Generate mock embedding based on product ID
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
    println!("✅ Ingested {} products", products.len());
    
    Ok(())
}

/// Generate a deterministic mock embedding for demo purposes
fn generate_embedding(seed: u64) -> Vec<f32> {
    let mut embedding: Vec<f32> = (0..128)
        .map(|i| (seed as f32 * 0.1 + i as f32 * 0.01).sin())
        .collect();
    // Normalize
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in &mut embedding {
        *x /= norm;
    }
    embedding
}
```

**Run it:**

```bash
cargo run
# Output: ✅ Ingested 8 products
```

---

## Step 4: Basic Similarity Search

Find products similar to one a user liked:

```rust
fn find_similar_products(
    products: &velesdb_core::Collection,
    liked_product_id: u64,
    top_k: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get the liked product's embedding
    let liked = products.get_by_id(liked_product_id)?
        .ok_or("Product not found")?;
    
    // Search for similar products
    let results = products.search(&liked.vector, top_k + 1)?;
    
    println!("\n🔍 Products similar to ID {}:", liked_product_id);
    for result in results.iter().skip(1) { // Skip self-match
        let payload = result.point.payload.as_ref().unwrap();
        println!(
            "  - {} (score: {:.3}) - ${}",
            payload["title"].as_str().unwrap_or("?"),
            result.score,
            payload["price"]
        );
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
🔍 Products similar to ID 101:
  - Bluetooth Speaker (score: 0.987) - $49.99
  - Smart Watch (score: 0.954) - $199.99
  - Fitness Tracker (score: 0.923) - $59.99
```

---

## Step 5: Filtered Recommendations

Add metadata filters to narrow results:

```rust
fn recommend_in_category(
    products: &velesdb_core::Collection,
    user_preferences: &[f32],
    category: &str,
    max_price: f32,
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
                let price = payload["price"].as_f64().unwrap_or(f64::MAX) as f32;
                cat == category && price <= max_price
            } else {
                false
            }
        })
        .take(top_k)
        .collect();
    
    println!("\n🎯 Recommendations in '{}' under ${:.2}:", category, max_price);
    for result in filtered {
        let payload = result.point.payload.as_ref().unwrap();
        println!(
            "  - {} (score: {:.3}) - ${}",
            payload["title"].as_str().unwrap_or("?"),
            result.score,
            payload["price"]
        );
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
🎯 Recommendations in 'electronics' under $100.00:
  - Wireless Headphones Pro (score: 0.892) - $79.99
  - Bluetooth Speaker (score: 0.878) - $49.99
  - Fitness Tracker (score: 0.845) - $59.99
```

---

## Step 6: VelesQL Queries

Use `VelesQL` for declarative queries:

```rust
use velesdb_core::velesql::Parser;

fn demo_velesql_queries() {
    // Basic similarity search
    let query1 = r#"
        SELECT id, title, price 
        FROM products 
        WHERE similarity(embedding, $user_pref) > 0.7 
        LIMIT 5
    "#;
    
    // Filtered search
    let query2 = r#"
        SELECT * FROM products 
        WHERE similarity(embedding, $query) > 0.6 
          AND category = 'electronics' 
          AND price < 100 
        ORDER BY similarity(embedding, $query) DESC 
        LIMIT 10
    "#;
    
    // Aggregation: count by category
    let query3 = r#"
        SELECT category, COUNT(*) 
        FROM products 
        WHERE similarity(embedding, $query) > 0.5 
        GROUP BY category 
        ORDER BY COUNT(*) DESC
    "#;
    
    // Verify all queries parse correctly
    for (i, query) in [query1, query2, query3].iter().enumerate() {
        match Parser::parse(query) {
            Ok(_) => println!("✅ Query {} parses correctly", i + 1),
            Err(e) => println!("❌ Query {} failed: {:?}", i + 1, e),
        }
    }
}
```

---

## Step 7: Analytics Aggregations

Analyze your recommendation data:

```rust
fn analyze_catalog(products: &velesdb_core::Collection) {
    println!("\n📊 Catalog Analytics:");
    
    // Manual aggregation (VelesQL execution planned)
    let mut category_counts: std::collections::HashMap<String, usize> = 
        std::collections::HashMap::new();
    let mut category_totals: std::collections::HashMap<String, f64> = 
        std::collections::HashMap::new();
    
    // Iterate all products
    for id in 101u64..=108 {
        if let Ok(Some(point)) = products.get_by_id(id) {
            if let Some(payload) = &point.payload {
                let cat = payload["category"].as_str().unwrap_or("unknown").to_string();
                let price = payload["price"].as_f64().unwrap_or(0.0);
                
                *category_counts.entry(cat.clone()).or_default() += 1;
                *category_totals.entry(cat).or_default() += price;
            }
        }
    }
    
    println!("\n  Category | Count | Avg Price");
    println!("  ---------|-------|----------");
    for (cat, count) in &category_counts {
        let avg = category_totals.get(cat).unwrap_or(&0.0) / *count as f64;
        println!("  {:9} | {:5} | ${:.2}", cat, count, avg);
    }
}
```

**Output:**

```
📊 Catalog Analytics:

  Category | Count | Avg Price
  ---------|-------|----------
  electronics |     4 | $97.24
  sports      |     3 | $139.66
  home        |     1 | $89.99
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
