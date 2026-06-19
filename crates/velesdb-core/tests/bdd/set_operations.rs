//! BDD-style end-to-end tests for `VelesQL` set operations, TRAIN QUANTIZER,
//! and LET bindings.
//!
//! These tests verify actual runtime behavior through the full pipeline:
//! SQL string -> `Parser::parse()` -> `Database::execute_query()` -> verify results.
//!
//! Each scenario follows Given-When-Then structure:
//! - **Given**: collections with known, deterministic data
//! - **When**: a `VelesQL` query is executed through the full pipeline
//! - **Then**: results match expected behavior

use serde_json::json;
use std::collections::HashSet;
use velesdb_core::{Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, result_ids, vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate two overlapping collections for set operation tests.
///
/// - Collection `alpha`: ids 1, 2, 3 with `category='tech'`
/// - Collection `beta`:  ids 3, 4, 5 with `category='science'`
///
/// Id 3 exists in both collections (overlapping point).
fn setup_two_collections(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION alpha (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE alpha");
    execute_sql(
        db,
        "CREATE COLLECTION beta (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE beta");

    let alpha = db
        .get_vector_collection("alpha")
        .expect("test: alpha must exist");
    alpha
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"category": "tech"})),
            ),
            Point::new(
                2,
                vec![0.9, 0.1, 0.0, 0.0],
                Some(json!({"category": "tech"})),
            ),
            Point::new(
                3,
                vec![0.5, 0.5, 0.0, 0.0],
                Some(json!({"category": "tech"})),
            ),
        ])
        .expect("test: upsert alpha");

    let beta = db
        .get_vector_collection("beta")
        .expect("test: beta must exist");
    beta.upsert(vec![
        Point::new(
            3,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({"category": "science"})),
        ),
        Point::new(
            4,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"category": "science"})),
        ),
        Point::new(
            5,
            vec![0.0, 0.9, 0.1, 0.0],
            Some(json!({"category": "science"})),
        ),
    ])
    .expect("test: upsert beta");
}

/// Populate a single `items` collection with two categories for same-collection
/// set operations.
///
/// - Ids 1-3: `category='A'`, vectors near `[1,0,0,0]`
/// - Ids 4-6: `category='B'`, vectors near `[0,1,0,0]`
fn setup_items_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION items (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE items");

    let items = db
        .get_vector_collection("items")
        .expect("test: items must exist");
    items
        .upsert(vec![
            Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"category": "A"}))),
            Point::new(
                2,
                vec![0.95, 0.05, 0.0, 0.0],
                Some(json!({"category": "A"})),
            ),
            Point::new(3, vec![0.9, 0.1, 0.0, 0.0], Some(json!({"category": "A"}))),
            Point::new(4, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"category": "B"}))),
            Point::new(
                5,
                vec![0.0, 0.95, 0.05, 0.0],
                Some(json!({"category": "B"})),
            ),
            Point::new(6, vec![0.0, 0.9, 0.1, 0.0], Some(json!({"category": "B"}))),
        ])
        .expect("test: upsert items");
}

/// Populate a collection with enough vectors for TRAIN QUANTIZER tests.
fn setup_large_collection(db: &Database, name: &str, count: usize) {
    execute_sql(
        db,
        &format!("CREATE COLLECTION {name} (dimension = 4, metric = 'cosine')"),
    )
    .expect("test: CREATE large collection");

    let vc = db
        .get_vector_collection(name)
        .expect("test: large collection must exist");

    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        #[allow(clippy::cast_precision_loss)]
        let fc = count as f32;
        let v = vec![fi / fc, 1.0 - fi / fc, 0.5, 0.3];
        points.push(Point::new(
            u64::try_from(i).expect("test: i fits u64"),
            v,
            Some(json!({"idx": i})),
        ));
    }
    vc.upsert(points).expect("test: upsert large collection");
}

/// Populate a "docs" collection for LET binding tests.
///
/// 20 points with vectors spread along a gradient for predictable similarity ordering.
fn setup_let_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE docs");

    let vc = db
        .get_vector_collection("docs")
        .expect("test: docs must exist");

    let mut points = Vec::with_capacity(20);
    for i in 0u64..20 {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        let v = vec![fi / 20.0, 1.0 - fi / 20.0, 0.5, 0.3];
        points.push(Point::new(
            i,
            v,
            Some(json!({"idx": i, "priority": 20 - i})),
        ));
    }
    vc.upsert(points).expect("test: upsert docs");
}

// =========================================================================
// UNION: combines results from two queries
// =========================================================================

/// GIVEN two collections `alpha` (ids 1,2,3) and `beta` (ids 3,4,5)
/// WHEN `SELECT * FROM alpha LIMIT 10 UNION SELECT * FROM beta LIMIT 10`
/// THEN returns deduplicated combined results (id 3 appears once).
#[test]
fn test_union_combines_results_from_two_queries() {
    let (_dir, db) = create_test_db();
    setup_two_collections(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM alpha LIMIT 10 UNION SELECT * FROM beta LIMIT 10",
    )
    .expect("UNION should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 5, "UNION should deduplicate: got {ids:?}");
    assert!(ids.contains(&1), "should contain id 1 from alpha");
    assert!(ids.contains(&3), "overlapping id 3 should appear once");
    assert!(ids.contains(&5), "should contain id 5 from beta");
}

// =========================================================================
// UNION ALL: keeps duplicates
// =========================================================================

/// GIVEN two collections `alpha` (ids 1,2,3) and `beta` (ids 3,4,5)
/// WHEN `UNION ALL`
/// THEN returns all rows including duplicate id 3 (total 6 rows).
#[test]
fn test_union_all_keeps_duplicates() {
    let (_dir, db) = create_test_db();
    setup_two_collections(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM alpha LIMIT 10 UNION ALL SELECT * FROM beta LIMIT 10",
    )
    .expect("UNION ALL should succeed");

    assert_eq!(
        results.len(),
        6,
        "UNION ALL should keep all rows including duplicates"
    );

    let id3_count = results.iter().filter(|r| r.point.id == 3).count();
    assert_eq!(id3_count, 2, "id 3 should appear twice in UNION ALL");
}

// =========================================================================
// INTERSECT: returns common rows
// =========================================================================

/// GIVEN two collections `alpha` (ids 1,2,3) and `beta` (ids 3,4,5)
/// WHEN `INTERSECT`
/// THEN returns only the overlapping id 3.
#[test]
fn test_intersect_returns_common_rows() {
    let (_dir, db) = create_test_db();
    setup_two_collections(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM alpha LIMIT 10 INTERSECT SELECT * FROM beta LIMIT 10",
    )
    .expect("INTERSECT should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 1, "INTERSECT should return only shared ids");
    assert!(ids.contains(&3), "id 3 is the only overlapping point");
}

// =========================================================================
// EXCEPT: removes second set from first
// =========================================================================

/// GIVEN two collections `alpha` (ids 1,2,3) and `beta` (ids 3,4,5)
/// WHEN `EXCEPT`
/// THEN returns ids from alpha that are NOT in beta (ids 1, 2).
#[test]
fn test_except_removes_second_set() {
    let (_dir, db) = create_test_db();
    setup_two_collections(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM alpha LIMIT 10 EXCEPT SELECT * FROM beta LIMIT 10",
    )
    .expect("EXCEPT should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 2, "EXCEPT should return alpha - beta");
    assert!(ids.contains(&1), "id 1 only in alpha");
    assert!(ids.contains(&2), "id 2 only in alpha");
    assert!(!ids.contains(&3), "id 3 should be excluded (in beta)");
}

// =========================================================================
// Same-collection UNION with different WHERE clauses
// =========================================================================

/// GIVEN a single `items` collection with categories A and B
/// WHEN UNION across two WHERE clauses on the same collection
/// THEN returns combined results from both categories.
#[test]
fn test_union_same_collection_different_where() {
    let (_dir, db) = create_test_db();
    setup_items_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM items WHERE category = 'A' LIMIT 10 \
         UNION SELECT * FROM items WHERE category = 'B' LIMIT 10",
    )
    .expect("same-collection UNION should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 6, "should combine both categories: got {ids:?}");
}

// =========================================================================
// TRAIN QUANTIZER: success on collection with enough vectors
// =========================================================================

/// GIVEN a collection with 200 vectors (dimension=4)
/// WHEN `TRAIN QUANTIZER ON trainable WITH (m = 2, k = 16)`
/// THEN succeeds without error and returns a training result.
#[test]
fn test_train_quantizer_on_collection() {
    let (_dir, db) = create_test_db();
    setup_large_collection(&db, "trainable", 200);

    let results = execute_sql(&db, "TRAIN QUANTIZER ON trainable WITH (m = 2, k = 16)")
        .expect("TRAIN QUANTIZER should succeed on 200 vectors");

    assert_eq!(results.len(), 1, "should return one training result");
    let payload = results[0]
        .point
        .payload
        .as_ref()
        .expect("training result should have payload");
    assert_eq!(
        payload.get("status").and_then(|v| v.as_str()),
        Some("trained"),
        "status should be 'trained'"
    );
}

// =========================================================================
// TRAIN QUANTIZER: empty collection returns clear error
// =========================================================================

/// GIVEN an empty collection
/// WHEN TRAIN QUANTIZER is executed
/// THEN returns an error mentioning no vectors available.
#[test]
fn test_train_quantizer_on_empty_collection() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION empty_col (dimension = 4, metric = 'cosine')",
    )
    .expect("test: CREATE empty_col");

    let result = execute_sql(&db, "TRAIN QUANTIZER ON empty_col WITH (m = 2, k = 16)");

    assert!(result.is_err(), "TRAIN on empty collection should fail");
    let msg = result.expect_err("expected error").to_string();
    assert!(
        msg.contains("no vectors"),
        "error should mention 'no vectors', got: {msg}"
    );
}

// =========================================================================
// TRAIN QUANTIZER: nonexistent collection returns clear error
// =========================================================================

/// WHEN TRAIN QUANTIZER targets a collection that does not exist
/// THEN returns an error mentioning the collection name.
#[test]
fn test_train_quantizer_on_nonexistent_collection() {
    let (_dir, db) = create_test_db();

    let result = execute_sql(
        &db,
        "TRAIN QUANTIZER ON does_not_exist WITH (m = 2, k = 16)",
    );

    assert!(
        result.is_err(),
        "TRAIN on nonexistent collection should fail"
    );
    let msg = result.expect_err("expected error").to_string();
    assert!(
        msg.contains("does_not_exist"),
        "error should mention collection name, got: {msg}"
    );
}

// =========================================================================
// LET binding in ORDER BY
// =========================================================================

/// GIVEN a `docs` collection with 20 vectors
/// WHEN `LET score = similarity() SELECT ... ORDER BY score DESC LIMIT 5`
/// THEN results are ordered by similarity descending (same as bare `similarity()`).
#[test]
fn test_let_binding_in_order_by() {
    let (_dir, db) = create_test_db();
    setup_let_collection(&db);

    let params = vector_param(&[0.5, 0.5, 0.5, 0.3]);

    let baseline = execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v ORDER BY similarity() DESC LIMIT 5",
        &params,
    )
    .expect("baseline query");

    let let_results = execute_sql_with_params(
        &db,
        "LET score = similarity() SELECT * FROM docs WHERE vector NEAR $v ORDER BY score DESC LIMIT 5",
        &params,
    )
    .expect("LET query");

    assert_eq!(baseline.len(), let_results.len());
    let baseline_ids: Vec<u64> = baseline.iter().map(|r| r.point.id).collect();
    let let_ids: Vec<u64> = let_results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        baseline_ids, let_ids,
        "LET score = similarity() ORDER BY score should match ORDER BY similarity()"
    );
}

// =========================================================================
// LET binding with arithmetic (monotonic transform)
// =========================================================================

/// GIVEN a `docs` collection
/// WHEN `LET boosted = similarity() * 2.0 ... ORDER BY boosted DESC LIMIT 5`
/// THEN results maintain the same ordering as plain similarity (monotonic transform).
#[test]
fn test_let_binding_arithmetic() {
    let (_dir, db) = create_test_db();
    setup_let_collection(&db);

    let params = vector_param(&[0.5, 0.5, 0.5, 0.3]);

    let baseline = execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v ORDER BY similarity() DESC LIMIT 5",
        &params,
    )
    .expect("baseline query");

    let boosted = execute_sql_with_params(
        &db,
        "LET boosted = similarity() * 2.0 \
         SELECT * FROM docs WHERE vector NEAR $v ORDER BY boosted DESC LIMIT 5",
        &params,
    )
    .expect("boosted LET query");

    let baseline_ids: Vec<u64> = baseline.iter().map(|r| r.point.id).collect();
    let boosted_ids: Vec<u64> = boosted.iter().map(|r| r.point.id).collect();
    assert_eq!(
        baseline_ids, boosted_ids,
        "similarity() * 2.0 is monotonic, ordering should be identical"
    );
}

// =========================================================================
// UNION with impossible filter returns partial results
// =========================================================================

/// GIVEN `items` collection with categories A and B
/// WHEN first SELECT matches category A, second matches nonexistent category 'Z'
/// THEN UNION returns only the first set (partial results, not an error).
#[test]
fn test_union_with_impossible_filter_returns_partial() {
    let (_dir, db) = create_test_db();
    setup_items_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM items WHERE category = 'A' LIMIT 10 \
         UNION SELECT * FROM items WHERE category = 'Z' LIMIT 10",
    )
    .expect("UNION with impossible filter should succeed");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([1u64, 2, 3]),
        "impossible second branch contributes nothing; UNION must return exactly the 3 category-A ids, got {ids:?}"
    );
}

// =========================================================================
// Set operation on nonexistent collection fails with clear error
// =========================================================================

/// WHEN UNION references a collection that does not exist
/// THEN returns an error mentioning the missing collection.
#[test]
fn test_set_operation_on_nonexistent_collection_fails() {
    let (_dir, db) = create_test_db();
    setup_items_collection(&db);

    let result = execute_sql(
        &db,
        "SELECT * FROM items LIMIT 10 UNION SELECT * FROM ghost_collection LIMIT 10",
    );

    assert!(
        result.is_err(),
        "UNION with nonexistent collection should fail"
    );
    let msg = result.expect_err("expected error").to_string();
    assert!(
        msg.contains("ghost_collection"),
        "error should mention missing collection name, got: {msg}"
    );
}
