//! BDD-style integration tests for vector-search GROUP BY (Issue #511).
//!
//! Tests the full pipeline: SQL string → Parser → Database::execute_query → verify results.
//! Covers nominal, edge-case, negative, and combination scenarios.

use std::collections::HashMap;

use velesdb_core::{velesql::Parser, Database, Point};

use super::helpers::{create_test_db, execute_sql_with_params, vector_param};

// =========================================================================
// Setup helper
// =========================================================================

/// Seeds a "chunks" collection with 3 parents × 3 chunks each (9 points, dim=4).
///
/// | id | parent_id | text            | category | vector (approx)       |
/// |----|-----------|-----------------|----------|-----------------------|
/// |  1 | doc-1     | doc1 chunk high | science  | [1.0, 0.0, 0.0, 0.0] |
/// |  2 | doc-1     | doc1 chunk mid  | science  | [0.9, 0.1, 0.0, 0.0] |
/// |  3 | doc-1     | doc1 chunk low  | science  | [0.8, 0.2, 0.0, 0.0] |
/// |  4 | doc-2     | doc2 chunk high | tech     | [0.7, 0.3, 0.0, 0.0] |
/// |  5 | doc-2     | doc2 chunk mid  | tech     | [0.6, 0.4, 0.0, 0.0] |
/// |  6 | doc-2     | doc2 chunk low  | tech     | [0.5, 0.5, 0.0, 0.0] |
/// |  7 | doc-3     | doc3 chunk high | science  | [0.4, 0.6, 0.0, 0.0] |
/// |  8 | doc-3     | doc3 chunk mid  | science  | [0.3, 0.7, 0.0, 0.0] |
/// |  9 | doc-3     | doc3 chunk low  | science  | [0.2, 0.8, 0.0, 0.0] |
fn setup_chunked_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION chunks (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create chunks");

    let vc = db
        .get_vector_collection("chunks")
        .expect("test: get chunks");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(serde_json::json!({"parent_id":"doc-1","text":"doc1 chunk high","category":"science"}))),
        Point::new(2, vec![0.9, 0.1, 0.0, 0.0], Some(serde_json::json!({"parent_id":"doc-1","text":"doc1 chunk mid","category":"science"}))),
        Point::new(3, vec![0.8, 0.2, 0.0, 0.0], Some(serde_json::json!({"parent_id":"doc-1","text":"doc1 chunk low","category":"science"}))),
        Point::new(4, vec![0.7, 0.3, 0.0, 0.0], Some(serde_json::json!({"parent_id":"doc-2","text":"doc2 chunk high","category":"tech"}))),
        Point::new(5, vec![0.6, 0.4, 0.0, 0.0], Some(serde_json::json!({"parent_id":"doc-2","text":"doc2 chunk mid","category":"tech"}))),
        Point::new(6, vec![0.5, 0.5, 0.0, 0.0], Some(serde_json::json!({"parent_id":"doc-2","text":"doc2 chunk low","category":"tech"}))),
        Point::new(7, vec![0.4, 0.6, 0.0, 0.0], Some(serde_json::json!({"parent_id":"doc-3","text":"doc3 chunk high","category":"science"}))),
        Point::new(8, vec![0.3, 0.7, 0.0, 0.0], Some(serde_json::json!({"parent_id":"doc-3","text":"doc3 chunk mid","category":"science"}))),
        Point::new(9, vec![0.2, 0.8, 0.0, 0.0], Some(serde_json::json!({"parent_id":"doc-3","text":"doc3 chunk low","category":"science"}))),
    ])
    .expect("test: upsert chunks");
}

/// Execute a VelesQL SQL string through the full pipeline.
fn execute_sql(db: &Database, sql: &str) -> velesdb_core::Result<Vec<velesdb_core::SearchResult>> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    db.execute_query(&query, &HashMap::new())
}

// =========================================================================
// Scenario 1: Nominal — GROUP BY MAX(score) returns one result per parent
// =========================================================================

#[test]
fn test_given_chunked_collection_when_group_by_max_score_then_one_result_per_parent() {
    let (_dir, db) = create_test_db();
    setup_chunked_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT parent_id, MAX(score) AS relevance FROM chunks WHERE vector NEAR $v GROUP BY parent_id LIMIT 10",
        &params,
    )
    .expect("test: GROUP BY MAX(score)");

    // Should have exactly 3 groups (doc-1, doc-2, doc-3).
    assert_eq!(results.len(), 3, "Expected 3 groups, got {}", results.len());

    // Each result should have a parent_id in its payload.
    let parent_ids: std::collections::HashSet<String> = results
        .iter()
        .filter_map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("parent_id"))
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();
    assert_eq!(parent_ids.len(), 3, "All 3 parent_ids should be unique");
    assert!(parent_ids.contains("doc-1"));
    assert!(parent_ids.contains("doc-2"));
    assert!(parent_ids.contains("doc-3"));
}

// =========================================================================
// Scenario 2: Nominal — GROUP BY AVG(score) returns correct averages
// =========================================================================

#[test]
fn test_given_chunked_collection_when_group_by_avg_score_then_correct_averages() {
    let (_dir, db) = create_test_db();
    setup_chunked_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT parent_id, AVG(score) AS avg_sim FROM chunks WHERE vector NEAR $v GROUP BY parent_id LIMIT 10",
        &params,
    )
    .expect("test: GROUP BY AVG(score)");

    assert_eq!(results.len(), 3, "Expected 3 groups");

    // doc-1 should have the highest average (vectors closest to query).
    let doc1 = results
        .iter()
        .find(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("parent_id"))
                .and_then(|v| v.as_str())
                == Some("doc-1")
        })
        .expect("doc-1 group");

    // doc-1 has 3 chunks; the AVG score should be > 0 (exact value depends on cosine).
    assert!(doc1.score > 0.0, "doc-1 avg score should be positive");
}

// =========================================================================
// Scenario 3: Nominal — FIRST(text) returns text from highest-scoring chunk
// =========================================================================

#[test]
fn test_given_chunked_collection_when_first_text_then_best_chunk_excerpt() {
    let (_dir, db) = create_test_db();
    setup_chunked_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT parent_id, MAX(score) AS relevance, FIRST(text) AS excerpt FROM chunks WHERE vector NEAR $v GROUP BY parent_id LIMIT 10",
        &params,
    )
    .expect("test: FIRST(text)");

    assert_eq!(results.len(), 3);

    // doc-1's best chunk (id=1, vector=[1,0,0,0]) has text "doc1 chunk high".
    let doc1 = results
        .iter()
        .find(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("parent_id"))
                .and_then(|v| v.as_str())
                == Some("doc-1")
        })
        .expect("doc-1 group");

    let excerpt = doc1
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("excerpt"))
        .and_then(|v| v.as_str());
    assert_eq!(
        excerpt,
        Some("doc1 chunk high"),
        "FIRST(text) should return text from highest-scoring chunk"
    );
}

// =========================================================================
// Scenario 4: Nominal — GROUP BY + ORDER BY DESC + LIMIT 2
// =========================================================================

#[test]
fn test_given_chunked_collection_when_group_by_order_by_limit_then_top_n_parents() {
    let (_dir, db) = create_test_db();
    setup_chunked_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT parent_id, MAX(score) AS relevance FROM chunks WHERE vector NEAR $v GROUP BY parent_id ORDER BY relevance DESC LIMIT 2",
        &params,
    )
    .expect("test: GROUP BY + ORDER BY + LIMIT");

    assert_eq!(results.len(), 2, "LIMIT 2 should return 2 groups");

    // Results should be in descending score order.
    assert!(
        results[0].score >= results[1].score,
        "Results should be in descending order: {} >= {}",
        results[0].score,
        results[1].score
    );

    // doc-1 should be first (closest to query vector).
    let first_parent = results[0]
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("parent_id"))
        .and_then(|v| v.as_str());
    assert_eq!(first_parent, Some("doc-1"), "doc-1 should be most relevant");
}

// =========================================================================
// Scenario 5: Edge — Single chunk per parent (passthrough)
// =========================================================================

#[test]
fn test_given_single_chunk_per_parent_when_group_by_then_passthrough() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION singles (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create singles");

    let vc = db
        .get_vector_collection("singles")
        .expect("test: get singles");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"parent_id":"A","text":"only"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"parent_id":"B","text":"only"})),
        ),
    ])
    .expect("test: upsert singles");

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT parent_id, MAX(score) AS relevance FROM singles WHERE vector NEAR $v GROUP BY parent_id LIMIT 10",
        &params,
    )
    .expect("test: single chunk passthrough");

    assert_eq!(results.len(), 2, "Should have 2 groups (one per parent)");
}

// =========================================================================
// Scenario 6: Edge — All chunks same parent → single result
// =========================================================================

#[test]
fn test_given_all_chunks_same_parent_when_group_by_then_single_result() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION same_parent (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create same_parent");

    let vc = db
        .get_vector_collection("same_parent")
        .expect("test: get same_parent");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"parent_id":"X"})),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(serde_json::json!({"parent_id":"X"})),
        ),
        Point::new(
            3,
            vec![0.8, 0.2, 0.0, 0.0],
            Some(serde_json::json!({"parent_id":"X"})),
        ),
    ])
    .expect("test: upsert same_parent");

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT parent_id, MAX(score) AS relevance FROM same_parent WHERE vector NEAR $v GROUP BY parent_id LIMIT 10",
        &params,
    )
    .expect("test: all same parent");

    assert_eq!(results.len(), 1, "All chunks same parent → 1 group");
}

// =========================================================================
// Scenario 7: Edge — Empty NEAR results → empty
// =========================================================================

#[test]
fn test_given_empty_near_results_when_group_by_then_empty() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION empty_col (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create empty_col");

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT parent_id, MAX(score) AS relevance FROM empty_col WHERE vector NEAR $v GROUP BY parent_id LIMIT 10",
        &params,
    )
    .expect("test: empty collection");

    assert!(results.is_empty(), "Empty collection → empty results");
}

// =========================================================================
// Scenario 8: Negative — FIRST without GROUP BY → error
// =========================================================================

#[test]
fn test_given_first_without_group_by_when_execute_then_error() {
    let (_dir, db) = create_test_db();
    setup_chunked_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let result = execute_sql_with_params(
        &db,
        "SELECT FIRST(text) AS excerpt FROM chunks WHERE vector NEAR $v LIMIT 10",
        &params,
    );

    assert!(result.is_err(), "FIRST without GROUP BY should error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("FIRST()") && err_msg.contains("GROUP BY"),
        "Error should mention FIRST and GROUP BY: {err_msg}"
    );
}

// =========================================================================
// Scenario 9: Negative — MAX(score) without NEAR → error
// =========================================================================

#[test]
fn test_given_max_score_without_near_when_execute_then_error() {
    let (_dir, db) = create_test_db();
    setup_chunked_collection(&db);

    let result = execute_sql(
        &db,
        "SELECT parent_id, MAX(score) AS relevance FROM chunks GROUP BY parent_id LIMIT 10",
    );

    assert!(result.is_err(), "MAX(score) without NEAR should error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("score") && err_msg.contains("NEAR"),
        "Error should mention score and NEAR: {err_msg}"
    );
}

// =========================================================================
// Scenario 10: Negative — GROUP BY on nonexistent field → empty
// =========================================================================

#[test]
fn test_given_nonexistent_group_field_when_group_by_then_empty() {
    let (_dir, db) = create_test_db();
    setup_chunked_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT nonexistent_field, MAX(score) AS relevance FROM chunks WHERE vector NEAR $v GROUP BY nonexistent_field LIMIT 10",
        &params,
    )
    .expect("test: nonexistent group field");

    assert!(
        results.is_empty(),
        "GROUP BY on nonexistent field → empty result set"
    );
}

// =========================================================================
// Scenario 11: Combination — GROUP BY + metadata filter + NEAR
// =========================================================================

#[test]
fn test_given_group_by_with_metadata_filter_when_execute_then_filtered_groups() {
    let (_dir, db) = create_test_db();
    setup_chunked_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT parent_id, MAX(score) AS relevance FROM chunks WHERE vector NEAR $v AND category = 'science' GROUP BY parent_id LIMIT 10",
        &params,
    )
    .expect("test: GROUP BY + metadata filter");

    // Only doc-1 and doc-3 have category='science'.
    assert_eq!(
        results.len(),
        2,
        "Only science parents should appear, got {}",
        results.len()
    );

    let parent_ids: std::collections::HashSet<String> = results
        .iter()
        .filter_map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("parent_id"))
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();
    assert!(parent_ids.contains("doc-1"), "doc-1 is science");
    assert!(parent_ids.contains("doc-3"), "doc-3 is science");
    assert!(
        !parent_ids.contains("doc-2"),
        "doc-2 is tech, should be filtered"
    );
}
