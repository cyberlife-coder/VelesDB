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
use velesdb_core::velesql::{Parser, QueryValidator};
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, result_ids};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate a `products` collection for parameterized WHERE testing.
///
/// | id | category    | price  | stock | tags              |
/// |----|-------------|--------|-------|-------------------|
/// | 1  | electronics | 299.99 | 50    | new, sale         |
/// | 2  | electronics | 99.99  | 200   | sale              |
/// | 3  | books       | 19.99  | 30    | new               |
/// | 4  | books       | 29.99  | 0     | clearance         |
/// | 5  | clothing    | 49.99  | 100   | sale, clearance   |
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
            Some(
                json!({"category": "electronics", "price": 299.99, "stock": 50,
                "tags": ["new", "sale"]}),
            ),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(
                json!({"category": "electronics", "price": 99.99, "stock": 200,
                "tags": ["sale"]}),
            ),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"category": "books", "price": 19.99, "stock": 30,
                "tags": ["new"]})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"category": "books", "price": 29.99, "stock": 0,
                "tags": ["clearance"]})),
        ),
        Point::new(
            5,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({"category": "clothing", "price": 49.99, "stock": 100,
                "tags": ["sale", "clearance"]})),
        ),
    ])
    .expect("test: upsert products");
}

/// Execute a `VelesQL` aggregation query (GROUP BY/HAVING) with bind params.
///
/// Aggregation queries return `serde_json::Value` rather than
/// `Vec<SearchResult>`, so they go through `VectorCollection::execute_aggregate`.
fn execute_aggregate_sql_with_params(
    db: &Database,
    sql: &str,
    params: &HashMap<String, serde_json::Value>,
) -> velesdb_core::Result<serde_json::Value> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    let collection_name = &query.select.from;
    let vc = db
        .get_vector_collection(collection_name)
        .ok_or_else(|| velesdb_core::Error::CollectionNotFound(collection_name.clone()))?;
    vc.execute_aggregate(&query, params)
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

// =========================================================================
// Scenarios: CONTAINS / CONTAINS ANY with parameters
// =========================================================================

#[test]
fn scenario_where_contains_param_matches_literal() {
    // GIVEN a products collection with array tags
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN selecting with a literal CONTAINS and the parameterized equivalent
    let literal = execute_sql(
        &db,
        "SELECT * FROM products WHERE tags CONTAINS 'sale' LIMIT 10",
    )
    .expect("test: literal CONTAINS query");
    let params = params_from(&[("tag", json!("sale"))]);
    let parameterized = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE tags CONTAINS $tag LIMIT 10",
        &params,
    )
    .expect("test: parameterized CONTAINS query");

    // THEN both return exactly the 'sale' tagged rows {1, 2, 5}
    assert_eq!(result_ids(&literal), [1, 2, 5].into_iter().collect());
    assert_eq!(
        result_ids(&parameterized),
        result_ids(&literal),
        "CONTAINS $tag must return the same rows as the literal query"
    );
}

#[test]
fn scenario_where_contains_any_params_matches_literal() {
    // GIVEN a products collection with array tags
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN filtering with CONTAINS ANY over two parameters
    let literal = execute_sql(
        &db,
        "SELECT * FROM products WHERE tags CONTAINS ANY ('new', 'clearance') LIMIT 10",
    )
    .expect("test: literal CONTAINS ANY query");
    let params = params_from(&[("a", json!("new")), ("b", json!("clearance"))]);
    let parameterized = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE tags CONTAINS ANY ($a, $b) LIMIT 10",
        &params,
    )
    .expect("test: parameterized CONTAINS ANY query");

    // THEN both return rows tagged 'new' or 'clearance' {1, 3, 4, 5}
    assert_eq!(result_ids(&literal), [1, 3, 4, 5].into_iter().collect());
    assert_eq!(
        result_ids(&parameterized),
        result_ids(&literal),
        "CONTAINS ANY ($a, $b) must return the same rows as the literal query"
    );
}

#[test]
fn scenario_where_contains_missing_param_errors() {
    // GIVEN a products collection with array tags
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN executing CONTAINS with an unbound parameter
    let result = execute_sql_with_params(
        &db,
        "SELECT * FROM products WHERE tags CONTAINS $tag LIMIT 10",
        &HashMap::new(),
    );

    // THEN an explicit missing-parameter error is raised (not an empty result)
    let err = result.expect_err("missing CONTAINS parameter must be an error");
    assert!(
        err.to_string().contains("Missing parameter"),
        "error should name the missing parameter, got: {err}"
    );
}

// =========================================================================
// Scenarios: HAVING with parameters and subqueries
// =========================================================================

#[test]
fn scenario_having_count_param_matches_literal() {
    // GIVEN a products collection (electronics: 2, books: 2, clothing: 1)
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN filtering groups with a literal HAVING and the parameterized equivalent
    let literal = execute_aggregate_sql_with_params(
        &db,
        "SELECT category, COUNT(*) FROM products GROUP BY category \
         HAVING COUNT(*) > 1 ORDER BY category",
        &HashMap::new(),
    )
    .expect("test: literal HAVING query");
    let params = params_from(&[("n", json!(1))]);
    let parameterized = execute_aggregate_sql_with_params(
        &db,
        "SELECT category, COUNT(*) FROM products GROUP BY category \
         HAVING COUNT(*) > $n ORDER BY category",
        &params,
    )
    .expect("test: parameterized HAVING query");

    // THEN both keep exactly the two groups with more than one row
    let literal_groups = literal.as_array().expect("test: literal array");
    assert_eq!(
        literal_groups.len(),
        2,
        "HAVING COUNT(*) > 1 should keep 2 groups, got {literal}"
    );
    assert_eq!(
        parameterized, literal,
        "HAVING COUNT(*) > $n must keep the same groups as the literal query"
    );
}

#[test]
fn scenario_having_missing_param_errors() {
    // GIVEN a products collection
    let (_dir, db) = create_test_db();
    setup_products_collection(&db);

    // WHEN executing HAVING with an unbound parameter
    let result = execute_aggregate_sql_with_params(
        &db,
        "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > $n",
        &HashMap::new(),
    );

    // THEN an explicit missing-parameter error is raised (not an empty result)
    let err = result.expect_err("missing HAVING parameter must be an error, not zero groups");
    assert!(
        err.to_string().contains("Missing parameter"),
        "error should name the missing parameter, got: {err}"
    );
}

#[test]
fn scenario_having_scalar_subquery_accepted_by_validation() {
    // GIVEN a parsed aggregation query whose HAVING threshold is a scalar
    // (non-correlated) subquery — now executable (EPIC-039)
    let query = Parser::parse(
        "SELECT category, COUNT(*) FROM products GROUP BY category \
         HAVING COUNT(*) > (SELECT AVG(stock) FROM other)",
    )
    .expect("test: HAVING subquery must parse");

    // WHEN validating it (the same gate the server runs before execution)
    // THEN validation accepts it; the executor resolves the scalar before
    // running the aggregation
    assert!(
        QueryValidator::validate(&query).is_ok(),
        "scalar HAVING subquery must pass validation"
    );
}

#[test]
fn scenario_having_correlated_subquery_rejected_by_validation() {
    // A correlated HAVING subquery (referencing the outer table) is not yet
    // executable and must be rejected with V010.
    let query = Parser::parse(
        "SELECT category, COUNT(*) FROM products GROUP BY category \
         HAVING COUNT(*) > (SELECT AVG(stock) FROM sub WHERE products.category = 5)",
    )
    .expect("test: HAVING subquery must parse");

    let err = QueryValidator::validate(&query)
        .expect_err("correlated HAVING subquery must be rejected by validation");
    assert!(
        err.to_string().contains("V010"),
        "error should carry the V010 subquery code, got: {err}"
    );
}
