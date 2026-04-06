//! BDD-style end-to-end tests for secondary index bitmap IN filtering (Issue #512).
//!
//! Each scenario follows GIVEN (setup data) → WHEN (execute SQL) → THEN (verify
//! results). Tests exercise the full pipeline: SQL string → `Parser::parse()`
//! → `Database::execute_query()` → verify returned `SearchResult` values.
//!
//! The collection has secondary indexes on `category` and `price`, so IN
//! conditions on those fields use bitmap pre-filtering instead of post-filtering.

use std::collections::HashSet;

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, result_ids, vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate an `articles` collection with secondary indexes on `category` and
/// `price`, plus diverse test data for IN bitmap filtering.
///
/// | id | category | price | status  |
/// |----|----------|-------|---------|
/// | 1  | tech     | 120.0 | active  |
/// | 2  | science  | 80.0  | active  |
/// | 3  | tech     | 45.0  | draft   |
/// | 4  | art      | 200.0 | active  |
/// | 5  | science  | 60.0  | deleted |
/// | 6  | history  | 30.0  | active  |
/// | 7  | tech     | 150.0 | deleted |
fn setup_indexed_articles(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION articles (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE articles");

    execute_sql(db, "CREATE INDEX ON articles (category);").expect("test: CREATE INDEX category");
    execute_sql(db, "CREATE INDEX ON articles (price);").expect("test: CREATE INDEX price");

    let vc = db
        .get_vector_collection("articles")
        .expect("test: get articles");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"category": "tech",    "price": 120.0, "status": "active"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"category": "science", "price": 80.0,  "status": "active"})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"category": "tech",    "price": 45.0,  "status": "draft"})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"category": "art",     "price": 200.0, "status": "active"})),
        ),
        Point::new(
            5,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({"category": "science", "price": 60.0,  "status": "deleted"})),
        ),
        Point::new(
            6,
            vec![0.5, 0.0, 0.5, 0.0],
            Some(json!({"category": "history", "price": 30.0,  "status": "active"})),
        ),
        Point::new(
            7,
            vec![0.0, 0.5, 0.0, 0.5],
            Some(json!({"category": "tech",    "price": 150.0, "status": "deleted"})),
        ),
    ])
    .expect("test: upsert articles");
}

// =========================================================================
// Nominal: IN on indexed string field
// =========================================================================

/// GIVEN articles with secondary index on `category`
/// WHEN querying WHERE category IN ('tech', 'science')
/// THEN returns articles 1, 2, 3, 5, 7 (all tech + science)
#[test]
fn test_in_string_indexed_returns_correct_results() {
    let (_dir, db) = create_test_db();
    setup_indexed_articles(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM articles WHERE category IN ('tech', 'science') LIMIT 20;",
    )
    .expect("test: IN string indexed query");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([1, 2, 3, 5, 7]),
        "tech (1,3,7) + science (2,5)"
    );
}

// =========================================================================
// Nominal: IN on indexed numeric field
// =========================================================================

/// GIVEN articles with secondary index on `price`
/// WHEN querying WHERE price IN (120.0, 80.0, 30.0)
/// THEN returns articles 1 (120), 2 (80), 6 (30)
#[test]
fn test_in_int_indexed_returns_correct_results() {
    let (_dir, db) = create_test_db();
    setup_indexed_articles(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM articles WHERE price IN (120.0, 80.0, 30.0) LIMIT 20;",
    )
    .expect("test: IN numeric indexed query");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([1, 2, 6]),
        "price 120 (id=1), 80 (id=2), 30 (id=6)"
    );
}

// =========================================================================
// Nominal: IN AND range intersection
// =========================================================================

/// GIVEN articles with indexes on `category` and `price`
/// WHEN querying WHERE category IN ('tech', 'science') AND price > 50
/// THEN returns articles matching both: 1 (tech,120), 2 (science,80), 7 (tech,150)
#[test]
fn test_in_and_range_intersection() {
    let (_dir, db) = create_test_db();
    setup_indexed_articles(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM articles WHERE category IN ('tech', 'science') AND price > 50 LIMIT 20;",
    )
    .expect("test: IN AND range query");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([1, 2, 5, 7]),
        "tech/science with price > 50: 1(120), 2(80), 5(60), 7(150)"
    );
}

// =========================================================================
// Nominal: IN OR equality union
// =========================================================================

/// GIVEN articles with index on `category`
/// WHEN querying WHERE category IN ('tech', 'science') OR status = 'active'
/// THEN returns union: tech/science (1,2,3,5,7) ∪ active (1,2,4,6) = {1,2,3,4,5,6,7}
#[test]
fn test_in_or_eq_union() {
    let (_dir, db) = create_test_db();
    setup_indexed_articles(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM articles WHERE category IN ('tech', 'science') OR status = 'active' LIMIT 20;",
    )
    .expect("test: IN OR eq query");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([1, 2, 3, 4, 5, 6, 7]),
        "union of tech/science and active"
    );
}

// =========================================================================
// Nominal: NOT IN on indexed field
// =========================================================================

/// GIVEN articles with index on `category`
/// WHEN querying WHERE category NOT IN ('draft', 'deleted')
/// THEN returns all articles (no category equals 'draft' or 'deleted' — those
/// are status values, not category values)
#[test]
fn test_not_in_indexed_returns_correct_results() {
    let (_dir, db) = create_test_db();
    setup_indexed_articles(&db);

    // Exclude categories 'art' and 'history' to get tech + science
    let results = execute_sql(
        &db,
        "SELECT * FROM articles WHERE category NOT IN ('art', 'history') LIMIT 20;",
    )
    .expect("test: NOT IN indexed query");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([1, 2, 3, 5, 7]),
        "all except art(4) and history(6)"
    );
}

// =========================================================================
// Edge: IN with nonexistent value
// =========================================================================

/// GIVEN articles collection
/// WHEN querying WHERE category IN ('nonexistent')
/// THEN returns empty result set
#[test]
fn test_in_nonexistent_value_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_indexed_articles(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM articles WHERE category IN ('nonexistent') LIMIT 20;",
    )
    .expect("test: IN nonexistent value");

    assert!(
        results.is_empty(),
        "nonexistent category should match nothing"
    );
}

// =========================================================================
// Edge: IN on unindexed field falls back to post-filter
// =========================================================================

/// GIVEN articles with NO index on `status`
/// WHEN querying WHERE status IN ('active', 'draft')
/// THEN returns correct results via post-filter fallback
#[test]
fn test_in_unindexed_field_falls_back_to_postfilter() {
    let (_dir, db) = create_test_db();
    setup_indexed_articles(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM articles WHERE status IN ('active', 'draft') LIMIT 20;",
    )
    .expect("test: IN unindexed field");

    let ids = result_ids(&results);
    // active: 1,2,4,6  draft: 3
    assert_eq!(
        ids,
        HashSet::from([1, 2, 3, 4, 6]),
        "active (1,2,4,6) + draft (3)"
    );
}

// =========================================================================
// Edge: IN with large list (100+ values)
// =========================================================================

/// GIVEN articles collection
/// WHEN querying WHERE category IN (100+ values including 'tech')
/// THEN returns correct results (tech articles)
#[test]
fn test_in_large_list_produces_correct_bitmap() {
    let (_dir, db) = create_test_db();
    setup_indexed_articles(&db);

    // Build a large IN list: 'val_0', 'val_1', ..., 'val_99', 'tech'
    let mut values: Vec<String> = (0..100).map(|i| format!("'val_{i}'")).collect();
    values.push("'tech'".to_string());
    let in_list = values.join(", ");
    let sql = format!("SELECT * FROM articles WHERE category IN ({in_list}) LIMIT 20;");

    let results = execute_sql(&db, &sql).expect("test: IN large list");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([1, 3, 7]), "only tech articles match");
}

// =========================================================================
// Combination: IN with vector NEAR search
// =========================================================================

/// GIVEN articles with index on `category` and vectors
/// WHEN querying WHERE category IN ('tech', 'science') AND vector NEAR $v
/// THEN returns results filtered by IN bitmap pre-filter and ranked by similarity
#[test]
fn test_in_with_vector_near_uses_bitmap_prefilter() {
    let (_dir, db) = create_test_db();
    setup_indexed_articles(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM articles WHERE category IN ('tech', 'science') AND vector NEAR $v LIMIT 5;",
        &params,
    )
    .expect("test: IN + NEAR query");

    assert!(!results.is_empty(), "should return results");
    // All results must be tech or science
    let ids = result_ids(&results);
    let valid_ids = HashSet::from([1, 2, 3, 5, 7]);
    for id in &ids {
        assert!(
            valid_ids.contains(id),
            "result id {id} should be tech or science"
        );
    }
}

// =========================================================================
// Combination: IN in JOIN WHERE clause
// =========================================================================

/// GIVEN products (vector) JOIN inventory (metadata) with IN filter on joined table
/// WHEN querying WHERE inventory.product_id IN (1, 3)
/// THEN returns only matching joined rows
#[test]
fn test_in_join_where_uses_column_store_bitmap() {
    let (_dir, db) = create_test_db();

    // Setup products (vector collection)
    execute_sql(
        &db,
        "CREATE COLLECTION products (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE products");

    let products = db
        .get_vector_collection("products")
        .expect("test: get products");
    products
        .upsert(vec![
            Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"name": "Alpha"}))),
            Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"name": "Beta"}))),
            Point::new(3, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"name": "Gamma"}))),
        ])
        .expect("test: upsert products");

    // Setup inventory (metadata collection)
    execute_sql(&db, "CREATE METADATA COLLECTION inventory;").expect("test: CREATE inventory");

    let inventory = db
        .get_metadata_collection("inventory")
        .expect("test: get inventory");
    inventory
        .upsert(vec![
            Point::metadata_only(1, json!({"product_id": 1, "price": 100, "stock": 10})),
            Point::metadata_only(2, json!({"product_id": 2, "price": 200, "stock": 0})),
            Point::metadata_only(3, json!({"product_id": 3, "price": 50,  "stock": 5})),
        ])
        .expect("test: upsert inventory");

    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               WHERE inventory.price IN (100, 50) \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("test: IN in JOIN WHERE");

    let ids = result_ids(&results);
    // price 100 → inventory row 1 (product Alpha), price 50 → row 3 (Gamma)
    assert_eq!(ids, HashSet::from([1, 3]), "Alpha (100) + Gamma (50)");
}
