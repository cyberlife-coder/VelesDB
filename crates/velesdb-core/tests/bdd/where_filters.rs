//! BDD-style end-to-end tests for `VelesQL` WHERE clause behaviors.
//!
//! Each scenario follows GIVEN (setup data) -> WHEN (execute SQL) -> THEN (verify
//! results).  Tests exercise the **full pipeline**: SQL string -> `Parser::parse()`
//! -> `Database::execute_query()` -> verify returned `SearchResult` values.

use std::collections::HashSet;

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, result_ids};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate a `products` collection with diverse test data for WHERE filtering.
///
/// | id | category    | price  | name             | brand  | stock | active |
/// |----|-------------|--------|------------------|--------|-------|--------|
/// | 1  | electronics | 299.99 | Laptop           | Acme   | 50    | true   |
/// | 2  | electronics | 99.99  | Mouse            | Acme   | 200   | true   |
/// | 3  | books       | 19.99  | Rust Programming | null   | 30    | true   |
/// | 4  | books       | 29.99  | Python Cookbook   | null   | 0     | false  |
/// | 5  | clothing    | 49.99  | T-Shirt          | Style  | 100   | true   |
fn setup_products_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION products (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE products");

    let vc = db
        .get_vector_collection("products")
        .expect("test: get products collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"category": "electronics", "price": 299.99, "name": "Laptop", "brand": "Acme", "stock": 50, "active": true})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"category": "electronics", "price": 99.99, "name": "Mouse", "brand": "Acme", "stock": 200, "active": true})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"category": "books", "price": 19.99, "name": "Rust Programming", "brand": null, "stock": 30, "active": true})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"category": "books", "price": 29.99, "name": "Python Cookbook", "brand": null, "stock": 0, "active": false})),
        ),
        Point::new(
            5,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({"category": "clothing", "price": 49.99, "name": "T-Shirt", "brand": "Style", "stock": 100, "active": true})),
        ),
    ])
    .expect("test: upsert products");
}

// =========================================================================
// Scenario 1: Equality filter returns matching rows
// =========================================================================

#[test]
fn test_where_equality_returns_matching_rows() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE category = 'electronics' LIMIT 10;",
    )
    .expect("test: equality filter should succeed");

    assert_eq!(
        results.len(),
        2,
        "Should return exactly 2 electronics items"
    );
    let ids = result_ids(&results);
    assert!(ids.contains(&1), "Should contain Laptop (id=1)");
    assert!(ids.contains(&2), "Should contain Mouse (id=2)");
}

// =========================================================================
// Scenario 2: Greater-than filter
// =========================================================================

#[test]
fn test_where_greater_than_filters_correctly() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(&db, "SELECT * FROM products WHERE price > 50 LIMIT 10;")
        .expect("test: greater-than filter should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 2, "Should return 2 items with price > 50");
    assert!(ids.contains(&1), "Laptop (299.99) should match price > 50");
    assert!(ids.contains(&2), "Mouse (99.99) should match price > 50");
}

// =========================================================================
// Scenario 3: AND combines filters
// =========================================================================

#[test]
fn test_where_and_combines_filters() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE category = 'electronics' AND price < 200 LIMIT 10;",
    )
    .expect("test: AND filter should succeed");

    assert_eq!(
        results.len(),
        1,
        "Only Mouse (99.99) is electronics under 200"
    );
    assert_eq!(results[0].point.id, 2, "Result should be Mouse (id=2)");
}

// =========================================================================
// Scenario 4: OR widens results
// =========================================================================

#[test]
fn test_where_or_widens_results() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE category = 'electronics' OR category = 'clothing' LIMIT 10;",
    )
    .expect("test: OR filter should succeed");

    let ids = result_ids(&results);
    assert_eq!(
        ids.len(),
        3,
        "Should return electronics + clothing = 3 items"
    );
    let expected: HashSet<u64> = [1, 2, 5].into_iter().collect();
    assert_eq!(ids, expected, "Should contain ids 1, 2, 5");
}

// =========================================================================
// Scenario 5: IN matches set membership
// =========================================================================

#[test]
fn test_where_in_matches_set_membership() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE category IN ('books', 'clothing') LIMIT 10;",
    )
    .expect("test: IN filter should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 3, "Should return books + clothing = 3 items");
    let expected: HashSet<u64> = [3, 4, 5].into_iter().collect();
    assert_eq!(ids, expected, "Should contain ids 3, 4, 5");
}

// =========================================================================
// Scenario 6: NOT IN excludes values
// =========================================================================

#[test]
fn test_where_not_in_excludes_values() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE category NOT IN ('electronics') LIMIT 10;",
    )
    .expect("test: NOT IN filter should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 3, "Should exclude electronics, leaving 3 items");
    let expected: HashSet<u64> = [3, 4, 5].into_iter().collect();
    assert_eq!(ids, expected, "Should contain ids 3, 4, 5");
}

// =========================================================================
// Scenario 7: BETWEEN filters range inclusively
// =========================================================================

#[test]
fn test_where_between_filters_range_inclusively() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE price BETWEEN 20 AND 100 LIMIT 10;",
    )
    .expect("test: BETWEEN filter should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 3, "Three items have price in [20, 100]");
    let expected: HashSet<u64> = [2, 4, 5].into_iter().collect();
    assert_eq!(ids, expected, "Should contain ids 2, 4, 5");
}

// =========================================================================
// Scenario 8: IS NULL finds null values
// =========================================================================

#[test]
fn test_where_is_null_finds_null_values() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(&db, "SELECT * FROM products WHERE brand IS NULL LIMIT 10;")
        .expect("test: IS NULL filter should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 2, "Two items have null brand");
    let expected: HashSet<u64> = [3, 4].into_iter().collect();
    assert_eq!(ids, expected, "Should contain ids 3, 4 (books)");
}

// =========================================================================
// Scenario 9: IS NOT NULL finds non-null values
// =========================================================================

#[test]
fn test_where_is_not_null_finds_non_null_values() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE brand IS NOT NULL LIMIT 10;",
    )
    .expect("test: IS NOT NULL filter should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 3, "Three items have non-null brand");
    let expected: HashSet<u64> = [1, 2, 5].into_iter().collect();
    assert_eq!(ids, expected, "Should contain ids 1, 2, 5");
}

// =========================================================================
// Scenario 10: LIKE pattern matching (substring)
// =========================================================================

#[test]
fn test_where_like_substring_match() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE name LIKE '%Shirt%' LIMIT 10;",
    )
    .expect("test: LIKE substring filter should succeed");

    assert_eq!(results.len(), 1, "Only T-Shirt contains 'Shirt'");
    assert_eq!(results[0].point.id, 5, "Result should be T-Shirt (id=5)");
}

// =========================================================================
// Scenario 11: LIKE prefix match
// =========================================================================

#[test]
fn test_where_like_prefix_match() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE name LIKE 'Rust%' LIMIT 10;",
    )
    .expect("test: LIKE prefix filter should succeed");

    assert_eq!(
        results.len(),
        1,
        "Only 'Rust Programming' starts with 'Rust'"
    );
    assert_eq!(
        results[0].point.id, 3,
        "Result should be Rust Programming (id=3)"
    );
}

// =========================================================================
// Scenario 12: No match returns empty results
// =========================================================================

#[test]
fn test_where_no_match_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE category = 'automotive' LIMIT 10;",
    )
    .expect("test: non-matching filter should succeed");

    assert!(
        results.is_empty(),
        "No items match category='automotive', should return empty"
    );
}

// =========================================================================
// Scenario 13: Complex AND + OR with grouping
// =========================================================================

#[test]
fn test_where_complex_and_or_grouping() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM products WHERE (category = 'electronics' OR category = 'books') AND price < 30 LIMIT 10;",
    )
    .expect("test: complex AND+OR filter should succeed");

    let ids = result_ids(&results);
    assert_eq!(ids.len(), 2, "Only two items match the compound condition");
    let expected: HashSet<u64> = [3, 4].into_iter().collect();
    assert_eq!(ids, expected, "Should contain ids 3, 4 (books under 30)");
}

// =========================================================================
// Scenario 14: Vector NEAR with metadata filter
// =========================================================================

#[test]
fn test_where_vector_near_with_filter() {
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), json!([1.0_f32, 0.0, 0.0, 0.0]));

    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE vector NEAR $v AND category = 'electronics' LIMIT 2;",
        &params,
    )
    .expect("test: NEAR + filter should succeed");

    assert_eq!(
        results.len(),
        2,
        "NEAR + category filter must return exactly 2 electronics items, got {}",
        results.len()
    );

    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(serde_json::Value::as_str);
        assert_eq!(
            cat,
            Some("electronics"),
            "All NEAR results must pass the category filter, got {:?} for id={}",
            cat,
            r.point.id
        );
    }

    let ids = result_ids(&results);
    let electronics: HashSet<u64> = [1, 2].into_iter().collect();
    assert_eq!(
        ids, electronics,
        "NEAR + filter must return both electronics ids (1, 2), got {:?}",
        ids
    );
}

// =========================================================================
// Scenario: BETWEEN on string and date bounds (docs/VELESQL_SPEC.md example)
// =========================================================================

/// `events`: four points with an ISO `date` and a `name`, in no sorted order.
fn setup_events(db: &Database) {
    db.create_vector_collection("events", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create events");
    let vc = db
        .get_vector_collection("events")
        .expect("test: get events");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"date": "2023-12-31", "name": "delta"})),
        ),
        Point::new(
            2,
            vec![1.0, 0.0],
            Some(json!({"date": "2024-03-15", "name": "alpha"})),
        ),
        Point::new(
            3,
            vec![1.0, 0.0],
            Some(json!({"date": "2024-12-31", "name": "charlie"})),
        ),
        Point::new(
            4,
            vec![1.0, 0.0],
            Some(json!({"date": "2025-01-01", "name": "bravo"})),
        ),
    ])
    .expect("test: upsert events");
}

/// GIVEN dates stored as ISO strings
/// WHEN the spec's `date BETWEEN '2024-01-01' AND '2024-12-31'` runs
/// THEN it keeps the 2024 dates, bounds included — it used to match nothing,
///      because BETWEEN turned string bounds into `null`.
#[test]
fn test_where_between_date_strings_keeps_the_range() {
    let (_dir, db) = create_test_db();
    setup_events(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM events WHERE date BETWEEN '2024-01-01' AND '2024-12-31' LIMIT 10;",
    )
    .expect("test: BETWEEN on dates");
    assert_eq!(
        result_ids(&results),
        [2, 3].into_iter().collect::<HashSet<u64>>()
    );
}

/// GIVEN string payloads
/// WHEN BETWEEN and its definition `>= AND <=` run with the same bounds
/// THEN they return the same rows.
#[test]
fn test_where_between_strings_equals_its_definition() {
    let (_dir, db) = create_test_db();
    setup_events(&db);

    let between = execute_sql(
        &db,
        "SELECT * FROM events WHERE name BETWEEN 'alpha' AND 'charlie' LIMIT 10;",
    )
    .expect("test: BETWEEN on names");
    let definition = execute_sql(
        &db,
        "SELECT * FROM events WHERE name >= 'alpha' AND name <= 'charlie' LIMIT 10;",
    )
    .expect("test: >= AND <= on names");
    assert_eq!(
        result_ids(&between),
        [2, 3, 4].into_iter().collect::<HashSet<u64>>()
    );
    assert_eq!(result_ids(&between), result_ids(&definition));
}
