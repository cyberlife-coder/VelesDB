//! BDD integration tests for aggregations / GROUP BY / HAVING / DISTINCT /
//! ORDER BY in the WASM VelesQL executor (S4-13).
//!
//! Structure: nominal happy path (~60%), edge cases (~20%), negative (~20%+).

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

fn db_with_seed() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("products")
        .expect("test: create");
    let sql = concat!(
        "INSERT INTO products (id, category, price) VALUES ",
        "(1, 'tech', 100), ",
        "(2, 'tech', 50), ",
        "(3, 'food', 10), ",
        "(4, 'food', 30), ",
        "(5, 'home', 200)"
    );
    execute(&mut db, sql, None).expect("test: seed");
    db
}

// =========================================================================
// Aggregations — nominal
// =========================================================================

#[test]
fn test_count_star_returns_total_rows() {
    let mut db = db_with_seed();
    let r = execute(&mut db, "SELECT COUNT(*) FROM products", None).expect("test: count");
    assert_eq!(r.row_count(), 1);
    assert!(r.rows_json().contains("\"count(*)\":5"));
}

#[test]
fn test_sum_over_column() {
    let mut db = db_with_seed();
    let r = execute(&mut db, "SELECT SUM(price) FROM products", None).expect("test: sum");
    // 100 + 50 + 10 + 30 + 200 = 390
    assert!(r.rows_json().contains("\"sum(price)\":390"));
}

#[test]
fn test_avg_over_column() {
    let mut db = db_with_seed();
    let r = execute(&mut db, "SELECT AVG(price) FROM products", None).expect("test: avg");
    // 390 / 5 = 78
    assert!(r.rows_json().contains("\"avg(price)\":78"));
}

#[test]
fn test_min_over_column() {
    let mut db = db_with_seed();
    let r = execute(&mut db, "SELECT MIN(price) FROM products", None).expect("test: min");
    assert!(r.rows_json().contains("\"min(price)\":10"));
}

#[test]
fn test_max_over_column() {
    let mut db = db_with_seed();
    let r = execute(&mut db, "SELECT MAX(price) FROM products", None).expect("test: max");
    assert!(r.rows_json().contains("\"max(price)\":200"));
}

#[test]
fn test_count_alias_uses_alias_as_key() {
    let mut db = db_with_seed();
    let r = execute(&mut db, "SELECT COUNT(*) AS total FROM products", None).expect("test: alias");
    assert!(r.rows_json().contains("\"total\":5"));
}

// =========================================================================
// GROUP BY — nominal
// =========================================================================

#[test]
fn test_group_by_category_returns_one_row_per_group() {
    let mut db = db_with_seed();
    let r = execute(
        &mut db,
        "SELECT category, COUNT(*) AS n FROM products GROUP BY category",
        None,
    )
    .expect("test: group by");
    assert_eq!(r.row_count(), 3); // tech, food, home
}

#[test]
fn test_group_by_with_sum() {
    let mut db = db_with_seed();
    let r = execute(
        &mut db,
        "SELECT category, SUM(price) AS total FROM products GROUP BY category",
        None,
    )
    .expect("test: group sum");
    assert_eq!(r.row_count(), 3);
    assert!(r.rows_json().contains("\"total\":150")); // tech: 100+50
    assert!(r.rows_json().contains("\"total\":40")); // food: 10+30
    assert!(r.rows_json().contains("\"total\":200")); // home
}

// =========================================================================
// HAVING — nominal
// =========================================================================

#[test]
fn test_having_filters_groups_by_count() {
    let mut db = db_with_seed();
    let r = execute(
        &mut db,
        "SELECT category, COUNT(*) AS n FROM products GROUP BY category HAVING COUNT(*) > 1",
        None,
    )
    .expect("test: having count");
    // tech (2) and food (2) pass; home (1) fails.
    assert_eq!(r.row_count(), 2);
}

#[test]
fn test_having_with_sum() {
    let mut db = db_with_seed();
    let r = execute(
        &mut db,
        "SELECT category, SUM(price) AS total FROM products GROUP BY category HAVING SUM(price) >= 150",
        None,
    )
    .expect("test: having sum");
    // tech (150) and home (200) pass; food (40) fails.
    assert_eq!(r.row_count(), 2);
}

// =========================================================================
// DISTINCT — nominal
// =========================================================================

#[test]
fn test_distinct_on_column_dedups() {
    let mut db = db_with_seed();
    let r =
        execute(&mut db, "SELECT DISTINCT category FROM products", None).expect("test: distinct");
    assert_eq!(r.row_count(), 3);
}

#[test]
fn test_distinct_star_all_rows_distinct() {
    let mut db = db_with_seed();
    let r = execute(&mut db, "SELECT DISTINCT * FROM products LIMIT 100", None).expect("test: d*");
    assert_eq!(r.row_count(), 5);
}

// =========================================================================
// ORDER BY — nominal
// =========================================================================

#[test]
fn test_order_by_price_asc() {
    let mut db = db_with_seed();
    let r = execute(
        &mut db,
        "SELECT * FROM products ORDER BY price ASC LIMIT 10",
        None,
    )
    .expect("test: order asc");
    let first = r.row(0).expect("test: first");
    // price=10 is id=3
    assert_eq!(first.id(), 3);
}

#[test]
fn test_order_by_price_desc() {
    let mut db = db_with_seed();
    let r = execute(
        &mut db,
        "SELECT * FROM products ORDER BY price DESC LIMIT 10",
        None,
    )
    .expect("test: order desc");
    let first = r.row(0).expect("test: first");
    // price=200 is id=5
    assert_eq!(first.id(), 5);
}

#[test]
fn test_order_by_multi_key() {
    let mut db = db_with_seed();
    let r = execute(
        &mut db,
        "SELECT * FROM products ORDER BY category ASC, price DESC LIMIT 10",
        None,
    )
    .expect("test: multi order");
    // Categories alphabetical: food, home, tech. Within each, price DESC.
    // food: (4, 30) then (3, 10); home: (5, 200); tech: (1, 100) then (2, 50)
    let first = r.row(0).expect("test: first");
    assert_eq!(first.id(), 4);
}

// =========================================================================
// Edge cases
// =========================================================================

#[test]
fn test_count_on_empty_collection_returns_zero() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("empty")
        .expect("test: create");
    let r = execute(&mut db, "SELECT COUNT(*) FROM empty", None).expect("test: count");
    assert_eq!(r.row_count(), 1);
    assert!(r.rows_json().contains("\"count(*)\":0"));
}

#[test]
fn test_avg_on_empty_returns_null() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("empty")
        .expect("test: create");
    let r = execute(&mut db, "SELECT AVG(price) FROM empty", None).expect("test: avg empty");
    // Null encoded as JSON null.
    assert!(r.rows_json().contains("null"));
}

#[test]
fn test_order_by_nulls_last_in_asc() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("t").expect("test: create");
    execute(
        &mut db,
        "INSERT INTO t (id, n) VALUES (1, 5), (2, 1), (3, 10)",
        None,
    )
    .expect("test: seed");
    // Manually insert a null-n row.
    execute(&mut db, "INSERT INTO t (id, other) VALUES (4, 'x')", None).expect("test: null n");
    let r = execute(&mut db, "SELECT * FROM t ORDER BY n ASC LIMIT 10", None)
        .expect("test: nulls last");
    let last = r.row(3).expect("test: last");
    assert_eq!(last.id(), 4); // null sorts last in ASC
}

// =========================================================================
// Negative (≥ 20%)
// =========================================================================

#[test]
fn test_group_by_missing_collection_errors() {
    let mut db = DatabaseInner::new();
    let err = execute(
        &mut db,
        "SELECT category, COUNT(*) FROM ghost GROUP BY category",
        None,
    );
    assert!(err.is_err());
}

#[test]
fn test_having_with_invalid_column_yields_no_matches() {
    let mut db = db_with_seed();
    // HAVING on an aggregate over a non-existent column: sum of null == 0, so
    // the predicate "> 500" fails everywhere.
    let r = execute(
        &mut db,
        "SELECT category, COUNT(*) AS n FROM products GROUP BY category HAVING SUM(ghost_col) > 500",
        None,
    )
    .expect("test: having none");
    assert_eq!(r.row_count(), 0);
}

#[test]
fn test_distinct_with_param_unbound_errors() {
    let mut db = db_with_seed();
    let err = execute(
        &mut db,
        "SELECT DISTINCT category FROM products WHERE price > $t",
        Some("{}"),
    );
    assert!(err.is_err());
}

#[test]
fn test_sum_of_non_numeric_is_zero() {
    let mut db = db_with_seed();
    // category is a string — summing it yields 0 (no numeric values).
    let r = execute(&mut db, "SELECT SUM(category) AS s FROM products", None)
        .expect("test: sum strings");
    // serde_json may encode f64 as "0.0" or "-0.0"; accept any zero encoding.
    let body = r.rows_json();
    assert!(
        body.contains("\"s\":0") || body.contains("\"s\":-0.0") || body.contains("\"s\":0.0"),
        "got: {body}"
    );
}
