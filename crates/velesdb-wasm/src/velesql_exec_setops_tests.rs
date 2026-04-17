//! BDD integration tests for UNION / UNION ALL / INTERSECT / EXCEPT
//! in the WASM VelesQL executor (S4-13).

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

fn db_with_two_collections() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("a").expect("test: a");
    db.create_metadata_collection("b").expect("test: b");
    execute(
        &mut db,
        "INSERT INTO a (id, tag) VALUES (1, 'x'), (2, 'y'), (3, 'z')",
        None,
    )
    .expect("test: seed a");
    execute(
        &mut db,
        "INSERT INTO b (id, tag) VALUES (2, 'y'), (3, 'z'), (4, 'w')",
        None,
    )
    .expect("test: seed b");
    db
}

// =========================================================================
// UNION — nominal
// =========================================================================

#[test]
fn test_union_dedups_common_rows() {
    let mut db = db_with_two_collections();
    let r = execute(&mut db, "SELECT * FROM a UNION SELECT * FROM b", None).expect("test: union");
    assert_eq!(r.row_count(), 4); // {1,2,3,4} — 2 and 3 dedup'd
}

#[test]
fn test_union_all_keeps_duplicates() {
    let mut db = db_with_two_collections();
    let r = execute(&mut db, "SELECT * FROM a UNION ALL SELECT * FROM b", None)
        .expect("test: union all");
    assert_eq!(r.row_count(), 6);
}

// =========================================================================
// INTERSECT — nominal
// =========================================================================

#[test]
fn test_intersect_returns_common_only() {
    let mut db = db_with_two_collections();
    let r = execute(&mut db, "SELECT * FROM a INTERSECT SELECT * FROM b", None)
        .expect("test: intersect");
    assert_eq!(r.row_count(), 2); // {2, 3}
}

// =========================================================================
// EXCEPT — nominal
// =========================================================================

#[test]
fn test_except_subtracts_right_from_left() {
    let mut db = db_with_two_collections();
    let r = execute(&mut db, "SELECT * FROM a EXCEPT SELECT * FROM b", None).expect("test: except");
    assert_eq!(r.row_count(), 1); // {1}
    let first = r.row(0).expect("test: row");
    assert_eq!(first.id(), 1);
}

// =========================================================================
// Edge cases
// =========================================================================

#[test]
fn test_union_with_identical_queries_returns_original_rows() {
    let mut db = db_with_two_collections();
    let r = execute(&mut db, "SELECT * FROM a UNION SELECT * FROM a", None).expect("test: union a");
    assert_eq!(r.row_count(), 3);
}

#[test]
fn test_intersect_with_disjoint_sets_is_empty() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("a").expect("test: a");
    db.create_metadata_collection("b").expect("test: b");
    execute(&mut db, "INSERT INTO a (id) VALUES (1), (2)", None).expect("test: a");
    execute(&mut db, "INSERT INTO b (id) VALUES (100), (200)", None).expect("test: b");
    let r = execute(&mut db, "SELECT * FROM a INTERSECT SELECT * FROM b", None)
        .expect("test: disjoint intersect");
    assert_eq!(r.row_count(), 0);
}

// =========================================================================
// Negative (≥ 20%)
// =========================================================================

#[test]
fn test_union_with_missing_right_collection_errors() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("a").expect("test: a");
    let err = execute(&mut db, "SELECT * FROM a UNION SELECT * FROM ghost", None);
    assert!(err.is_err());
}

#[test]
fn test_except_with_missing_left_errors() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("b").expect("test: b");
    let err = execute(&mut db, "SELECT * FROM ghost EXCEPT SELECT * FROM b", None);
    assert!(err.is_err());
}

#[test]
fn test_intersect_unbound_param_errors() {
    let mut db = db_with_two_collections();
    let err = execute(
        &mut db,
        "SELECT * FROM a WHERE tag = $missing INTERSECT SELECT * FROM b",
        Some("{}"),
    );
    assert!(err.is_err());
}
