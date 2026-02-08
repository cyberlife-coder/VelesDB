//! Metrics & Fusion scenario tests (Scenarios 0b/0c) — Tests created by Phase 4 Plan 03.
//!
//! **Scenario 0c**: All 5 distance metrics produce correct search results with
//! proper ordering semantics (higher_is_better vs lower_is_better).
//!
//! **Scenario 0b**: Multi-vector fusion via `multi_query_search()` API with
//! all 4 strategies (RRF, Average, Maximum, Weighted).

use std::collections::HashMap;

use serde_json::json;

use velesdb_core::velesql::Parser;
use velesdb_core::DistanceMetric;

use crate::helpers;

/// Converts a f32 embedding vector to a JSON array for query parameters.
fn embedding_to_json_param(vec: &[f32]) -> serde_json::Value {
    serde_json::Value::Array(
        vec.iter()
            .map(|&f| serde_json::Value::from(f64::from(f)))
            .collect(),
    )
}

// ============================================================================
// Scenario 0c: All 5 Distance Metrics
// ============================================================================

/// Cosine similarity (NLP/Semantic Search):
/// ```sql
/// SELECT * FROM documents
/// WHERE vector NEAR $query
/// ORDER BY similarity(vector, $query) DESC
/// LIMIT 10
/// ```
#[test]
fn test_scenario0c_cosine_metric() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "documents", 128, DistanceMetric::Cosine);

    let nodes: Vec<(u64, Vec<f32>, serde_json::Value)> = (1..=12)
        .map(|i| {
            (
                i,
                helpers::generate_embedding(100 + i, 128),
                json!({"doc_id": format!("D{:03}", i), "topic": "nlp"}),
            )
        })
        .collect();
    helpers::insert_labeled_nodes(&collection, &nodes);

    let query_str = "\
        SELECT doc_id, topic \
        FROM documents \
        WHERE vector NEAR $query \
        ORDER BY similarity(vector, $query) DESC \
        LIMIT 10";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    let query_vec = helpers::generate_embedding(101, 128);
    let mut params = HashMap::new();
    params.insert("query".to_string(), embedding_to_json_param(&query_vec));

    let results = collection
        .execute_query(&parsed, &params)
        .expect("Cosine query should execute");

    assert!(!results.is_empty(), "Cosine search should return results");
    assert!(results.len() <= 10, "LIMIT 10 should be respected");

    // Cosine: higher_is_better=true → DESC means scores non-increasing
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Cosine DESC: {:.6} >= {:.6}",
            window[0].score,
            window[1].score
        );
    }

    // First result should be the point with seed 101 (identical to query)
    // Seed = 100 + i, so seed 101 → id 1
    assert_eq!(
        results[0].point.id, 1,
        "Point with same seed as query should be most similar"
    );
}

/// Euclidean distance (Spatial/Clustering):
/// ```sql
/// SELECT * FROM locations
/// WHERE vector NEAR $gps AND category = 'restaurant'
/// ORDER BY similarity(vector, $gps) DESC
/// LIMIT 5
/// ```
///
/// Note: DESC means "most similar first" for ALL metrics.
/// For Euclidean, this sorts by ascending distance (closest first).
#[test]
fn test_scenario0c_euclidean_metric() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "locations", 3, DistanceMetric::Euclidean);

    // 3D spatial points — restaurants and cafes
    let nodes = vec![
        (
            1,
            vec![1.0, 2.0, 0.0],
            json!({"name": "Pizza Place", "category": "restaurant"}),
        ),
        (
            2,
            vec![1.5, 2.5, 0.0],
            json!({"name": "Sushi Bar", "category": "restaurant"}),
        ),
        (
            3,
            vec![10.0, 10.0, 0.0],
            json!({"name": "Distant Diner", "category": "restaurant"}),
        ),
        (
            4,
            vec![1.2, 2.2, 0.0],
            json!({"name": "Taco Truck", "category": "restaurant"}),
        ),
        (
            5,
            vec![1.1, 2.1, 0.0],
            json!({"name": "Burger Joint", "category": "restaurant"}),
        ),
        (
            6,
            vec![0.5, 1.0, 0.0],
            json!({"name": "Cafe Latte", "category": "cafe"}),
        ),
        (
            7,
            vec![0.8, 1.5, 0.0],
            json!({"name": "Tea House", "category": "cafe"}),
        ),
        (
            8,
            vec![5.0, 5.0, 0.0],
            json!({"name": "Steakhouse", "category": "restaurant"}),
        ),
        (
            9,
            vec![1.3, 2.3, 0.0],
            json!({"name": "Noodle Shop", "category": "restaurant"}),
        ),
        (
            10,
            vec![2.0, 3.0, 0.0],
            json!({"name": "Ramen Place", "category": "restaurant"}),
        ),
    ];
    helpers::insert_labeled_nodes(&collection, &nodes);

    let query_str = "\
        SELECT name, category \
        FROM locations \
        WHERE vector NEAR $gps \
          AND category = 'restaurant' \
        ORDER BY similarity(vector, $gps) DESC \
        LIMIT 5";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    // Query point near (1.0, 2.0, 0.0)
    let query_vec = vec![1.0, 2.0, 0.0];
    let mut params = HashMap::new();
    params.insert("gps".to_string(), embedding_to_json_param(&query_vec));

    let results = collection
        .execute_query(&parsed, &params)
        .expect("Euclidean query should execute");

    assert!(
        !results.is_empty(),
        "Euclidean search should return results"
    );
    assert!(results.len() <= 5, "LIMIT 5 should be respected");

    // All results should be restaurants (category filter)
    for r in &results {
        let payload = r.point.payload.as_ref().expect("Should have payload");
        let category = payload
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(
            category, "restaurant",
            "Only restaurants should be returned"
        );
    }

    // Euclidean: higher_is_better=false → DESC means ascending distance (closest first)
    // Scores are raw Euclidean distances, so score[i] <= score[i+1]
    for window in results.windows(2) {
        assert!(
            window[0].score <= window[1].score,
            "Euclidean DESC (closest first): {:.6} <= {:.6}",
            window[0].score,
            window[1].score
        );
    }

    // Pizza Place (1.0, 2.0) is at distance 0 from query
    assert_eq!(
        results[0].point.id, 1,
        "Closest point should be Pizza Place (id=1)"
    );
}

/// DotProduct (RAG/Recommendations):
/// ```sql
/// SELECT * FROM products
/// WHERE vector NEAR $user_pref AND in_stock = true
/// ORDER BY similarity(vector, $user_pref) DESC
/// LIMIT 8
/// ```
#[test]
fn test_scenario0c_dotproduct_metric() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "products", 128, DistanceMetric::DotProduct);

    let nodes: Vec<(u64, Vec<f32>, serde_json::Value)> = (1..=12)
        .map(|i| {
            (
                i,
                helpers::generate_embedding(200 + i, 128),
                json!({
                    "product_id": format!("PROD{:03}", i),
                    "in_stock": i % 3 != 0 // 2 out of 3 in stock
                }),
            )
        })
        .collect();
    helpers::insert_labeled_nodes(&collection, &nodes);

    let query_str = "\
        SELECT product_id \
        FROM products \
        WHERE vector NEAR $user_pref \
          AND in_stock = true \
        ORDER BY similarity(vector, $user_pref) DESC \
        LIMIT 8";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    let query_vec = helpers::generate_embedding(201, 128);
    let mut params = HashMap::new();
    params.insert("user_pref".to_string(), embedding_to_json_param(&query_vec));

    let results = collection
        .execute_query(&parsed, &params)
        .expect("DotProduct query should execute");

    assert!(
        !results.is_empty(),
        "DotProduct search should return results"
    );
    assert!(results.len() <= 8, "LIMIT 8 should be respected");

    // All results should have in_stock = true
    for r in &results {
        let payload = r.point.payload.as_ref().expect("Should have payload");
        let in_stock = payload
            .get("in_stock")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        assert!(in_stock, "Only in-stock products should be returned");
    }

    // DotProduct: higher_is_better=true → DESC means scores non-increasing
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "DotProduct DESC: {:.6} >= {:.6}",
            window[0].score,
            window[1].score
        );
    }
}

/// Hamming distance (Binary Vectors):
/// ```sql
/// SELECT * FROM image_hashes
/// WHERE vector NEAR $hash AND source = 'user_uploads'
/// ORDER BY similarity(vector, $hash) DESC
/// LIMIT 10
/// ```
///
/// Note: DESC means "most similar first". For Hamming, this sorts by
/// ascending distance (fewest bit differences first).
#[test]
fn test_scenario0c_hamming_metric() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "image_hashes", 64, DistanceMetric::Hamming);

    // Binary-like vectors (0.0/1.0 values)
    let mut nodes = Vec::new();
    for i in 1..=12_u64 {
        #[allow(clippy::cast_precision_loss)] // Reason: i is small test constant
        let vec: Vec<f32> = (0..64)
            .map(|j| if ((i + j) % 3) == 0 { 1.0 } else { 0.0 })
            .collect();
        let source = if i <= 8 { "user_uploads" } else { "web_scrape" };
        nodes.push((
            i,
            vec,
            json!({"hash_id": format!("H{:03}", i), "source": source}),
        ));
    }
    helpers::insert_labeled_nodes(&collection, &nodes);

    let query_str = "\
        SELECT hash_id, source \
        FROM image_hashes \
        WHERE vector NEAR $hash \
          AND source = 'user_uploads' \
        ORDER BY similarity(vector, $hash) DESC \
        LIMIT 10";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    // Query = same pattern as point 1
    let query_vec: Vec<f32> = (0..64)
        .map(|j| if ((1 + j) % 3) == 0 { 1.0 } else { 0.0 })
        .collect();
    let mut params = HashMap::new();
    params.insert("hash".to_string(), embedding_to_json_param(&query_vec));

    let results = collection
        .execute_query(&parsed, &params)
        .expect("Hamming query should execute");

    assert!(!results.is_empty(), "Hamming search should return results");
    assert!(results.len() <= 10, "LIMIT 10 should be respected");

    // All results should be from user_uploads
    for r in &results {
        let payload = r.point.payload.as_ref().expect("Should have payload");
        let source = payload.get("source").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(
            source, "user_uploads",
            "Only user_uploads should be returned"
        );
    }

    // Hamming: higher_is_better=false → DESC means ascending distance (fewest diffs first)
    for window in results.windows(2) {
        assert!(
            window[0].score <= window[1].score,
            "Hamming DESC (closest first): {:.6} <= {:.6}",
            window[0].score,
            window[1].score
        );
    }

    // First result should be point 1 (identical to query, distance=0)
    assert_eq!(
        results[0].point.id, 1,
        "Identical hash should be most similar (distance 0)"
    );
    assert!(
        results[0].score.abs() < f32::EPSILON,
        "Distance to identical hash should be 0, got {}",
        results[0].score
    );
}

/// Jaccard similarity (Set Similarity):
/// ```sql
/// SELECT * FROM user_tags
/// WHERE vector NEAR $tags
/// ORDER BY similarity(vector, $tags) DESC
/// LIMIT 20
/// ```
#[test]
fn test_scenario0c_jaccard_metric() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "user_tags", 32, DistanceMetric::Jaccard);

    // Sparse-like vectors (many zeros, some ones) representing tag sets
    let mut nodes = Vec::new();
    for i in 1..=20_u64 {
        let vec: Vec<f32> = (0..32)
            .map(|j| {
                if ((i + j) % 5) == 0 || j < i.min(8) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        nodes.push((
            i,
            vec,
            json!({"user_id": format!("U{:03}", i), "active": true}),
        ));
    }
    helpers::insert_labeled_nodes(&collection, &nodes);

    let query_str = "\
        SELECT user_id \
        FROM user_tags \
        WHERE vector NEAR $tags \
        ORDER BY similarity(vector, $tags) DESC \
        LIMIT 20";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    // Query = same pattern as point 1
    let query_vec: Vec<f32> = (0..32)
        .map(|j| {
            if ((1 + j) % 5) == 0 || j < 1 {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let mut params = HashMap::new();
    params.insert("tags".to_string(), embedding_to_json_param(&query_vec));

    let results = collection
        .execute_query(&parsed, &params)
        .expect("Jaccard query should execute");

    assert!(!results.is_empty(), "Jaccard search should return results");
    assert!(results.len() <= 20, "LIMIT 20 should be respected");

    // Jaccard: higher_is_better=true → DESC means scores non-increasing
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Jaccard DESC: {:.6} >= {:.6}",
            window[0].score,
            window[1].score
        );
    }

    // First result should be point 1 (identical set, Jaccard=1.0)
    assert_eq!(
        results[0].point.id, 1,
        "Identical tag set should be most similar"
    );
    assert!(
        (results[0].score - 1.0).abs() < 1e-4,
        "Jaccard similarity to identical set should be ~1.0, got {}",
        results[0].score
    );
}

// ============================================================================
// Scenario 0b: NEAR_FUSED Multi-Vector Fusion
// ============================================================================

/// Tests NEAR_FUSED VelesQL syntax parsing.
/// Verifies the parser accepts the README fusion query format.
#[test]
fn test_scenario0b_near_fused_parsing() {
    let query_str = "\
        SELECT * FROM products \
        WHERE vector NEAR_FUSED [$text_embedding, $image_embedding] \
          USING FUSION 'rrf' (k = 60) \
          AND category = 'electronics' \
        ORDER BY similarity(vector, $text_embedding) DESC \
        LIMIT 10";

    let parsed = Parser::parse(query_str);
    assert!(
        parsed.is_ok(),
        "NEAR_FUSED query should parse: {:?}",
        parsed.err()
    );
}

/// Tests RRF fusion via `multi_query_search()` API with category filter.
#[test]
fn test_scenario0b_fusion_rrf() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "products_rrf", 32, DistanceMetric::Cosine);

    let nodes: Vec<(u64, Vec<f32>, serde_json::Value)> = (1..=20)
        .map(|i| {
            let category = if i <= 12 { "electronics" } else { "clothing" };
            (
                i,
                helpers::generate_embedding(300 + i, 32),
                json!({"product": format!("P{:03}", i), "category": category}),
            )
        })
        .collect();
    helpers::insert_labeled_nodes(&collection, &nodes);

    let text_embedding = helpers::generate_embedding(301, 32);
    let image_embedding = helpers::generate_embedding(305, 32);
    let filter = velesdb_core::filter::Filter::new(velesdb_core::filter::Condition::eq(
        "category",
        "electronics",
    ));

    let results = collection
        .multi_query_search(
            &[&text_embedding, &image_embedding],
            10,
            velesdb_core::fusion::FusionStrategy::RRF { k: 60 },
            Some(&filter),
        )
        .expect("RRF fusion should succeed");

    assert!(!results.is_empty(), "RRF fusion should return results");
    assert!(results.len() <= 10, "LIMIT 10 should be respected");

    // All results should be electronics
    for r in &results {
        let payload = r.point.payload.as_ref().expect("Should have payload");
        let category = payload
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(
            category, "electronics",
            "Filter should restrict to electronics"
        );
    }

    // Scores should be non-zero and non-increasing
    for r in &results {
        assert!(
            r.score > 0.0,
            "RRF scores should be positive, got {}",
            r.score
        );
    }
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Fused scores should be non-increasing: {:.6} >= {:.6}",
            window[0].score,
            window[1].score
        );
    }
}

/// Tests Average fusion via `multi_query_search()` API.
#[test]
fn test_scenario0b_fusion_average() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "products_avg", 32, DistanceMetric::Cosine);

    let nodes: Vec<(u64, Vec<f32>, serde_json::Value)> = (1..=20)
        .map(|i| {
            (
                i,
                helpers::generate_embedding(400 + i, 32),
                json!({"product": format!("P{:03}", i), "category": "electronics"}),
            )
        })
        .collect();
    helpers::insert_labeled_nodes(&collection, &nodes);

    let v1 = helpers::generate_embedding(401, 32);
    let v2 = helpers::generate_embedding(410, 32);

    let results = collection
        .multi_query_search(
            &[&v1, &v2],
            10,
            velesdb_core::fusion::FusionStrategy::Average,
            None,
        )
        .expect("Average fusion should succeed");

    assert!(!results.is_empty(), "Average fusion should return results");
    assert!(results.len() <= 10, "LIMIT 10 should be respected");

    // Scores should be non-increasing
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Average scores non-increasing: {:.6} >= {:.6}",
            window[0].score,
            window[1].score
        );
    }
}

/// Tests Maximum fusion via `multi_query_search()` API.
#[test]
fn test_scenario0b_fusion_maximum() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "products_max", 32, DistanceMetric::Cosine);

    let nodes: Vec<(u64, Vec<f32>, serde_json::Value)> = (1..=20)
        .map(|i| {
            (
                i,
                helpers::generate_embedding(500 + i, 32),
                json!({"product": format!("P{:03}", i)}),
            )
        })
        .collect();
    helpers::insert_labeled_nodes(&collection, &nodes);

    let v1 = helpers::generate_embedding(501, 32);
    let v2 = helpers::generate_embedding(515, 32);

    let results = collection
        .multi_query_search(
            &[&v1, &v2],
            10,
            velesdb_core::fusion::FusionStrategy::Maximum,
            None,
        )
        .expect("Maximum fusion should succeed");

    assert!(!results.is_empty(), "Maximum fusion should return results");
    assert!(results.len() <= 10, "LIMIT 10 should be respected");

    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Maximum scores non-increasing: {:.6} >= {:.6}",
            window[0].score,
            window[1].score
        );
    }
}

/// Tests Weighted fusion via `multi_query_search()` API.
#[test]
fn test_scenario0b_fusion_weighted() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "products_wt", 32, DistanceMetric::Cosine);

    let nodes: Vec<(u64, Vec<f32>, serde_json::Value)> = (1..=20)
        .map(|i| {
            (
                i,
                helpers::generate_embedding(600 + i, 32),
                json!({"product": format!("P{:03}", i)}),
            )
        })
        .collect();
    helpers::insert_labeled_nodes(&collection, &nodes);

    let v1 = helpers::generate_embedding(601, 32);
    let v2 = helpers::generate_embedding(605, 32);
    let v3 = helpers::generate_embedding(610, 32);

    let results = collection
        .multi_query_search(
            &[&v1, &v2, &v3],
            10,
            velesdb_core::fusion::FusionStrategy::Weighted {
                avg_weight: 0.5,
                max_weight: 0.3,
                hit_weight: 0.2,
            },
            None,
        )
        .expect("Weighted fusion should succeed");

    assert!(!results.is_empty(), "Weighted fusion should return results");
    assert!(results.len() <= 10, "LIMIT 10 should be respected");

    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Weighted scores non-increasing: {:.6} >= {:.6}",
            window[0].score,
            window[1].score
        );
    }
}
