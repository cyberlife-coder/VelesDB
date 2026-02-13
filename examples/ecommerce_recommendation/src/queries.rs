//! Query demonstrations for the e-commerce recommendation demo.

use crate::data_gen::{generate_product_embedding, Product};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use velesdb_core::collection::Collection;
use velesdb_core::SearchResult;

/// QUERY 1: Pure Vector Similarity (Semantic Search)
pub fn query_vector_similarity(
    collection: &Collection,
    sample_product: &Product,
) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
    println!("┌─────────────────────────────────────────────────────────────────┐");
    println!("│ QUERY 1: Vector Similarity - \"Products similar to current\"     │");
    println!("└─────────────────────────────────────────────────────────────────┘");

    let query_embedding = generate_product_embedding(sample_product, 128);

    let start = Instant::now();
    let results = collection.search(&query_embedding, 10)?;
    let search_latency = start.elapsed();

    println!(
        "  Found {} similar products in {:?}\n",
        results.len(),
        search_latency
    );
    for (i, result) in results.iter().take(5).enumerate() {
        if let Some(payload) = &result.point.payload {
            println!(
                "  {}. {} (score: {:.4})",
                i + 1,
                payload
                    .get("name")
                    .and_then(|v: &JsonValue| v.as_str())
                    .unwrap_or("?"),
                result.score
            );
            println!(
                "     ${:.2} | {} | {}/5 ⭐",
                payload
                    .get("price")
                    .and_then(|v: &JsonValue| v.as_f64())
                    .unwrap_or(0.0),
                payload
                    .get("brand")
                    .and_then(|v: &JsonValue| v.as_str())
                    .unwrap_or("?"),
                payload
                    .get("rating")
                    .and_then(|v: &JsonValue| v.as_f64())
                    .unwrap_or(0.0)
            );
        }
    }

    Ok(results)
}

/// QUERY 2: Vector + Filter (Business Rules)
pub fn query_filtered_vector(results: &[SearchResult]) {
    println!("\n┌─────────────────────────────────────────────────────────────────┐");
    println!("│ QUERY 2: Vector + Filter - \"Similar, in-stock, under $500\"     │");
    println!("└─────────────────────────────────────────────────────────────────┘");

    let start = Instant::now();

    // Reason: Display-only — shows VelesQL equivalent of the programmatic filter below
    let query = r#"SELECT * FROM products 
           WHERE similarity(embedding, ?) > 0.7
             AND in_stock = true 
             AND price < 500
           ORDER BY similarity DESC
           LIMIT 10"#;

    let filtered_results: Vec<_> = results
        .iter()
        .filter(|r| {
            if let Some(p) = &r.point.payload {
                let in_stock = p
                    .get("in_stock")
                    .and_then(|v: &JsonValue| v.as_bool())
                    .unwrap_or(false);
                let price = p
                    .get("price")
                    .and_then(|v: &JsonValue| v.as_f64())
                    .unwrap_or(f64::MAX);
                in_stock && price < 500.0
            } else {
                false
            }
        })
        .take(5)
        .collect();

    println!(
        "  VelesQL: {}",
        query.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    println!(
        "  Found {} filtered results in {:?}\n",
        filtered_results.len(),
        start.elapsed()
    );

    for (i, result) in filtered_results.iter().enumerate() {
        if let Some(payload) = &result.point.payload {
            println!(
                "  {}. {} ✓ In Stock",
                i + 1,
                payload
                    .get("name")
                    .and_then(|v: &JsonValue| v.as_str())
                    .unwrap_or("?")
            );
            println!(
                "     ${:.2} | {} | {}/5 ⭐",
                payload
                    .get("price")
                    .and_then(|v: &JsonValue| v.as_f64())
                    .unwrap_or(0.0),
                payload
                    .get("brand")
                    .and_then(|v: &JsonValue| v.as_str())
                    .unwrap_or("?"),
                payload
                    .get("rating")
                    .and_then(|v: &JsonValue| v.as_f64())
                    .unwrap_or(0.0)
            );
        }
    }
}

/// QUERY 3: Graph-like Traversal (Co-purchase relationships)
pub fn query_graph_lookup(sample_product: &Product, products: &[Product]) {
    println!("\n┌─────────────────────────────────────────────────────────────────┐");
    println!("│ QUERY 3: Graph Lookup - \"Products bought together with this\"   │");
    println!("└─────────────────────────────────────────────────────────────────┘");

    let start = Instant::now();

    let related_ids: &[u64] = &sample_product.related_products;

    println!("  Graph Query: MATCH (p:Product)-[:BOUGHT_TOGETHER]-(other)");
    println!("               WHERE p.id = {}", sample_product.id);
    println!(
        "  Found {} co-purchased products in {:?}\n",
        related_ids.len(),
        start.elapsed()
    );

    for (i, &related_id) in related_ids.iter().take(5).enumerate() {
        if let Some(product) = products.iter().find(|p| p.id == related_id) {
            println!("  {}. {} (co-purchase)", i + 1, product.name);
            println!(
                "     ${:.2} | {} | {}/5 ⭐",
                product.price, product.brand, product.rating
            );
        }
    }
}

/// QUERY 4: Combined Vector + Graph + Filter (Full Power)
pub fn query_combined(
    results: &[SearchResult],
    sample_product: &Product,
    products: &[Product],
) {
    println!("\n┌─────────────────────────────────────────────────────────────────┐");
    println!("│ QUERY 4: COMBINED - Vector + Graph + Filter (Full Power!)      │");
    println!("└─────────────────────────────────────────────────────────────────┘");

    let start = Instant::now();

    println!("  Strategy: Union of:");
    println!("    1. Semantically similar (vector)");
    println!("    2. Frequently bought together (graph)");
    println!(
        "    3. Filtered by: in_stock=true, rating>=4.0, price<${:.0}\n",
        sample_product.price * 1.5
    );

    let graph_neighbors: HashSet<u64> =
        sample_product.related_products.iter().copied().collect();

    let mut combined_scores: HashMap<u64, f32> = HashMap::new();

    // Add vector similarity scores (weight: 0.6)
    for result in results {
        *combined_scores.entry(result.point.id).or_insert(0.0) += result.score * 0.6;
    }

    // Add graph proximity bonus (weight: 0.4)
    for &neighbor_id in &graph_neighbors {
        *combined_scores.entry(neighbor_id).or_insert(0.0) += 0.4;
    }

    // Filter and sort
    let price_threshold = sample_product.price * 1.5;
    let mut final_recommendations: Vec<_> = combined_scores
        .iter()
        .filter_map(|(&id, &score)| {
            products.iter().find(|p| p.id == id).and_then(|p| {
                if p.in_stock
                    && p.rating >= 4.0
                    && p.price < price_threshold
                    && p.id != sample_product.id
                {
                    Some((p, score))
                } else {
                    None
                }
            })
        })
        .collect();

    final_recommendations
        .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    println!(
        "  Found {} recommendations in {:?}\n",
        final_recommendations.len(),
        start.elapsed()
    );

    for (i, (product, score)) in final_recommendations.iter().take(10).enumerate() {
        let source = if graph_neighbors.contains(&product.id) {
            "📊 Graph+Vector"
        } else {
            "🔍 Vector"
        };

        println!(
            "  {}. {} [score: {:.3}] {}",
            i + 1,
            product.name,
            score,
            source
        );
        println!(
            "     ${:.2} | {} | {}/5 ⭐ | {} reviews",
            product.price, product.brand, product.rating, product.review_count
        );
    }
}
