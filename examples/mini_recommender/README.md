# VelesDB Mini Recommender

> **Difficulty: Beginner** | Showcases: Collection creation, vector upsert, similarity search, filtered queries, VelesQL parsing, catalog analytics

A self-contained product recommendation engine that demonstrates the core VelesDB workflow end-to-end in under 250 lines of Rust.

## What It Does

1. Creates a `products` collection (128-dim, cosine)
2. Ingests 8 products with embeddings and metadata (title, category, price)
3. Finds products similar to a given product (vector similarity)
4. Recommends products filtered by category and price ceiling
5. Parses several VelesQL queries to show the SQL-like syntax
6. Prints catalog analytics grouped by category

## Prerequisites

- Rust 1.89+ with Cargo

## How to Run

```bash
cd examples/mini_recommender
cargo run
```

## Expected Output

```
🚀 VelesDB Mini Recommender Example

✅ Ingested 8 products

🔍 Products similar to ID 101:
  - Bluetooth Speaker (score: 0.999) - $49.99
  - Running Shoes X1 (score: 0.997) - $129.99
  - Yoga Mat Premium (score: 0.993) - $39.99

🎯 Recommendations in 'electronics' under $100.00:
  - Wireless Headphones Pro (score: 0.999) - $79.99
  - Bluetooth Speaker (score: 0.997) - $49.99
  - Fitness Tracker (score: 0.962) - $59.99

📝 VelesQL Query Parsing:
  ✅ Similarity search: parses correctly
  ✅ Filtered search: parses correctly
  ✅ Aggregation: parses correctly

📊 Catalog Analytics:

  Category    | Count | Avg Price
  ------------|-------|----------
  electronics |     4 | $97.49
  home        |     1 | $89.99
  sports      |     3 | $139.99

✨ Tutorial complete! See docs/guides/TUTORIALS/MINI_RECOMMENDER.md
```

Scores may differ slightly depending on your platform.

## VelesDB Features Demonstrated

| Feature | Where |
|---------|-------|
| `Database::open()` | Opens a temporary database |
| `create_collection()` | 128-dim cosine collection |
| `upsert()` | Batch insert with JSON payloads |
| `search()` | K-nearest-neighbor vector search |
| Post-search filtering | Category + price filter in Rust |
| `Parser::parse()` | VelesQL query parsing |
| Catalog analytics | Iterate and aggregate point payloads |

## License

MIT License
