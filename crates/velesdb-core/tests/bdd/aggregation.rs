//! BDD-style end-to-end tests for `VelesQL` aggregation, GROUP BY, HAVING,
//! ORDER BY, LIMIT/OFFSET, and DISTINCT behaviors.
//!
//! These tests exercise the **full pipeline** from the user's perspective:
//! SQL string -> `Parser::parse()` -> `Database::execute_query()` (or
//! `VectorCollection::execute_aggregate()` for aggregation) -> verify results.

use std::collections::HashMap;

use velesdb_core::{velesql::Parser, Database, Point};

use super::helpers::{create_test_db, execute_sql, payload_f64, payload_str};

// =========================================================================
// Module-specific helpers
// =========================================================================

/// Execute a `VelesQL` aggregation query through the typed collection API.
///
/// Aggregation queries (GROUP BY, COUNT, SUM, AVG, MIN, MAX) return
/// `serde_json::Value` rather than `Vec<SearchResult>`, so they must go
/// through `VectorCollection::execute_aggregate`.
fn execute_aggregate_sql(db: &Database, sql: &str) -> velesdb_core::Result<serde_json::Value> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    let collection_name = &query.select.from;
    let vc = db
        .get_vector_collection(collection_name)
        .ok_or_else(|| velesdb_core::Error::CollectionNotFound(collection_name.clone()))?;
    vc.execute_aggregate(&query, &HashMap::new())
}

/// Seed the "orders" collection with 10 orders across 3 categories.
///
/// | id | category    | amount | status    |
/// |----|-------------|--------|-----------|
/// |  1 | electronics | 299.99 | delivered |
/// |  2 | electronics |  49.99 | delivered |
/// |  3 | electronics | 149.99 | pending   |
/// |  4 | books       |  19.99 | delivered |
/// |  5 | books       |  29.99 | delivered |
/// |  6 | books       |   9.99 | cancelled |
/// |  7 | clothing    |  59.99 | delivered |
/// |  8 | clothing    |  39.99 | pending   |
/// |  9 | clothing    |  79.99 | delivered |
/// | 10 | clothing    |  19.99 | cancelled |
fn setup_orders_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION orders (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create orders");

    let vc = db
        .get_vector_collection("orders")
        .expect("test: get orders");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(
                serde_json::json!({"category":"electronics","amount":299.99,"status":"delivered"}),
            ),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(serde_json::json!({"category":"electronics","amount":49.99,"status":"delivered"})),
        ),
        Point::new(
            3,
            vec![0.8, 0.2, 0.0, 0.0],
            Some(serde_json::json!({"category":"electronics","amount":149.99,"status":"pending"})),
        ),
        Point::new(
            4,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"category":"books","amount":19.99,"status":"delivered"})),
        ),
        Point::new(
            5,
            vec![0.0, 0.9, 0.1, 0.0],
            Some(serde_json::json!({"category":"books","amount":29.99,"status":"delivered"})),
        ),
        Point::new(
            6,
            vec![0.0, 0.8, 0.2, 0.0],
            Some(serde_json::json!({"category":"books","amount":9.99,"status":"cancelled"})),
        ),
        Point::new(
            7,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(serde_json::json!({"category":"clothing","amount":59.99,"status":"delivered"})),
        ),
        Point::new(
            8,
            vec![0.0, 0.0, 0.9, 0.1],
            Some(serde_json::json!({"category":"clothing","amount":39.99,"status":"pending"})),
        ),
        Point::new(
            9,
            vec![0.0, 0.0, 0.8, 0.2],
            Some(serde_json::json!({"category":"clothing","amount":79.99,"status":"delivered"})),
        ),
        Point::new(
            10,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(serde_json::json!({"category":"clothing","amount":19.99,"status":"cancelled"})),
        ),
    ])
    .expect("test: upsert orders");
}

// =========================================================================
// Scenario 1: SELECT * LIMIT returns correct count
// =========================================================================

#[test]
fn test_limit_returns_correct_count() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let results = execute_sql(&db, "SELECT * FROM orders LIMIT 5").expect("test: SELECT LIMIT 5");
    assert_eq!(
        results.len(),
        5,
        "LIMIT 5 should return exactly 5 results, got {}",
        results.len()
    );
}

// =========================================================================
// Scenario 2: LIMIT + OFFSET provides pagination
// =========================================================================

#[test]
fn test_limit_offset_pagination() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let page1 = execute_sql(&db, "SELECT * FROM orders LIMIT 3 OFFSET 0").expect("test: page 1");
    let page2 = execute_sql(&db, "SELECT * FROM orders LIMIT 3 OFFSET 3").expect("test: page 2");
    let page3 = execute_sql(&db, "SELECT * FROM orders LIMIT 3 OFFSET 6").expect("test: page 3");

    assert_eq!(page1.len(), 3, "Page 1 should have 3 results");
    assert_eq!(page2.len(), 3, "Page 2 should have 3 results");
    assert!(
        page3.len() <= 3,
        "Page 3 should have at most 3 results, got {}",
        page3.len()
    );

    // All IDs across pages should be unique (no overlap).
    let all_ids: std::collections::HashSet<u64> = page1
        .iter()
        .chain(page2.iter())
        .chain(page3.iter())
        .map(|r| r.point.id)
        .collect();
    let total_count = page1.len() + page2.len() + page3.len();
    assert_eq!(
        all_ids.len(),
        total_count,
        "No duplicate IDs across pages: {} unique vs {} total",
        all_ids.len(),
        total_count
    );
}

// =========================================================================
// Scenario 3: ORDER BY field sorts correctly
// =========================================================================

#[test]
fn test_order_by_field_descending() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let results = execute_sql(&db, "SELECT * FROM orders ORDER BY amount DESC LIMIT 3")
        .expect("test: ORDER BY amount DESC");

    assert_eq!(results.len(), 3, "Should return 3 results");

    let first_amount =
        payload_f64(&results[0], "amount").expect("test: first result should have amount");
    assert!(
        (first_amount - 299.99).abs() < 0.01,
        "First result should be 299.99, got {}",
        first_amount
    );

    for window in results.windows(2) {
        let a = payload_f64(&window[0], "amount").expect("test: amount field");
        let b = payload_f64(&window[1], "amount").expect("test: amount field");
        assert!(
            a >= b,
            "Results should be in descending order: {} should be >= {}",
            a,
            b
        );
    }
}

// =========================================================================
// Scenario 4: GROUP BY counts per category
// =========================================================================

#[test]
fn test_group_by_counts_per_category() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category",
    )
    .expect("test: GROUP BY COUNT");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    assert_eq!(groups.len(), 3, "Should have 3 categories");

    let counts: HashMap<String, i64> = groups
        .iter()
        .filter_map(|g| {
            let cat = g.get("category")?.as_str()?.to_string();
            let count = g.get("count")?.as_i64()?;
            Some((cat, count))
        })
        .collect();

    assert_eq!(
        counts.get("electronics"),
        Some(&3),
        "electronics should have 3 orders"
    );
    assert_eq!(counts.get("books"), Some(&3), "books should have 3 orders");
    assert_eq!(
        counts.get("clothing"),
        Some(&4),
        "clothing should have 4 orders"
    );
}

// =========================================================================
// Scenario 5: GROUP BY with SUM aggregation
// =========================================================================

#[test]
fn test_group_by_sum_aggregation() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, SUM(amount) FROM orders GROUP BY category",
    )
    .expect("test: GROUP BY SUM");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    assert_eq!(groups.len(), 3, "Should have 3 categories");

    let sums: HashMap<String, f64> = groups
        .iter()
        .filter_map(|g| {
            let cat = g.get("category")?.as_str()?.to_string();
            let sum = g.get("sum_amount")?.as_f64()?;
            Some((cat, sum))
        })
        .collect();

    let electronics_sum = sums
        .get("electronics")
        .expect("test: electronics sum should exist");
    assert!(
        (*electronics_sum - 499.97).abs() < 0.1,
        "electronics sum should be ~499.97, got {}",
        electronics_sum
    );

    let books_sum = sums.get("books").expect("test: books sum should exist");
    assert!(
        (*books_sum - 59.97).abs() < 0.1,
        "books sum should be ~59.97, got {}",
        books_sum
    );

    let clothing_sum = sums
        .get("clothing")
        .expect("test: clothing sum should exist");
    assert!(
        (*clothing_sum - 199.96).abs() < 0.1,
        "clothing sum should be ~199.96, got {}",
        clothing_sum
    );
}

// =========================================================================
// Scenario 6: HAVING filters groups
// =========================================================================

#[test]
fn test_having_filters_groups() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category HAVING COUNT(*) > 3",
    )
    .expect("test: GROUP BY HAVING");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");

    assert_eq!(
        groups.len(),
        1,
        "HAVING COUNT(*) > 3 should return 1 group, got {}",
        groups.len()
    );

    let category = groups[0]
        .get("category")
        .and_then(serde_json::Value::as_str)
        .expect("test: group should have category");
    assert_eq!(category, "clothing", "Only clothing has count > 3");

    let count = groups[0]
        .get("count")
        .and_then(serde_json::Value::as_i64)
        .expect("test: group should have count");
    assert_eq!(count, 4, "clothing should have count = 4");
}

// =========================================================================
// Scenario 7: DISTINCT eliminates duplicate values
// =========================================================================

#[test]
fn test_distinct_eliminates_duplicates() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let results = execute_sql(&db, "SELECT DISTINCT category FROM orders LIMIT 10")
        .expect("test: SELECT DISTINCT");

    let categories: std::collections::HashSet<&str> = results
        .iter()
        .filter_map(|r| payload_str(r, "category"))
        .collect();

    assert_eq!(
        categories.len(),
        3,
        "DISTINCT should yield 3 unique categories, got {:?}",
        categories
    );
    assert!(categories.contains("electronics"));
    assert!(categories.contains("books"));
    assert!(categories.contains("clothing"));
}

// =========================================================================
// Scenario 8: DISTINCT with filter
// =========================================================================

#[test]
fn test_distinct_with_filter() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT DISTINCT status FROM orders WHERE category = 'electronics' LIMIT 10",
    )
    .expect("test: SELECT DISTINCT with WHERE");

    let statuses: std::collections::HashSet<&str> = results
        .iter()
        .filter_map(|r| payload_str(r, "status"))
        .collect();

    assert_eq!(
        statuses.len(),
        2,
        "electronics should have 2 distinct statuses, got {:?}",
        statuses
    );
    assert!(statuses.contains("delivered"), "Should contain 'delivered'");
    assert!(statuses.contains("pending"), "Should contain 'pending'");
}

// =========================================================================
// Scenario 9: COUNT(*) without GROUP BY counts all rows
// =========================================================================

#[test]
fn test_count_star_without_group_by() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(&db, "SELECT COUNT(*) FROM orders").expect("test: COUNT(*)");

    let count = result
        .get("count")
        .and_then(serde_json::Value::as_i64)
        .expect("test: result should have 'count' field");
    assert_eq!(count, 10, "COUNT(*) should return 10 for 10 rows");
}

// =========================================================================
// Scenario 10: Empty result set on impossible filter
// =========================================================================

#[test]
fn test_empty_result_on_impossible_filter() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let results = execute_sql(&db, "SELECT * FROM orders WHERE amount > 999 LIMIT 10")
        .expect("test: impossible filter");
    assert!(
        results.is_empty(),
        "Filter amount > 999 should return 0 results, got {}",
        results.len()
    );
}

// =========================================================================
// Scenario 11: OFFSET beyond data returns empty
// =========================================================================

#[test]
fn test_offset_beyond_data_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let results = execute_sql(&db, "SELECT * FROM orders LIMIT 10 OFFSET 100")
        .expect("test: OFFSET beyond data");
    assert!(
        results.is_empty(),
        "OFFSET 100 on 10 rows should return empty, got {}",
        results.len()
    );
}

// =========================================================================
// Scenario 12: ORDER BY ascending
// =========================================================================

#[test]
fn test_order_by_field_ascending() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let results = execute_sql(&db, "SELECT * FROM orders ORDER BY amount ASC LIMIT 10")
        .expect("test: ORDER BY amount ASC");

    assert_eq!(results.len(), 10, "Should return all 10 results");

    let first_amount =
        payload_f64(&results[0], "amount").expect("test: first result should have amount");
    assert!(
        (first_amount - 9.99).abs() < 0.01,
        "First result should be 9.99, got {}",
        first_amount
    );

    for window in results.windows(2) {
        let a = payload_f64(&window[0], "amount").expect("test: amount field");
        let b = payload_f64(&window[1], "amount").expect("test: amount field");
        assert!(
            a <= b,
            "Results should be in ascending order: {} should be <= {}",
            a,
            b
        );
    }
}

// =========================================================================
// Scenario 13: GROUP BY with AVG aggregation
// =========================================================================

#[test]
fn test_group_by_avg_aggregation() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, AVG(amount) FROM orders GROUP BY category",
    )
    .expect("test: GROUP BY AVG");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    assert_eq!(groups.len(), 3, "Should have 3 categories");

    let avgs: HashMap<String, f64> = groups
        .iter()
        .filter_map(|g| {
            let cat = g.get("category")?.as_str()?.to_string();
            let avg = g.get("avg_amount")?.as_f64()?;
            Some((cat, avg))
        })
        .collect();

    let electronics_avg = avgs
        .get("electronics")
        .expect("test: electronics avg should exist");
    assert!(
        (*electronics_avg - 166.66).abs() < 1.0,
        "electronics avg should be ~166.66, got {}",
        electronics_avg
    );

    let clothing_avg = avgs
        .get("clothing")
        .expect("test: clothing avg should exist");
    assert!(
        (*clothing_avg - 49.99).abs() < 1.0,
        "clothing avg should be ~49.99, got {}",
        clothing_avg
    );
}

// =========================================================================
// Scenario 14: LIMIT 0 returns empty
// =========================================================================

#[test]
fn test_limit_zero_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let results = execute_sql(&db, "SELECT * FROM orders LIMIT 0").expect("test: LIMIT 0");
    assert!(
        results.is_empty(),
        "LIMIT 0 should return 0 results, got {}",
        results.len()
    );
}

// =========================================================================
// Scenario 15: HAVING with >= operator
// =========================================================================

#[test]
fn test_having_with_gte_operator() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category HAVING COUNT(*) >= 3",
    )
    .expect("test: HAVING >=");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");

    assert_eq!(
        groups.len(),
        3,
        "HAVING COUNT(*) >= 3 should return all 3 groups, got {}",
        groups.len()
    );
}

// =========================================================================
// Scenario 16: LIMIT truncates GROUP BY results (issue #1556)
// =========================================================================

#[test]
fn test_group_by_with_limit_truncates_groups() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category ORDER BY category ASC LIMIT 2",
    )
    .expect("test: GROUP BY LIMIT");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    assert_eq!(
        groups.len(),
        2,
        "LIMIT 2 should truncate 3 groups down to 2, got {}",
        groups.len()
    );

    let categories: Vec<&str> = groups
        .iter()
        .filter_map(|g| g.get("category")?.as_str())
        .collect();
    assert_eq!(
        categories,
        vec!["books", "clothing"],
        "LIMIT should keep the first 2 groups in ORDER BY category ASC order"
    );
}

// =========================================================================
// Scenario 17: OFFSET skips groups before LIMIT on GROUP BY results (#1556)
// =========================================================================

#[test]
fn test_group_by_with_offset_and_limit() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category ORDER BY category ASC LIMIT 1 OFFSET 1",
    )
    .expect("test: GROUP BY OFFSET LIMIT");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    assert_eq!(
        groups.len(),
        1,
        "OFFSET 1 LIMIT 1 should return exactly 1 group, got {}",
        groups.len()
    );
    assert_eq!(
        groups[0].get("category").and_then(|v| v.as_str()),
        Some("clothing"),
        "OFFSET 1 should skip 'books' and return the second group 'clothing'"
    );
}

// =========================================================================
// Scenario 18: GROUP BY without LIMIT still returns every group (#1556)
// =========================================================================

#[test]
fn test_group_by_without_limit_returns_all_groups() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category",
    )
    .expect("test: GROUP BY without LIMIT");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    assert_eq!(
        groups.len(),
        3,
        "A LIMIT-less GROUP BY must return every group (no default cap), got {}",
        groups.len()
    );
}

// =========================================================================
// Scenario 19: OFFSET without LIMIT skips groups on GROUP BY results (#1556)
// =========================================================================

#[test]
fn test_group_by_with_offset_only() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category ORDER BY category ASC OFFSET 1",
    )
    .expect("test: GROUP BY OFFSET without LIMIT");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    let categories: Vec<&str> = groups
        .iter()
        .filter_map(|g| g.get("category")?.as_str())
        .collect();
    assert_eq!(
        categories,
        vec!["clothing", "electronics"],
        "OFFSET 1 without LIMIT should skip the first group and return the rest"
    );
}

// =========================================================================
// Scenario 20: LIMIT combined with HAVING on GROUP BY results (#1556)
// =========================================================================

#[test]
fn test_group_by_having_then_limit() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    // HAVING COUNT(*) >= 3 keeps all 3 categories (electronics=3, books=3,
    // clothing=4); LIMIT 1 must then truncate what HAVING already filtered,
    // never the other way around.
    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category HAVING COUNT(*) >= 3 ORDER BY category ASC LIMIT 1",
    )
    .expect("test: GROUP BY HAVING then LIMIT");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    assert_eq!(
        groups.len(),
        1,
        "LIMIT 1 should apply after HAVING's 3-group result, got {}",
        groups.len()
    );
    assert_eq!(
        groups[0].get("category").and_then(|v| v.as_str()),
        Some("books"),
        "HAVING should keep all 3 categories, then LIMIT 1 (ORDER BY category ASC) keeps 'books'"
    );
}

// =========================================================================
// Scenario 21: LIMIT 0 on GROUP BY returns empty (#1556)
// =========================================================================

#[test]
fn test_group_by_limit_zero_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category LIMIT 0",
    )
    .expect("test: GROUP BY LIMIT 0");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    assert!(
        groups.is_empty(),
        "LIMIT 0 on GROUP BY should return 0 groups, got {}",
        groups.len()
    );
}

// =========================================================================
// Scenario 22: OFFSET beyond group count returns empty (#1556)
// =========================================================================

#[test]
fn test_group_by_offset_beyond_group_count_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_orders_collection(&db);

    let result = execute_aggregate_sql(
        &db,
        "SELECT category, COUNT(*) FROM orders GROUP BY category OFFSET 100",
    )
    .expect("test: GROUP BY OFFSET beyond group count");

    let groups = result
        .as_array()
        .expect("test: aggregation should return array");
    assert!(
        groups.is_empty(),
        "OFFSET 100 on 3 groups should return empty, got {}",
        groups.len()
    );
}
