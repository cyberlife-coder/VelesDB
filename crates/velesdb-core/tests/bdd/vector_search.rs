//! BDD-style end-to-end tests for `VelesQL` vector search behaviors.
//!
//! These tests verify actual runtime behavior of NEAR, similarity scoring,
//! WITH clause, and filter integration from the user's perspective.
//!
//! Each scenario follows Given-When-Then structure:
//! - **Given**: a collection with known, deterministic vectors
//! - **When**: a `VelesQL` query is executed through the full pipeline
//! - **Then**: results match expected behavior (ordering, filtering, scores)

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{
    approx_eq, create_test_db, execute_sql, execute_sql_with_params, vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate a "docs" collection with known vectors for predictable search.
///
/// Three orthogonal clusters plus an outlier:
/// - Science (ids 1-3): vectors near `[1,0,0,0]`
/// - Tech (ids 4-5): vectors near `[0,1,0,0]`
/// - History (ids 6-7): vectors near `[0,0,1,0]`
/// - Misc outlier (id 8): uniform `[0.25, 0.25, 0.25, 0.25]`
fn setup_search_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE docs");

    let vc = db
        .get_vector_collection("docs")
        .expect("test: docs collection must exist");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"title":"Physics 101","category":"science","year":2024})),
        ),
        Point::new(
            2,
            vec![0.95, 0.05, 0.0, 0.0],
            Some(json!({"title":"Chemistry Basics","category":"science","year":2023})),
        ),
        Point::new(
            3,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(json!({"title":"Biology Today","category":"science","year":2024})),
        ),
        Point::new(
            4,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"title":"Rust Handbook","category":"tech","year":2024})),
        ),
        Point::new(
            5,
            vec![0.05, 0.95, 0.0, 0.0],
            Some(json!({"title":"Python Guide","category":"tech","year":2023})),
        ),
        Point::new(
            6,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"title":"World History","category":"history","year":2022})),
        ),
        Point::new(
            7,
            vec![0.0, 0.0, 0.9, 0.1],
            Some(json!({"title":"Ancient Rome","category":"history","year":2021})),
        ),
        Point::new(
            8,
            vec![0.25, 0.25, 0.25, 0.25],
            Some(json!({"title":"General Knowledge","category":"misc","year":2024})),
        ),
    ])
    .expect("test: upsert search corpus");
}

// =========================================================================
// Scenario 1: NEAR returns closest vectors first
// =========================================================================

/// GIVEN a search collection with clustered vectors
/// WHEN querying with vector `[1,0,0,0]` (exact match for science cluster)
/// THEN the top-3 results are all from the science cluster, with id 1 first.
#[test]
fn test_near_returns_closest_vectors_first() {
    let (_dir, db) = create_test_db();
    setup_search_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 3";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR search");

    assert_eq!(results.len(), 3, "LIMIT 3 must return exactly 3 results");
    assert_eq!(
        results[0].point.id, 1,
        "Exact match (id 1) must be the first result"
    );

    let science_ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        science_ids.contains(&1) && science_ids.contains(&2) && science_ids.contains(&3),
        "Top-3 for [1,0,0,0] must be all science cluster ids (1,2,3), got {:?}",
        science_ids
    );
}

// =========================================================================
// Scenario 2: NEAR with LIMIT respects count
// =========================================================================

/// GIVEN a search collection
/// WHEN querying with vector `[0,1,0,0]` and LIMIT 2
/// THEN exactly 2 results are returned, both from the tech cluster.
#[test]
fn test_near_with_limit_respects_count() {
    let (_dir, db) = create_test_db();
    setup_search_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 2";
    let params = vector_param(&[0.0, 1.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR LIMIT 2");

    assert_eq!(results.len(), 2, "LIMIT 2 must return exactly 2 results");

    let tech_ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        tech_ids.contains(&4) && tech_ids.contains(&5),
        "Top-2 for [0,1,0,0] must be tech cluster ids (4,5), got {:?}",
        tech_ids
    );
}

// =========================================================================
// Scenario 3: NEAR + category filter narrows results
// =========================================================================

/// GIVEN a search collection
/// WHEN querying with vector `[1,0,0,0]` (closest to science) but filtering
///      `category = 'tech'`
/// THEN only tech docs (ids 4,5) are returned, not science.
#[test]
fn test_near_with_category_filter_narrows_results() {
    let (_dir, db) = create_test_db();
    setup_search_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 5";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR + filter");

    assert!(
        !results.is_empty(),
        "NEAR + category filter must return at least one result"
    );

    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(
            cat,
            Some("tech"),
            "All results must have category='tech', got {:?} for id={}",
            cat,
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 4: NEAR results have non-zero similarity scores
// =========================================================================

/// GIVEN a search collection
/// WHEN any NEAR query is executed
/// THEN all returned results have `score > 0.0`.
#[test]
fn test_near_results_have_nonzero_scores() {
    let (_dir, db) = create_test_db();
    setup_search_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR scores");

    assert!(!results.is_empty(), "NEAR must return at least one result");
    for r in &results {
        assert!(
            r.score > 0.0,
            "Score must be positive, got {} for id={}",
            r.score,
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 5: NEAR results are ordered by decreasing similarity
// =========================================================================

/// GIVEN a search collection
/// WHEN querying with LIMIT 5
/// THEN results are in non-increasing score order.
#[test]
fn test_near_results_ordered_by_decreasing_similarity() {
    let (_dir, db) = create_test_db();
    setup_search_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: score ordering");

    assert!(
        results.len() >= 2,
        "Need at least 2 results to verify ordering"
    );
    for i in 0..results.len().saturating_sub(1) {
        assert!(
            results[i].score >= results[i + 1].score,
            "Scores must be non-increasing: results[{}].score={} < results[{}].score={}",
            i,
            results[i].score,
            i + 1,
            results[i + 1].score
        );
    }
}

// =========================================================================
// Scenario 6: WITH mode='fast' still returns results
// =========================================================================

/// GIVEN a search collection
/// WHEN using `WITH (mode = 'fast')` hint
/// THEN the engine still returns at least one result (lower quality accepted).
#[test]
fn test_with_mode_fast_still_returns_results() {
    let (_dir, db) = create_test_db();
    setup_search_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 3 WITH (mode = 'fast')";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: WITH fast");

    assert_eq!(
        results.len(),
        3,
        "LIMIT 3 with mode='fast' must return exactly 3 results"
    );
    assert_eq!(
        results[0].point.id, 1,
        "mode='fast' must still rank the exact-match vector (id 1) first"
    );
}

// =========================================================================
// Scenario 7: Similarity score for exact match is approximately 1.0
// =========================================================================

/// GIVEN a search collection with cosine metric
/// WHEN querying with vector `[1,0,0,0]` (identical to id 1)
/// THEN the top result has a similarity score close to 1.0.
#[test]
fn test_exact_match_score_approximately_one() {
    let (_dir, db) = create_test_db();
    setup_search_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 1";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: exact match score");

    assert_eq!(results.len(), 1, "LIMIT 1 must return exactly 1 result");
    assert_eq!(
        results[0].point.id, 1,
        "Exact match vector must return id 1"
    );
    assert!(
        approx_eq(results[0].score, 1.0, 0.01),
        "Cosine similarity for exact match must be ~1.0, got {}",
        results[0].score
    );
}

// =========================================================================
// Scenario 8: Query vector far from all data returns low scores
// =========================================================================

/// GIVEN a search collection
/// WHEN querying with `[0,0,0,1]` (orthogonal to most data)
/// THEN the top score is lower than an exact-match query's top score.
#[test]
fn test_orthogonal_query_returns_lower_scores() {
    let (_dir, db) = create_test_db();
    setup_search_collection(&db);

    let exact_sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 1";
    let exact_params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let exact_results =
        execute_sql_with_params(&db, exact_sql, &exact_params).expect("test: exact query");

    let ortho_sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 1";
    let ortho_params = vector_param(&[0.0, 0.0, 0.0, 1.0]);
    let ortho_results =
        execute_sql_with_params(&db, ortho_sql, &ortho_params).expect("test: orthogonal query");

    assert!(!exact_results.is_empty(), "Exact query must return results");
    assert!(
        !ortho_results.is_empty(),
        "Orthogonal query must return results"
    );
    assert!(
        ortho_results[0].score < exact_results[0].score,
        "Orthogonal query score ({}) must be lower than exact match score ({})",
        ortho_results[0].score,
        exact_results[0].score
    );
}

// =========================================================================
// Scenario 9: NEAR with year filter
// =========================================================================

/// GIVEN a search collection with year metadata
/// WHEN querying with `[1,0,0,0]` and `year = 2024` filter
/// THEN all returned results have year 2024 in their payload.
#[test]
fn test_near_with_year_filter() {
    let (_dir, db) = create_test_db();
    setup_search_collection(&db);

    let sql = "SELECT * FROM docs WHERE vector NEAR $v AND year = 2024 LIMIT 5";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR + year filter");

    assert!(
        !results.is_empty(),
        "NEAR + year filter must return at least one result"
    );
    for r in &results {
        let year = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("year"))
            .and_then(serde_json::Value::as_i64);
        assert_eq!(
            year,
            Some(2024),
            "All results must have year=2024, got {:?} for id={}",
            year,
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 10: Empty collection returns no results
// =========================================================================

/// GIVEN an empty vector collection
/// WHEN a NEAR query is executed
/// THEN zero results are returned (no error).
#[test]
fn test_empty_collection_returns_no_results() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION empty_coll (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE empty_coll");

    let sql = "SELECT * FROM empty_coll WHERE vector NEAR $v LIMIT 10";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results =
        execute_sql_with_params(&db, sql, &params).expect("test: search empty collection");

    assert_eq!(
        results.len(),
        0,
        "Empty collection must return 0 results, got {}",
        results.len()
    );
}
