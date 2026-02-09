//! Native-compatible tests for WASM ColumnStore binding.
//!
//! Tests use core's `ColumnStore` directly since `JsValue`/`wasm_bindgen`
//! are not available on native targets. This validates the binding logic
//! operates correctly over core's API.

use velesdb_core::column_store::{ColumnStore, ColumnType, ColumnValue, VacuumConfig};

/// Helper: create a test schema with PK.
fn make_store() -> ColumnStore {
    ColumnStore::with_primary_key(
        &[
            ("id", ColumnType::Int),
            ("name", ColumnType::String),
            ("age", ColumnType::Int),
            ("score", ColumnType::Float),
            ("active", ColumnType::Bool),
        ],
        "id",
    )
    .unwrap()
}

/// Helper: insert a row with string interning.
fn insert_row(store: &mut ColumnStore, id: i64, name: &str, age: i64, score: f64, active: bool) {
    let name_id = store.string_table_mut().intern(name);
    store
        .insert_row(&[
            ("id", ColumnValue::Int(id)),
            ("name", ColumnValue::String(name_id)),
            ("age", ColumnValue::Int(age)),
            ("score", ColumnValue::Float(score)),
            ("active", ColumnValue::Bool(active)),
        ])
        .unwrap();
}

#[test]
fn test_create_schema_and_insert() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);
    assert_eq!(store.row_count(), 1);
    assert_eq!(store.active_row_count(), 1);
}

#[test]
fn test_primary_key_upsert() {
    let mut store = make_store();

    let name_id = store.string_table_mut().intern("Alice");
    store
        .upsert(&[
            ("id", ColumnValue::Int(1)),
            ("name", ColumnValue::String(name_id)),
            ("age", ColumnValue::Int(30)),
        ])
        .unwrap();

    let name_id2 = store.string_table_mut().intern("Alice Updated");
    let result = store
        .upsert(&[
            ("id", ColumnValue::Int(1)),
            ("name", ColumnValue::String(name_id2)),
            ("age", ColumnValue::Int(31)),
        ])
        .unwrap();

    assert!(matches!(
        result,
        velesdb_core::column_store::UpsertResult::Updated
    ));
    assert_eq!(store.active_row_count(), 1);
}

#[test]
fn test_get_row_by_pk() {
    let mut store = make_store();
    insert_row(&mut store, 42, "Bob", 25, 88.0, false);

    assert!(store.get_row_idx_by_pk(42).is_some());
    assert!(store.get_row_idx_by_pk(999).is_none());
}

#[test]
fn test_filter_eq_int() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);
    insert_row(&mut store, 2, "Bob", 25, 88.0, false);
    insert_row(&mut store, 3, "Charlie", 30, 77.0, true);

    let indices = store.filter_eq_int("age", 30);
    assert_eq!(indices.len(), 2);
}

#[test]
fn test_filter_eq_string() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);
    insert_row(&mut store, 2, "Bob", 25, 88.0, false);

    let indices = store.filter_eq_string("name", "Alice");
    assert_eq!(indices.len(), 1);
    assert_eq!(indices[0], 0);
}

#[test]
fn test_filter_range() {
    let mut store = make_store();
    insert_row(&mut store, 1, "A", 20, 0.0, true);
    insert_row(&mut store, 2, "B", 30, 0.0, true);
    insert_row(&mut store, 3, "C", 40, 0.0, true);
    insert_row(&mut store, 4, "D", 50, 0.0, true);

    // Range: 25 < age < 45 → B(30) and C(40)
    let indices = store.filter_range_int("age", 25, 45);
    assert_eq!(indices.len(), 2);
}

#[test]
fn test_filter_in_string() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);
    insert_row(&mut store, 2, "Bob", 25, 88.0, false);
    insert_row(&mut store, 3, "Charlie", 35, 77.0, true);

    let indices = store.filter_in_string("name", &["Alice", "Charlie"]);
    assert_eq!(indices.len(), 2);
}

#[test]
fn test_delete_and_vacuum() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);
    insert_row(&mut store, 2, "Bob", 25, 88.0, false);
    insert_row(&mut store, 3, "Charlie", 35, 77.0, true);

    assert!(store.delete_by_pk(2));
    assert_eq!(store.active_row_count(), 2);
    assert_eq!(store.deleted_row_count(), 1);

    // Deleted row not found by PK
    assert!(store.get_row_idx_by_pk(2).is_none());

    // Deleted row excluded from filters
    let indices = store.filter_eq_int("age", 25);
    assert!(indices.is_empty());

    // Vacuum compacts
    assert!(store.should_vacuum(0.2)); // 1/3 = 33% > 20%
    let stats = store.vacuum(VacuumConfig::default());
    assert!(stats.completed);
    assert_eq!(stats.tombstones_removed, 1);
    assert_eq!(store.active_row_count(), 2);
    assert_eq!(store.deleted_row_count(), 0);
}

#[test]
fn test_batch_upsert() {
    let mut store = make_store();

    let n1 = store.string_table_mut().intern("Alice");
    let n2 = store.string_table_mut().intern("Bob");
    let n3 = store.string_table_mut().intern("Charlie");

    let rows = vec![
        vec![
            ("id", ColumnValue::Int(1)),
            ("name", ColumnValue::String(n1)),
            ("age", ColumnValue::Int(30)),
        ],
        vec![
            ("id", ColumnValue::Int(2)),
            ("name", ColumnValue::String(n2)),
            ("age", ColumnValue::Int(25)),
        ],
        vec![
            ("id", ColumnValue::Int(3)),
            ("name", ColumnValue::String(n3)),
            ("age", ColumnValue::Int(35)),
        ],
    ];

    let result = store.batch_upsert(&rows);
    assert_eq!(result.inserted, 3);
    assert!(result.failed.is_empty());
    assert_eq!(store.active_row_count(), 3);
}

#[test]
fn test_update_row() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);

    store.update_by_pk(1, "age", ColumnValue::Int(31)).unwrap();

    let indices = store.filter_eq_int("age", 31);
    assert_eq!(indices.len(), 1);

    // Old value no longer matches
    let old = store.filter_eq_int("age", 30);
    assert!(old.is_empty());
}

#[test]
fn test_clear_via_rebuild() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);
    insert_row(&mut store, 2, "Bob", 25, 88.0, false);
    assert_eq!(store.active_row_count(), 2);

    // Simulate clear: rebuild from same schema
    store = make_store();
    assert_eq!(store.active_row_count(), 0);
    assert_eq!(store.row_count(), 0);
}

#[test]
fn test_column_names() {
    let store = make_store();
    let names: Vec<&str> = store.column_names().collect();
    assert_eq!(names.len(), 5);
    assert!(names.contains(&"id"));
    assert!(names.contains(&"name"));
    assert!(names.contains(&"age"));
}

#[test]
fn test_filter_gt_lt() {
    let mut store = make_store();
    insert_row(&mut store, 1, "A", 20, 0.0, true);
    insert_row(&mut store, 2, "B", 30, 0.0, true);
    insert_row(&mut store, 3, "C", 40, 0.0, true);

    let gt = store.filter_gt_int("age", 25);
    assert_eq!(gt.len(), 2); // B(30), C(40)

    let lt = store.filter_lt_int("age", 35);
    assert_eq!(lt.len(), 2); // A(20), B(30)
}

#[test]
fn test_ttl_set_and_expire() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);
    insert_row(&mut store, 2, "Bob", 25, 88.0, false);

    // Set TTL of 0 seconds (immediately expired)
    store.set_ttl(1, 0).unwrap();

    // Wait a tiny bit then expire
    std::thread::sleep(std::time::Duration::from_millis(10));
    let result = store.expire_rows();
    assert_eq!(result.expired_count, 1);
    assert_eq!(store.active_row_count(), 1);

    // Alice should be gone
    assert!(store.get_row_idx_by_pk(1).is_none());
    // Bob still alive
    assert!(store.get_row_idx_by_pk(2).is_some());
}

#[test]
fn test_get_value_as_json() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);

    let val = store.get_value_as_json("age", 0);
    assert_eq!(val, Some(serde_json::json!(30)));

    let name_val = store.get_value_as_json("name", 0);
    assert_eq!(name_val, Some(serde_json::json!("Alice")));

    let score_val = store.get_value_as_json("score", 0);
    assert_eq!(score_val, Some(serde_json::json!(95.5)));
}

#[test]
fn test_duplicate_key_error() {
    let mut store = make_store();
    insert_row(&mut store, 1, "Alice", 30, 95.5, true);

    let name_id = store.string_table_mut().intern("Bob");
    let result = store.insert_row(&[
        ("id", ColumnValue::Int(1)),
        ("name", ColumnValue::String(name_id)),
        ("age", ColumnValue::Int(25)),
    ]);
    assert!(result.is_err());
}
