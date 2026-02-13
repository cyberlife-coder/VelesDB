//! Multi-Model Search Example
//!
//! Demonstrates VelesDB's multi-model query capabilities:
//! - Vector similarity search
//! - VelesQL queries with filters
//! - Hybrid search (vector + text)
//!
//! Run with: cargo run --example multimodel_search

use std::collections::HashMap;
use velesdb_core::{Database, DistanceMetric, Point};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== VelesDB Multi-Model Search Example ===\n");

    // 1. Create database with temp directory
    let temp_dir = tempfile::TempDir::new()?;
    let db = Database::open(temp_dir.path())?;

    // 2. Create collection with embeddings
    db.create_collection("documents", 384, DistanceMetric::Cosine)?;
    let collection = db.get_collection("documents").ok_or("Collection not found")?;

    // 3. Insert sample documents with vectors and metadata
    let points = vec![
        Point::new(
            1,
            generate_embedding(384, 0.1),
            Some(serde_json::json!({
                "title": "Introduction to Rust",
                "category": "programming",
                "content": "Rust is a systems programming language"
            })),
        ),
        Point::new(
            2,
            generate_embedding(384, 0.2),
            Some(serde_json::json!({
                "title": "Vector Databases Explained",
                "category": "database",
                "content": "Vector databases enable similarity search"
            })),
        ),
        Point::new(
            3,
            generate_embedding(384, 0.15),
            Some(serde_json::json!({
                "title": "Graph Algorithms in Practice",
                "category": "algorithms",
                "content": "Graph algorithms solve network problems"
            })),
        ),
        Point::new(
            4,
            generate_embedding(384, 0.25),
            Some(serde_json::json!({
                "title": "Machine Learning with Rust",
                "category": "programming",
                "content": "Machine learning in Rust offers performance"
            })),
        ),
        Point::new(
            5,
            generate_embedding(384, 0.3),
            Some(serde_json::json!({
                "title": "Building Search Engines",
                "category": "search",
                "content": "Search engines index and retrieve documents"
            })),
        ),
    ];

    let num_points = points.len();
    collection.upsert(points)?;
    println!("Inserted {} documents\n", num_points);

    // 4. Example 1: Basic vector search
    println!("--- Example 1: Basic Vector Search ---");
    let query_vector = generate_embedding(384, 0.12);

    let results = collection.search(&query_vector, 3)?;
    for result in &results {
        println!(
            "  ID: {}, Score: {:.4}, Title: {}",
            result.point.id,
            result.score,
            result
                .point
                .payload
                .as_ref()
                .and_then(|p| p.get("title"))
                .and_then(|t| t.as_str())
                .unwrap_or("N/A")
        );
    }
    println!();

    // 5. Example 2: VelesQL query with similarity threshold
    println!("--- Example 2: VelesQL with Similarity ---");

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM documents WHERE similarity(vector, $v) > 0.5 AND category = 'programming' LIMIT 5",
    )?;

    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!(query_vector));

    let results = collection.execute_query(&query, &params)?;
    println!("  Found {} results with category='programming'", results.len());
    for result in &results {
        println!(
            "    ID: {}, Score: {:.4}",
            result.point.id, result.score
        );
    }
    println!();

    // 6. Example 3: VelesQL with ORDER BY similarity
    println!("--- Example 3: ORDER BY Similarity ---");

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM documents \
         WHERE similarity(vector, $v) > 0.3 \
         ORDER BY similarity(vector, $v) DESC \
         LIMIT 5",
    )?;

    let results = collection.execute_query(&query, &params)?;
    println!("  Results ordered by similarity:");
    for result in &results {
        println!(
            "    ID: {}, Score: {:.4}",
            result.point.id, result.score
        );
    }
    println!();

    // 7. Example 4: Hybrid search (vector + text)
    println!("--- Example 4: Hybrid Search ---");

    let hybrid_results = collection.hybrid_search(&query_vector, "rust", 5, Some(0.7))?;
    println!("  Hybrid search results (vector + text 'rust'):");
    for result in &hybrid_results {
        println!(
            "    ID: {}, Score: {:.4}, Title: {}",
            result.point.id,
            result.score,
            result
                .point
                .payload
                .as_ref()
                .and_then(|p| p.get("title"))
                .and_then(|t| t.as_str())
                .unwrap_or("N/A")
        );
    }
    println!();

    // 8. Example 5: Text-only search
    println!("--- Example 5: Text Search ---");
    let text_results = collection.text_search("programming", 3);
    println!("  Text search results for 'programming':");
    for result in &text_results {
        println!(
            "    ID: {}, Score: {:.4}, Title: {}",
            result.point.id,
            result.score,
            result
                .point
                .payload
                .as_ref()
                .and_then(|p| p.get("title"))
                .and_then(|t| t.as_str())
                .unwrap_or("N/A")
        );
    }

    println!("\n=== Example Complete ===");
    Ok(())
}

/// Generate a deterministic embedding for demo purposes
fn generate_embedding(dim: usize, seed: f32) -> Vec<f32> {
    (0..dim)
        .map(|i| ((i as f32 * seed).sin() + seed) / 2.0)
        .collect()
}
