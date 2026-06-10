//! BDD-style end-to-end tests for parameter placeholders in WHERE clauses.
//!
//! Each scenario follows GIVEN (setup data) -> WHEN (execute SQL with params)
//! -> THEN (verify results).  Regression coverage for the silent-NULL bug:
//! `SELECT * FROM products WHERE category = $cat` with `{cat: "x"}` returned
//! 0 results without any error because `Value::Parameter` was converted to
//! JSON `null` before the filter ever saw the bound value.  A parameterized
//! query must return exactly the same rows as its literal equivalent, and a
//! missing parameter must produce an explicit error, never an empty result.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, result_ids};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate a `products` collection for parameterized WHERE testing.
///
/// | id | category    | price  | stock |
/// |----|-------------|--------|-------|
/// | 1  | electronics | 299.99 | 50    |
/// | 2  | electronics | 99.99  | 200   |
/// | 3  | books       | 19.99  | 30    |
/// | 4  | books       | 29.99  | 0     |
/// | 5  | clothing    | 49.99  | 100   |
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
            Some(json!({"category": "electronics", "price": 299.99, "stock": 50})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"category": "electronics", "price": 99.99, "stock": 200})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"category": "books", "price": 19.99, "stock": 30})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"category": "books", "price": 29.99, "stock": 0})),
        ),
        Point::new(
            5,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({"category": "clothing", "price": 49.99, "stock": 100})),
        ),
    ])
    .expect("test: upsert products");
}

fn params_from(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

// =========================================================================
// Scenarios: parameterized WHERE must match the literal equivalent
// =========================================================================

#[test]
fn scenario_where_eq_string_param_matches_literal() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN selecting with a literal and with an equivalent parameter
    let literal = execute_sql(
        &db,
        "SELECT * FROM products WHERE category = 'electronics' LIMIT 10",
    )
    .expect("test: literal query");
    let params = params_from(&[("cat", json!("electronics"))]);
    let parameterized = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE category = $cat LIMIT 10",
        &params,
    )
    .expect("test: parameterized query");

    // THEN both return exactly ids {1, 2}
    assert_eq!(result_ids(&literal), [1, 2].into_iter().collect());
    assert_eq!(
        result_ids(&parameterized),
        result_ids(&literal),
        "parameterized WHERE must return the same rows as the literal query"
    );
}

#[test]
fn scenario_where_in_params_matches_literal() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN filtering with IN over two parameters
    let params = params_from(&[("a", json!("books")), ("b", json!("clothing"))]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE category IN ($a, $b) LIMIT 10",
        &params,
    )
    .expect("test: IN params query");

    // THEN books (3, 4) and clothing (5) are returned
    assert_eq!(result_ids(&results), [3, 4, 5].into_iter().collect());
}

#[test]
fn scenario_where_between_params_matches_literal() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN filtering with BETWEEN over two parameters
    let params = params_from(&[("lo", json!(20.0)), ("hi", json!(100.0))]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE price BETWEEN $lo AND $hi LIMIT 10",
        &params,
    )
    .expect("test: BETWEEN params query");

    // THEN prices 29.99 (4), 49.99 (5), 99.99 (2) are in range
    assert_eq!(result_ids(&results), [2, 4, 5].into_iter().collect());
}

#[test]
fn scenario_where_gt_param_matches_literal() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN filtering with a numeric comparison parameter
    let params = params_from(&[("min", json!(50.0))]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE price > $min LIMIT 10",
        &params,
    )
    .expect("test: > param query");

    // THEN only prices above 50 (ids 1, 2) are returned
    assert_eq!(result_ids(&results), [1, 2].into_iter().collect());
}

#[test]
fn scenario_near_with_scalar_param_filter() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN combining NEAR with a parameterized metadata filter
    let mut params = params_from(&[("cat", json!("books"))]);
    params.insert("v".to_string(), json!([0.0, 0.0, 1.0, 0.0]));
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE vector NEAR $v AND category = $cat LIMIT 10",
        &params,
    )
    .expect("test: NEAR + scalar param query");

    // THEN only books (ids 3, 4) survive the metadata filter
    assert_eq!(result_ids(&results), [3, 4].into_iter().collect());
}

// =========================================================================
// Scenarios: missing parameters must error, never return empty results
// =========================================================================

#[test]
fn scenario_where_missing_param_errors() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN executing with an unbound parameter
    let result = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE category = $cat LIMIT 10",
        &HashMap::new(),
    );

    // THEN an explicit missing-parameter error is raised (not an empty result)
    let err = result.expect_err("missing parameter must be an error, not an empty result set");
    assert!(
        err.to_string().contains("Missing parameter"),
        "error should name the missing parameter, got: {err}"
    );
}

#[test]
fn scenario_where_in_missing_param_errors() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN executing IN with only one of two parameters bound
    let params = params_from(&[("a", json!("books"))]);
    let result = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE category IN ($a, $b) LIMIT 10",
        &params,
    );

    // THEN the unbound $b is reported as an error
    let err = result.expect_err("missing IN parameter must be an error");
    assert!(
        err.to_string().contains("Missing parameter"),
        "error should name the missing parameter, got: {err}"
    );
}

// =========================================================================
// Scenarios: DML UPDATE WHERE with parameters
// =========================================================================

#[test]
fn scenario_update_where_param_updates_only_matching_rows() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN updating rows selected by a parameterized WHERE
    let params = params_from(&[("cat", json!("electronics"))]);
    let updated = execute_sql_with_params(
        &db,
        "UPDATE products SET stock = 0 WHERE category = $cat",
        &params,
    )
    .expect("test: UPDATE with param");

    // THEN exactly the two electronics rows were updated
    assert_eq!(
        result_ids(&updated),
        [1, 2].into_iter().collect(),
        "UPDATE WHERE category = $cat must touch only matching rows"
    );
}

#[test]
fn scenario_update_where_missing_param_errors() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN updating with an unbound WHERE parameter
    let result = execute_sql_with_params(
        &db,
        "UPDATE products SET stock = 0 WHERE category = $cat",
        &HashMap::new(),
    );

    // THEN an explicit missing-parameter error is raised (no rows touched)
    let err = result.expect_err("missing UPDATE WHERE parameter must be an error");
    assert!(
        err.to_string().contains("Missing parameter"),
        "error should name the missing parameter, got: {err}"
    );
}
