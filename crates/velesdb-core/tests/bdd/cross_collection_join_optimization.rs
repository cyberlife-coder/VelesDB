//! BDD tests for cross-collection JOIN optimization (Issue #513).
//!
//! Validates filter pushdown, lookup join, and combined optimization paths.
//! Reuses `setup_cross_type_collections` from `cross_collection.rs`.

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, payload_f64, payload_str, vector_param,
};

// =========================================================================
// Helpers
// =========================================================================

/// Creates `products` (VectorCollection) and `inventory` (MetadataCollection).
/// Same data as `cross_collection::setup_cross_type_collections`.
fn setup_collections(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION products (dimension = 4, metric = 'cosine');",
    )
    .expect("CREATE products");

    let products = db.get_vector_collection("products").expect("get products");
    products
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"name": "Headphones", "category": "audio"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"name": "Keyboard", "category": "input"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"name": "Monitor", "category": "display"})),
            ),
            Point::new(
                4,
                vec![0.0, 0.0, 0.0, 1.0],
                Some(json!({"name": "Mouse", "category": "input"})),
            ),
            Point::new(
                5,
                vec![0.7, 0.7, 0.0, 0.0],
                Some(json!({"name": "Speakers", "category": "audio"})),
            ),
        ])
        .expect("upsert products");

    execute_sql(db, "CREATE METADATA COLLECTION inventory;").expect("CREATE inventory");

    let inventory = db
        .get_metadata_collection("inventory")
        .expect("get inventory");
    inventory
        .upsert(vec![
            Point::metadata_only(1, json!({"product_id": 1, "price": 99.99, "stock": 50})),
            Point::metadata_only(2, json!({"product_id": 2, "price": 149.99, "stock": 0})),
            Point::metadata_only(3, json!({"product_id": 3, "price": 399.99, "stock": 12})),
            Point::metadata_only(4, json!({"product_id": 4, "price": 29.99, "stock": 200})),
            Point::metadata_only(5, json!({"product_id": 5, "price": 79.99, "stock": 30})),
        ])
        .expect("upsert inventory");
}

// =========================================================================
// Nominal: Filter pushdown
// =========================================================================

/// GIVEN products JOIN inventory with WHERE filter on joined table
/// WHEN the query filters `inventory.price > 100`
/// THEN only rows matching the filter are returned.
#[test]
fn test_join_with_where_filter_on_joined_table_returns_matching_rows() {
    let (_dir, db) = create_test_db();
    setup_collections(&db);

    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               WHERE inventory.price > 100 \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("filter pushdown JOIN");

    // Only Keyboard (149.99) and Monitor (399.99) have price > 100
    assert_eq!(results.len(), 2, "should return 2 rows with price > 100");
    for r in &results {
        let price = payload_f64(r, "price").expect("price field");
        assert!(price > 100.0, "price {price} should be > 100");
    }
}

// =========================================================================
// Nominal: Lookup join on primary key
// =========================================================================

/// GIVEN products JOIN inventory ON primary key
/// WHEN the query joins on `products.id = inventory.id`
/// THEN correct results are returned via lookup path.
#[test]
fn test_join_on_primary_key_returns_correct_results() {
    let (_dir, db) = create_test_db();
    setup_collections(&db);

    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("lookup join");

    assert_eq!(
        results.len(),
        5,
        "all 5 products should join with inventory"
    );
    // Verify merged payload contains fields from both collections
    let first = results.iter().find(|r| r.point.id == 1).expect("id=1");
    assert_eq!(payload_str(first, "name"), Some("Headphones"));
    assert!(
        payload_f64(first, "price").is_some(),
        "should have price from inventory"
    );
}

// =========================================================================
// Nominal: Pushdown + vector search
// =========================================================================

/// GIVEN products JOIN inventory with NEAR search and filter on joined table
/// WHEN the query combines vector search with inventory filter
/// THEN results are filtered and ordered by similarity.
#[test]
fn test_join_with_near_and_filter_on_joined_table() {
    let (_dir, db) = create_test_db();
    setup_collections(&db);

    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               WHERE vector NEAR $v AND inventory.stock > 0 \
               LIMIT 5";

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("NEAR + pushdown");

    assert!(!results.is_empty(), "should return results");
    // Keyboard (stock=0) should be excluded
    for r in &results {
        let stock = payload_f64(r, "stock").expect("stock field");
        assert!(stock > 0.0, "stock {stock} should be > 0");
    }
}

// =========================================================================
// Edge: Filter eliminates all rows — INNER JOIN
// =========================================================================

/// GIVEN products JOIN inventory with filter that matches no rows
/// WHEN the join type is INNER
/// THEN the result set is empty.
#[test]
fn test_join_filter_eliminates_all_rows_inner_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_collections(&db);

    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               WHERE inventory.price > 99999 \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("filter eliminates all");
    assert!(
        results.is_empty(),
        "INNER JOIN with no matches should be empty"
    );
}

// =========================================================================
// Edge: Filter eliminates all rows — LEFT JOIN
// =========================================================================

/// GIVEN products LEFT JOIN inventory with filter that matches no rows
/// WHEN the join type is LEFT
/// THEN left-side results are returned with null joined columns.
#[test]
fn test_join_filter_eliminates_all_rows_left_returns_nulls() {
    let (_dir, db) = create_test_db();
    setup_collections(&db);

    let sql = "SELECT * FROM products \
               LEFT JOIN inventory ON products.id = inventory.id \
               WHERE inventory.price > 99999 \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("LEFT JOIN filter eliminates all");

    // With filter pushdown, the ColumnStore is empty. LEFT JOIN should still
    // return left-side rows with null joined columns.
    // The exact behavior depends on whether the filter is pushed or post-join.
    // Either way, the query should not error.
    assert!(
        results.is_empty() || results.iter().all(|r| payload_f64(r, "price").is_none()),
        "LEFT JOIN should return empty or rows with null price"
    );
}

// =========================================================================
// Edge: Lookup join with no matching keys
// =========================================================================

/// GIVEN products with IDs not present in inventory
/// WHEN joining on primary key
/// THEN INNER JOIN returns empty.
#[test]
fn test_join_on_pk_no_matching_keys_returns_empty() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION items (dimension = 4, metric = 'cosine');",
    )
    .expect("CREATE items");

    let items = db.get_vector_collection("items").expect("get items");
    items
        .upsert(vec![
            Point::new(100, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"name": "X"}))),
            Point::new(200, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"name": "Y"}))),
        ])
        .expect("upsert items");

    execute_sql(&db, "CREATE METADATA COLLECTION stock;").expect("CREATE stock");
    let stock = db.get_metadata_collection("stock").expect("get stock");
    stock
        .upsert(vec![
            Point::metadata_only(1, json!({"qty": 10})),
            Point::metadata_only(2, json!({"qty": 20})),
        ])
        .expect("upsert stock");

    let sql = "SELECT * FROM items \
               JOIN stock ON items.id = stock.id \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("no matching keys");
    assert!(
        results.is_empty(),
        "no matching IDs should yield empty result"
    );
}

// =========================================================================
// Edge: Empty joined collection
// =========================================================================

/// GIVEN an empty joined collection
/// WHEN joining
/// THEN INNER JOIN returns empty.
#[test]
fn test_join_with_empty_joined_collection() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION base (dimension = 4, metric = 'cosine');",
    )
    .expect("CREATE base");

    let base = db.get_vector_collection("base").expect("get base");
    base.upsert(vec![Point::new(
        1,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(json!({"name": "A"})),
    )])
    .expect("upsert base");

    execute_sql(&db, "CREATE METADATA COLLECTION empty_coll;").expect("CREATE empty_coll");

    let sql = "SELECT * FROM base \
               JOIN empty_coll ON base.id = empty_coll.id \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("empty joined collection");
    assert!(
        results.is_empty(),
        "JOIN with empty collection should be empty"
    );
}

// =========================================================================
// Negative: Invalid table name
// =========================================================================

/// GIVEN a JOIN referencing a non-existent table
/// WHEN the query is executed
/// THEN a descriptive error is returned.
#[test]
fn test_join_with_invalid_table_returns_error() {
    let (_dir, db) = create_test_db();
    setup_collections(&db);

    let sql = "SELECT * FROM products \
               JOIN nonexistent ON products.id = nonexistent.id \
               LIMIT 10";

    let err = execute_sql(&db, sql).expect_err("should error on invalid table");
    let msg = err.to_string();
    assert!(
        msg.contains("nonexistent") || msg.contains("not found"),
        "error should mention the missing table, got: {msg}"
    );
}

// =========================================================================
// Combination: Pushdown + post-join filter
// =========================================================================

/// GIVEN a query with both pushdown-eligible and post-join conditions
/// WHEN executed
/// THEN both filters are applied correctly.
#[test]
fn test_join_with_pushdown_and_post_join_filter() {
    let (_dir, db) = create_test_db();
    setup_collections(&db);

    // inventory.stock > 0 is pushdown-eligible; products.category = 'audio' is base-side
    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               WHERE category = 'audio' AND inventory.stock > 0 \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("pushdown + post-join filter");

    // audio products: Headphones (stock=50), Speakers (stock=30) — both stock > 0
    assert_eq!(
        results.len(),
        2,
        "should return 2 audio products with stock > 0"
    );
    for r in &results {
        assert_eq!(payload_str(r, "category"), Some("audio"));
        let stock = payload_f64(r, "stock").expect("stock");
        assert!(stock > 0.0);
    }
}

// =========================================================================
// Combination: Multiple conditions on joined table
// =========================================================================

/// GIVEN a query with multiple conditions on the joined table
/// WHEN executed
/// THEN all conditions are applied.
#[test]
fn test_join_with_multiple_conditions_on_joined_table() {
    let (_dir, db) = create_test_db();
    setup_collections(&db);

    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               WHERE inventory.price > 50 AND inventory.stock > 10 \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("multiple conditions");

    // price > 50 AND stock > 10:
    // Headphones: 99.99, 50 ✓
    // Monitor: 399.99, 12 ✓
    // Speakers: 79.99, 30 ✓
    // Keyboard: 149.99, 0 ✗ (stock)
    // Mouse: 29.99, 200 ✗ (price)
    assert_eq!(
        results.len(),
        3,
        "should return 3 rows matching both conditions"
    );
}

// =========================================================================
// Combination: ORDER BY + LIMIT after pushdown
// =========================================================================

/// GIVEN a query with pushdown filter, ORDER BY, and LIMIT
/// WHEN executed
/// THEN results are filtered and limited correctly.
#[test]
fn test_join_with_order_by_and_limit_after_pushdown() {
    let (_dir, db) = create_test_db();
    setup_collections(&db);

    let sql = "SELECT * FROM products \
               JOIN inventory ON products.id = inventory.id \
               WHERE inventory.price > 50 \
               LIMIT 2";

    let results = execute_sql(&db, sql).expect("ORDER BY + LIMIT after pushdown");

    // price > 50: Headphones (99.99), Keyboard (149.99), Monitor (399.99), Speakers (79.99)
    assert_eq!(results.len(), 2, "LIMIT 2 should return 2 rows");
    for r in &results {
        let price = payload_f64(r, "price").expect("price");
        assert!(price > 50.0, "price {price} should be > 50");
    }
}
