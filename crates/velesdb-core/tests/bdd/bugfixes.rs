//! BDD tests for specific bug fixes.
//!
//! Each test documents the original bug, provides a minimal reproduction,
//! and proves the fix holds.

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, payload_str, vector_param,
};
use serde_json::json;
use velesdb_core::Point;

// =========================================================================
// Helpers
// =========================================================================

/// Creates a vector collection `docs` with duplicate categories for DISTINCT tests.
fn setup_docs_with_duplicate_categories(db: &velesdb_core::Database) {
    execute_sql(
        db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE docs");

    let coll = db.get_vector_collection("docs").expect("test: get docs");
    coll.upsert(vec![
        Point::new(
            1,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(json!({"category": "tech", "content": "database systems"})),
        ),
        Point::new(
            2,
            vec![0.8, 0.2, 0.0, 0.0],
            Some(json!({"category": "tech", "content": "database indexing"})),
        ),
        Point::new(
            3,
            vec![0.1, 0.9, 0.0, 0.0],
            Some(json!({"category": "science", "content": "quantum physics"})),
        ),
        Point::new(
            4,
            vec![0.0, 0.1, 0.9, 0.0],
            Some(json!({"category": "tech", "content": "machine learning"})),
        ),
        Point::new(
            5,
            vec![0.0, 0.0, 0.1, 0.9],
            Some(json!({"category": "science", "content": "biology research"})),
        ),
    ])
    .expect("test: upsert docs");
}

// =========================================================================
// Bug #475: early-return query paths skip DISTINCT
// =========================================================================

/// GIVEN a collection with duplicate category values
/// WHEN `SELECT DISTINCT category FROM docs WHERE NOT similarity(vector, $v) > 0.8 LIMIT 10`
/// THEN the result should not contain duplicate categories.
///
/// Bug: `execute_early_return_query` applied ORDER BY, OFFSET, LIMIT but NOT DISTINCT.
/// The `finalize_query_results` path correctly applied DISTINCT via `apply_select_postprocessing`.
#[test]
fn test_bug_475_early_return_applies_distinct() {
    let (_dir, db) = create_test_db();
    setup_docs_with_duplicate_categories(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);

    let sql = "SELECT DISTINCT category FROM docs \
               WHERE NOT similarity(vector, $v) > 0.8 \
               LIMIT 10";

    let results =
        execute_sql_with_params(&db, sql, &params).expect("DISTINCT + NOT similarity should work");

    // Collect all category values from results
    let categories: Vec<&str> = results
        .iter()
        .filter_map(|r| payload_str(r, "category"))
        .collect();

    // There should be no duplicate categories
    let mut unique = categories.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        categories.len(),
        unique.len(),
        "Bug #475: DISTINCT should remove duplicate categories in early-return path. \
         Got duplicates: {categories:?}"
    );
}

// =========================================================================
// Bug #474: NEAR + text MATCH + metadata filter silently drops metadata filter
// =========================================================================

/// GIVEN a collection with documents having different categories
/// WHEN `SELECT * FROM docs WHERE vector NEAR $v AND content MATCH 'database' AND category = 'tech' LIMIT 10`
/// THEN only results where category = 'tech' should be returned.
///
/// Bug: `dispatch_near_with_filter` called `hybrid_search` when it found a MATCH clause,
/// but silently dropped co-occurring metadata filters like `category = 'tech'`.
#[test]
fn test_bug_474_near_match_metadata_filter_not_dropped() {
    let (_dir, db) = create_test_db();
    setup_docs_with_duplicate_categories(&db);

    let params = vector_param(&[0.5, 0.5, 0.0, 0.0]);

    let sql = "SELECT * FROM docs \
               WHERE vector NEAR $v AND content MATCH 'database' AND category = 'tech' \
               LIMIT 10";

    let results = execute_sql_with_params(&db, sql, &params)
        .expect("NEAR + MATCH + metadata filter should work");

    // Every result must have category = 'tech'
    for result in &results {
        let cat = payload_str(result, "category");
        assert_eq!(
            cat,
            Some("tech"),
            "Bug #474: metadata filter 'category = tech' was silently dropped. \
             Got category={cat:?} for point id={}",
            result.point.id
        );
    }
}
