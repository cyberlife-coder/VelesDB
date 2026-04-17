//! BDD integration tests for JOIN in the WASM VelesQL executor (S4-13).

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

fn db_with_users_orders() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("users").expect("test: users");
    execute(
        &mut db,
        "INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')",
        None,
    )
    .expect("test: seed users");
    db.create_metadata_collection("orders")
        .expect("test: orders");
    execute(
        &mut db,
        "INSERT INTO orders (id, user_id, total) VALUES (10, 1, 50), (11, 1, 75), (12, 2, 20)",
        None,
    )
    .expect("test: seed orders");
    db
}

// =========================================================================
// INNER JOIN — nominal
// =========================================================================

#[test]
fn test_inner_join_pairs_matching_rows() {
    let mut db = db_with_users_orders();
    let r = execute(
        &mut db,
        "SELECT * FROM users JOIN orders ON users.id = orders.user_id LIMIT 10",
        None,
    )
    .expect("test: inner join");
    // Alice has 2 orders, Bob has 1, Carol has 0. INNER JOIN drops Carol.
    assert_eq!(r.row_count(), 3);
}

#[test]
fn test_inner_join_filters_via_where_on_left_column() {
    let mut db = db_with_users_orders();
    let r = execute(
        &mut db,
        "SELECT * FROM users JOIN orders ON users.id = orders.user_id WHERE name = 'Alice' LIMIT 10",
        None,
    )
    .expect("test: where on left");
    assert_eq!(r.row_count(), 2);
}

#[test]
fn test_inner_join_filters_via_where_on_right_column() {
    let mut db = db_with_users_orders();
    let r = execute(
        &mut db,
        "SELECT * FROM users JOIN orders ON users.id = orders.user_id WHERE total > 40 LIMIT 10",
        None,
    )
    .expect("test: where on right");
    assert_eq!(r.row_count(), 2); // orders 10 and 11
}

// =========================================================================
// LEFT JOIN — nominal
// =========================================================================

#[test]
fn test_left_join_keeps_unmatched_left_rows() {
    let mut db = db_with_users_orders();
    let r = execute(
        &mut db,
        "SELECT * FROM users LEFT JOIN orders ON users.id = orders.user_id LIMIT 10",
        None,
    )
    .expect("test: left join");
    // Alice 2 rows + Bob 1 + Carol null-padded = 4
    assert_eq!(r.row_count(), 4);
}

// =========================================================================
// Edge cases
// =========================================================================

#[test]
fn test_inner_join_between_empty_collections_returns_zero() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("a").expect("test: a");
    db.create_metadata_collection("b").expect("test: b");
    let r = execute(
        &mut db,
        "SELECT * FROM a JOIN b ON a.id = b.id LIMIT 10",
        None,
    )
    .expect("test: empty join");
    assert_eq!(r.row_count(), 0);
}

#[test]
fn test_inner_join_with_limit_caps_result() {
    let mut db = db_with_users_orders();
    let r = execute(
        &mut db,
        "SELECT * FROM users JOIN orders ON users.id = orders.user_id LIMIT 2",
        None,
    )
    .expect("test: limited");
    assert_eq!(r.row_count(), 2);
}

// =========================================================================
// Negative (≥ 20%)
// =========================================================================

#[test]
fn test_right_join_is_rejected() {
    let mut db = db_with_users_orders();
    let err = execute(
        &mut db,
        "SELECT * FROM users RIGHT JOIN orders ON users.id = orders.user_id LIMIT 10",
        None,
    );
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("RIGHT JOIN"));
}

#[test]
fn test_full_join_is_rejected() {
    let mut db = db_with_users_orders();
    let err = execute(
        &mut db,
        "SELECT * FROM users FULL JOIN orders ON users.id = orders.user_id LIMIT 10",
        None,
    );
    assert!(err.is_err());
}

#[test]
fn test_join_missing_left_collection_errors() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("orders").expect("test: o");
    let err = execute(
        &mut db,
        "SELECT * FROM ghost JOIN orders ON ghost.id = orders.user_id LIMIT 10",
        None,
    );
    assert!(err.is_err());
}

#[test]
fn test_join_missing_right_collection_errors() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("users").expect("test: u");
    let err = execute(
        &mut db,
        "SELECT * FROM users JOIN ghost ON users.id = ghost.user_id LIMIT 10",
        None,
    );
    assert!(err.is_err());
}
