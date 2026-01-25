//! Multi-Model Search Example (EPIC-031)
//!
//! Demonstrates VelesDB's multi-model query capabilities:
//! - Vector similarity search
//! - Graph traversal
//! - Custom ORDER BY expressions
//!
//! Run with: cargo run --example multimodel_search

use std::collections::HashMap;
use velesdb_core::{Database, DatabaseConfig, DistanceMetric, StorageMode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== VelesDB Multi-Model Search Example ===\n");

    // 1. Create database
    let config = DatabaseConfig::default();
    let db = Database::open_with_config("./example_data", config)?;

    // 2. Create collection with embeddings
    let collection = db.create_collection("documents", 384, DistanceMetric::Cosine)?;
    collection.set_storage_mode(StorageMode::Full)?;

    // 3. Insert sample documents with vectors and metadata
    let documents = vec![
        (
            1,
            generate_embedding(384, 0.1),
            serde_json::json!({
                "title": "Introduction to Rust",
                "category": "programming",
                "tags": ["rust", "systems", "performance"]
            }),
        ),
        (
            2,
            generate_embedding(384, 0.2),
            serde_json::json!({
                "title": "Vector Databases Explained",
                "category": "database",
                "tags": ["vectors", "ai", "search"]
            }),
        ),
        (
            3,
            generate_embedding(384, 0.15),
            serde_json::json!({
                "title": "Graph Algorithms in Practice",
                "category": "algorithms",
                "tags": ["graphs", "algorithms", "optimization"]
            }),
        ),
        (
            4,
            generate_embedding(384, 0.25),
            serde_json::json!({
                "title": "Machine Learning with Rust",
                "category": "programming",
                "tags": ["rust", "ml", "ai"]
            }),
        ),
        (
            5,
            generate_embedding(384, 0.3),
            serde_json::json!({
                "title": "Building Search Engines",
                "category": "search",
                "tags": ["search", "indexing", "retrieval"]
            }),
        ),
    ];

    for (id, vector, payload) in &documents {
        collection.upsert(*id, vector.clone(), Some(payload.clone()))?;
    }

    println!("Inserted {} documents\n", documents.len());

    // 4. Example 1: Basic vector search
    println!("--- Example 1: Basic Vector Search ---");
    let query_vector = generate_embedding(384, 0.12);

    let results = collection.search(&query_vector, 3, None)?;
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

    // 5. Example 2: VelesQL query with filter
    println!("--- Example 2: VelesQL with Filter ---");

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM documents WHERE vector NEAR $v AND category = 'programming' LIMIT 5",
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

    // 6. Example 3: Multi-model query with ORDER BY expression
    println!("--- Example 3: ORDER BY Expression ---");

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM documents \
         WHERE vector NEAR $v \
         ORDER BY 0.7 * vector_score + 0.3 * graph_score DESC \
         LIMIT 5",
    )?;

    let results = collection.execute_query(&query, &params)?;
    println!("  Results ordered by custom scoring formula:");
    for result in &results {
        println!(
            "    ID: {}, Fused Score: {:.4}",
            result.point.id, result.score
        );
    }
    println!();

    // 7. Example 4: Hybrid search (vector + text)
    println!("--- Example 4: Hybrid Search ---");

    let query = velesdb_core::velesql::Parser::parse(
        "SELECT * FROM documents \
         WHERE vector NEAR $v AND content MATCH 'rust' \
         LIMIT 5",
    )?;

    let results = collection.execute_query(&query, &params)?;
    println!("  Hybrid search results (vector + text 'rust'):");
    for result in &results {
        println!(
            "    ID: {}, Score: {:.4}",
            result.point.id, result.score
        );
    }
    println!();

    // 8. Cleanup
    db.delete_collection("documents")?;
    println!("=== Example Complete ===");

    Ok(())
}

/// Generate a deterministic embedding for demo purposes
fn generate_embedding(dim: usize, seed: f32) -> Vec<f32> {
    (0..dim)
        .map(|i| ((i as f32 * seed).sin() + seed) / 2.0)
        .collect()
}
