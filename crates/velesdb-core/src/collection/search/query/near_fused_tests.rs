//! Tests for NEAR_FUSED execution wiring (VP-012, Plan 06-01).
//!
//! Tests verify that `VectorFusedSearch` conditions in VelesQL queries
//! are correctly dispatched to `multi_query_search()` with proper
//! fusion strategy mapping and metadata filtering.

use std::collections::HashMap;

use serde_json::json;

use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use crate::velesql::{
    CompareOp, Comparison, Condition, DistinctMode, FusionConfig, Query, SelectColumns,
    SelectStatement, Value, VectorExpr, VectorFusedSearch, VectorSearch,
};
use crate::Database;

/// Helper: create a collection with diverse vectors for NEAR_FUSED testing.
fn setup_near_fused_collection() -> (tempfile::TempDir, Collection) {
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let db = Database::open(temp_dir.path()).expect("Failed to open database");

    db.create_collection("docs", 4, DistanceMetric::Cosine)
        .expect("Failed to create collection");
    let collection = db.get_collection("docs").expect("Failed to get collection");

    // Insert 6 docs with different vectors and metadata
    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"category": "tech", "score": 90})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"category": "tech", "score": 80})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"category": "science", "score": 70})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"category": "science", "score": 60})),
        ),
        Point::new(
            5,
            vec![0.7, 0.7, 0.0, 0.0],
            Some(json!({"category": "tech", "score": 85})),
        ),
        Point::new(
            6,
            vec![0.0, 0.7, 0.7, 0.0],
            Some(json!({"category": "science", "score": 75})),
        ),
    ];

    collection.upsert(points).expect("Failed to upsert points");
    (temp_dir, collection)
}

/// Helper: build a SELECT * FROM docs WHERE [condition] LIMIT [limit] query.
fn make_query(where_clause: Option<Condition>, limit: Option<u64>) -> Query {
    Query {
        select: SelectStatement {
            distinct: DistinctMode::None,
            columns: SelectColumns::All,
            from: "docs".to_string(),
            from_alias: None,
            joins: Vec::new(),
            where_clause,
            order_by: None,
            limit,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
        compound: None,
        match_clause: None,
    }
}

/// Helper: build a NEAR_FUSED condition with parameter vectors.
fn make_near_fused(
    param_names: &[&str],
    strategy: &str,
    params: HashMap<String, f64>,
) -> Condition {
    let vectors: Vec<VectorExpr> = param_names
        .iter()
        .map(|name| VectorExpr::Parameter(name.to_string()))
        .collect();
    Condition::VectorFusedSearch(VectorFusedSearch {
        vectors,
        fusion: FusionConfig {
            strategy: strategy.to_string(),
            params,
        },
    })
}

/// Helper: build query params with named vectors.
fn make_vector_params(entries: &[(&str, Vec<f32>)]) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    for (name, vec) in entries {
        params.insert(
            name.to_string(),
            serde_json::Value::Array(vec.iter().map(|&v| json!(v)).collect()),
        );
    }
    params
}

// ============================================================================
// Basic NEAR_FUSED execution tests
// ============================================================================

#[test]
fn test_near_fused_rrf_basic() {
    let (_dir, collection) = setup_near_fused_collection();

    let cond = make_near_fused(&["v1", "v2"], "rrf", HashMap::new());
    let query = make_query(Some(cond), Some(5));
    let params = make_vector_params(&[
        ("v1", vec![1.0, 0.0, 0.0, 0.0]),
        ("v2", vec![0.0, 1.0, 0.0, 0.0]),
    ]);

    let results = collection
        .execute_query(&query, &params)
        .expect("NEAR_FUSED RRF should work");
    assert!(!results.is_empty(), "Should return results");
    assert!(results.len() <= 5, "Should respect LIMIT");
}

#[test]
fn test_near_fused_average_strategy() {
    let (_dir, collection) = setup_near_fused_collection();

    let cond = make_near_fused(&["v1", "v2"], "average", HashMap::new());
    let query = make_query(Some(cond), Some(5));
    let params = make_vector_params(&[
        ("v1", vec![1.0, 0.0, 0.0, 0.0]),
        ("v2", vec![0.0, 0.0, 1.0, 0.0]),
    ]);

    let results = collection
        .execute_query(&query, &params)
        .expect("Average fusion should work");
    assert!(!results.is_empty(), "Should return results");
}

#[test]
fn test_near_fused_maximum_strategy() {
    let (_dir, collection) = setup_near_fused_collection();

    let cond = make_near_fused(&["v1", "v2"], "maximum", HashMap::new());
    let query = make_query(Some(cond), Some(5));
    let params = make_vector_params(&[
        ("v1", vec![1.0, 0.0, 0.0, 0.0]),
        ("v2", vec![0.0, 1.0, 0.0, 0.0]),
    ]);

    let results = collection
        .execute_query(&query, &params)
        .expect("Maximum fusion should work");
    assert!(!results.is_empty(), "Should return results");
}

#[test]
fn test_near_fused_weighted_strategy() {
    let (_dir, collection) = setup_near_fused_collection();

    let mut fusion_params = HashMap::new();
    fusion_params.insert("avg_weight".to_string(), 0.5);
    fusion_params.insert("max_weight".to_string(), 0.3);
    fusion_params.insert("hit_weight".to_string(), 0.2);

    let cond = make_near_fused(&["v1", "v2"], "weighted", fusion_params);
    let query = make_query(Some(cond), Some(5));
    let params = make_vector_params(&[
        ("v1", vec![1.0, 0.0, 0.0, 0.0]),
        ("v2", vec![0.0, 1.0, 0.0, 0.0]),
    ]);

    let results = collection
        .execute_query(&query, &params)
        .expect("Weighted fusion should work");
    assert!(!results.is_empty(), "Should return results");
}

// ============================================================================
// Metadata filter tests
// ============================================================================

#[test]
fn test_near_fused_with_metadata_filter() {
    let (_dir, collection) = setup_near_fused_collection();

    // NEAR_FUSED AND category = 'tech'
    let fused = make_near_fused(&["v1", "v2"], "rrf", HashMap::new());
    let filter = Condition::Comparison(Comparison {
        column: "category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    });
    let cond = Condition::And(Box::new(fused), Box::new(filter));

    let query = make_query(Some(cond), Some(10));
    let params = make_vector_params(&[
        ("v1", vec![1.0, 0.0, 0.0, 0.0]),
        ("v2", vec![0.0, 1.0, 0.0, 0.0]),
    ]);

    let results = collection
        .execute_query(&query, &params)
        .expect("NEAR_FUSED + filter should work");
    // All results should have category = "tech"
    for r in &results {
        if let Some(ref payload) = r.point.payload {
            assert_eq!(
                payload.get("category").and_then(|v| v.as_str()),
                Some("tech"),
                "All results should be tech category"
            );
        }
    }
}

// ============================================================================
// LIMIT tests
// ============================================================================

#[test]
fn test_near_fused_with_limit() {
    let (_dir, collection) = setup_near_fused_collection();

    let cond = make_near_fused(&["v1", "v2"], "rrf", HashMap::new());
    let query = make_query(Some(cond), Some(2));
    let params = make_vector_params(&[
        ("v1", vec![1.0, 0.0, 0.0, 0.0]),
        ("v2", vec![0.0, 1.0, 0.0, 0.0]),
    ]);

    let results = collection
        .execute_query(&query, &params)
        .expect("NEAR_FUSED with LIMIT should work");
    assert!(
        results.len() <= 2,
        "LIMIT 2 should be respected, got {}",
        results.len()
    );
}

// ============================================================================
// Single vector degenerate case
// ============================================================================

#[test]
fn test_near_fused_single_vector() {
    let (_dir, collection) = setup_near_fused_collection();

    let cond = make_near_fused(&["v1"], "rrf", HashMap::new());
    let query = make_query(Some(cond), Some(5));
    let params = make_vector_params(&[("v1", vec![1.0, 0.0, 0.0, 0.0])]);

    let results = collection
        .execute_query(&query, &params)
        .expect("Single vector NEAR_FUSED should work");
    assert!(
        !results.is_empty(),
        "Should return results even with 1 vector"
    );
}

// ============================================================================
// Parameter resolution tests
// ============================================================================

#[test]
fn test_near_fused_parameter_resolution() {
    let (_dir, collection) = setup_near_fused_collection();

    // Use parameter names
    let cond = make_near_fused(&["query_a", "query_b"], "rrf", HashMap::new());
    let query = make_query(Some(cond), Some(3));
    let params = make_vector_params(&[
        ("query_a", vec![0.5, 0.5, 0.0, 0.0]),
        ("query_b", vec![0.0, 0.5, 0.5, 0.0]),
    ]);

    let results = collection
        .execute_query(&query, &params)
        .expect("Parameter resolution should work");
    assert!(!results.is_empty(), "Should return results");
}

// ============================================================================
// Error case tests
// ============================================================================

#[test]
fn test_near_fused_plus_similarity_is_error() {
    let (_dir, collection) = setup_near_fused_collection();

    // NEAR_FUSED AND similarity() — should be rejected
    let fused = make_near_fused(&["v1", "v2"], "rrf", HashMap::new());
    let sim = Condition::Similarity(crate::velesql::SimilarityCondition {
        field: "vector".to_string(),
        vector: VectorExpr::Parameter("v1".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.5,
    });
    let cond = Condition::And(Box::new(fused), Box::new(sim));

    let query = make_query(Some(cond), Some(5));
    let params = make_vector_params(&[
        ("v1", vec![1.0, 0.0, 0.0, 0.0]),
        ("v2", vec![0.0, 1.0, 0.0, 0.0]),
    ]);

    let result = collection.execute_query(&query, &params);
    assert!(
        result.is_err(),
        "NEAR_FUSED + similarity() should be an error"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("NEAR_FUSED") || err_msg.contains("fused"),
        "Error should mention NEAR_FUSED: {err_msg}"
    );
}

#[test]
fn test_near_fused_unknown_strategy_is_error() {
    let (_dir, collection) = setup_near_fused_collection();

    let cond = make_near_fused(&["v1", "v2"], "unknown_strategy", HashMap::new());
    let query = make_query(Some(cond), Some(5));
    let params = make_vector_params(&[
        ("v1", vec![1.0, 0.0, 0.0, 0.0]),
        ("v2", vec![0.0, 1.0, 0.0, 0.0]),
    ]);

    let result = collection.execute_query(&query, &params);
    assert!(
        result.is_err(),
        "Unknown fusion strategy should be an error"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Unknown fusion strategy") || err_msg.contains("Supported"),
        "Error should list supported strategies: {err_msg}"
    );
}

#[test]
fn test_near_fused_missing_parameter_is_error() {
    let (_dir, collection) = setup_near_fused_collection();

    let cond = make_near_fused(&["v1", "v_missing"], "rrf", HashMap::new());
    let query = make_query(Some(cond), Some(5));
    // Only provide v1, not v_missing
    let params = make_vector_params(&[("v1", vec![1.0, 0.0, 0.0, 0.0])]);

    let result = collection.execute_query(&query, &params);
    assert!(result.is_err(), "Missing parameter should be an error");
}

#[test]
fn test_near_fused_in_match_where_is_error() {
    use crate::velesql::{GraphPattern, MatchClause, NodePattern, ReturnClause};

    // Create a collection with labeled nodes so find_start_nodes returns matches
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let db = Database::open(temp_dir.path()).expect("Failed to open database");
    db.create_collection("labeled", 4, DistanceMetric::Cosine)
        .expect("Failed to create collection");
    let collection = db
        .get_collection("labeled")
        .expect("Failed to get collection");

    // Nodes with _labels so MATCH (p:Product) finds them
    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"_labels": ["Product"], "name": "A"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"_labels": ["Product"], "name": "B"})),
        ),
    ];
    collection.upsert(points).expect("Failed to upsert");

    // MATCH (p:Product) WHERE NEAR_FUSED — should be error, not silently pass
    let match_clause = MatchClause {
        patterns: vec![GraphPattern {
            name: None,
            nodes: vec![NodePattern::new().with_alias("p").with_label("Product")],
            relationships: vec![],
        }],
        where_clause: Some(make_near_fused(&["v1"], "rrf", HashMap::new())),
        return_clause: ReturnClause {
            items: vec![],
            order_by: None,
            limit: Some(10),
        },
    };

    let params = make_vector_params(&[("v1", vec![1.0, 0.0, 0.0, 0.0])]);
    let result = collection.execute_match(&match_clause, &params);
    assert!(
        result.is_err(),
        "NEAR_FUSED in MATCH WHERE should be an error, not silently pass"
    );
}

#[test]
fn test_near_fused_plus_near_is_error() {
    let (_dir, collection) = setup_near_fused_collection();

    // NEAR AND NEAR_FUSED — conflicting vector search modes
    let near = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v1".to_string()),
    });
    let fused = make_near_fused(&["v2", "v3"], "rrf", HashMap::new());
    let cond = Condition::And(Box::new(near), Box::new(fused));

    let query = make_query(Some(cond), Some(5));
    let params = make_vector_params(&[
        ("v1", vec![1.0, 0.0, 0.0, 0.0]),
        ("v2", vec![0.0, 1.0, 0.0, 0.0]),
        ("v3", vec![0.0, 0.0, 1.0, 0.0]),
    ]);

    let result = collection.execute_query(&query, &params);
    assert!(result.is_err(), "NEAR + NEAR_FUSED should be an error");
}
