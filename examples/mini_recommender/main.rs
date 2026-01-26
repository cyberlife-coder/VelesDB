//! Mini Recommender Example
//!
//! Demonstrates building a product recommendation engine with `VelesDB`.
//! See the full tutorial: docs/guides/TUTORIALS/MINI_RECOMMENDER.md

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::{velesql::Parser, Database, DistanceMetric, Point};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 VelesDB Mini Recommender Example\n");

    // Use temp directory for demo (use persistent path in production)
    let temp_dir = TempDir::new()?;
    let db = Database::open(temp_dir.path())?;

    // Step 1: Create products collection
    db.create_collection("products", 128, DistanceMetric::Cosine)?;
    let products = db.get_collection("products").ok_or("Collection not found")?;

    // Step 2: Ingest products
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
    println!("✅ Ingested {} products\n", products.len());

    // Step 3: Basic similarity search
    find_similar_products(&products, 101, 3)?;

    // Step 4: Filtered recommendations
    let alice_prefs = generate_embedding(100);
    recommend_in_category(&products, &alice_prefs, "electronics", 100.0, 3)?;

    // Step 5: VelesQL query parsing demo
    demo_velesql_queries();

    // Step 6: Analytics
    analyze_catalog(&products);

    println!("\n✨ Tutorial complete! See docs/guides/TUTORIALS/MINI_RECOMMENDER.md");

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

/// Find products similar to a given product
fn find_similar_products(
    products: &velesdb_core::Collection,
    liked_product_id: u64,
    top_k: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let liked = products
        .get(&[liked_product_id])
        .into_iter()
        .next()
        .flatten()
        .ok_or("Product not found")?;

    let results = products.search(&liked.vector, top_k + 1)?;

    println!("🔍 Products similar to ID {}:", liked_product_id);
    for result in results.iter().skip(1) {
        if let Some(payload) = &result.point.payload {
            println!(
                "  - {} (score: {:.3}) - ${}",
                payload["title"].as_str().unwrap_or("?"),
                result.score,
                payload["price"]
            );
        }
    }

    Ok(())
}

/// Recommend products in a specific category under a price limit
fn recommend_in_category(
    products: &velesdb_core::Collection,
    user_preferences: &[f32],
    category: &str,
    max_price: f32,
    top_k: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let results = products.search(user_preferences, 20)?;

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

    println!(
        "\n🎯 Recommendations in '{}' under ${:.2}:",
        category, max_price
    );
    for result in filtered {
        if let Some(payload) = &result.point.payload {
            println!(
                "  - {} (score: {:.3}) - ${}",
                payload["title"].as_str().unwrap_or("?"),
                result.score,
                payload["price"]
            );
        }
    }

    Ok(())
}

/// Demonstrate VelesQL query parsing
fn demo_velesql_queries() {
    println!("\n📝 VelesQL Query Parsing:");

    let queries = [
        (
            "Similarity search",
            "SELECT id, title FROM products WHERE similarity(embedding, $pref) > 0.7 LIMIT 5",
        ),
        (
            "Filtered search",
            "SELECT * FROM products WHERE similarity(embedding, $q) > 0.6 AND category = 'electronics' AND price < 100 ORDER BY similarity(embedding, $q) DESC LIMIT 10",
        ),
        (
            "Aggregation",
            "SELECT category, COUNT(*) FROM products WHERE similarity(embedding, $q) > 0.5 GROUP BY category ORDER BY COUNT(*) DESC",
        ),
    ];

    for (name, query) in queries {
        match Parser::parse(query) {
            Ok(_) => println!("  ✅ {}: parses correctly", name),
            Err(e) => println!("  ❌ {}: {:?}", name, e),
        }
    }
}

/// Analyze the product catalog
fn analyze_catalog(products: &velesdb_core::Collection) {
    println!("\n📊 Catalog Analytics:");

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
        println!("  {:11} | {:5} | ${:.2}", cat, count, avg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_embedding_normalized() {
        let emb = generate_embedding(42);
        assert_eq!(emb.len(), 128);
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001, "Embedding should be normalized");
    }

    #[test]
    fn test_velesql_queries_parse() {
        let queries = [
            "SELECT * FROM products WHERE similarity(embedding, $q) > 0.7 LIMIT 5",
            "SELECT category, COUNT(*) FROM products GROUP BY category",
        ];

        for query in queries {
            assert!(Parser::parse(query).is_ok(), "Query should parse: {}", query);
        }
    }
}
