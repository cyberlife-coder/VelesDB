//! Tests for `batch` module - Batch operations for ColumnStore.

use super::{BatchUpdate, ColumnStore, ColumnType, ColumnValue, TypedColumn};

fn create_test_store() -> ColumnStore {
    let fields = [
        ("id", ColumnType::Int),
        ("value", ColumnType::Float),
        ("active", ColumnType::Bool),
    ];
    ColumnStore::with_primary_key(&fields, "id")
}

#[test]
fn test_batch_update_basic() {
    let mut store = create_test_store();
    store
        .insert_row(&[
            ("id", ColumnValue::Int(1)),
            ("value", ColumnValue::Float(1.0)),
        ])
        .unwrap();

    let updates = vec![BatchUpdate {
        pk: 1,
        column: "value".to_string(),
        value: ColumnValue::Float(2.0),
    }];

    let result = store.batch_update(&updates);
    assert_eq!(result.successful, 1);
    assert!(result.failed.is_empty());
}

#[test]
fn test_batch_update_row_not_found() {
    let mut store = create_test_store();
    let updates = vec![BatchUpdate {
        pk: 999,
        column: "value".to_string(),
        value: ColumnValue::Float(1.0),
    }];

    let result = store.batch_update(&updates);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed.len(), 1);
}

#[test]
fn test_batch_update_primary_key_rejected() {
    let mut store = create_test_store();
    store
        .insert_row(&[
            ("id", ColumnValue::Int(1)),
            ("value", ColumnValue::Float(1.0)),
        ])
        .unwrap();

    let updates = vec![BatchUpdate {
        pk: 1,
        column: "id".to_string(),
        value: ColumnValue::Int(2),
    }];

    let result = store.batch_update(&updates);
    assert_eq!(result.successful, 0);
    assert!(!result.failed.is_empty());
}

#[test]
fn test_batch_update_same_value() {
    let mut store = create_test_store();
    for i in 1..=3 {
        store
            .insert_row(&[
                ("id", ColumnValue::Int(i)),
                ("value", ColumnValue::Float(i as f64)),
            ])
            .unwrap();
    }

    let result = store.batch_update_same_value(&[1, 2, 3], "value", &ColumnValue::Float(99.0));
    assert_eq!(result.successful, 3);
}

#[test]
fn test_set_ttl_basic() {
    let mut store = create_test_store();
    store
        .insert_row(&[
            ("id", ColumnValue::Int(1)),
            ("value", ColumnValue::Float(1.0)),
        ])
        .unwrap();

    let result = store.set_ttl(1, 3600);
    assert!(result.is_ok());
}

#[test]
fn test_set_ttl_row_not_found() {
    let mut store = create_test_store();
    let result = store.set_ttl(999, 3600);
    assert!(result.is_err());
}

#[test]
fn test_expire_rows_empty() {
    let mut store = create_test_store();
    let result = store.expire_rows();
    assert_eq!(result.expired_count, 0);
}

#[test]
fn test_column_type_name() {
    assert_eq!(
        ColumnStore::column_type_name(&TypedColumn::Int(vec![])),
        "Int"
    );
    assert_eq!(
        ColumnStore::column_type_name(&TypedColumn::Float(vec![])),
        "Float"
    );
    assert_eq!(
        ColumnStore::column_type_name(&TypedColumn::String(vec![])),
        "String"
    );
    assert_eq!(
        ColumnStore::column_type_name(&TypedColumn::Bool(vec![])),
        "Bool"
    );
}

#[test]
fn test_value_type_name() {
    use super::types::StringId;
    assert_eq!(ColumnStore::value_type_name(&ColumnValue::Int(1)), "Int");
    assert_eq!(
        ColumnStore::value_type_name(&ColumnValue::Float(1.0)),
        "Float"
    );
    assert_eq!(
        ColumnStore::value_type_name(&ColumnValue::String(StringId(0))),
        "String"
    );
    assert_eq!(
        ColumnStore::value_type_name(&ColumnValue::Bool(true)),
        "Bool"
    );
    assert_eq!(ColumnStore::value_type_name(&ColumnValue::Null), "Null");
}

#[test]
fn test_now_timestamp() {
    let ts = ColumnStore::now_timestamp();
    assert!(ts > 1_577_836_800);
}

#[test]
fn test_batch_update_type_mismatch() {
    let mut store = create_test_store();
    store
        .insert_row(&[
            ("id", ColumnValue::Int(1)),
            ("value", ColumnValue::Float(1.0)),
        ])
        .unwrap();

    let updates = vec![BatchUpdate {
        pk: 1,
        column: "value".to_string(),
        value: ColumnValue::Int(42),
    }];

    let result = store.batch_update(&updates);
    assert_eq!(result.successful, 0);
    assert!(!result.failed.is_empty());
}

#[test]
fn test_batch_update_column_not_found() {
    let mut store = create_test_store();
    store
        .insert_row(&[
            ("id", ColumnValue::Int(1)),
            ("value", ColumnValue::Float(1.0)),
        ])
        .unwrap();

    let updates = vec![BatchUpdate {
        pk: 1,
        column: "nonexistent".to_string(),
        value: ColumnValue::Float(1.0),
    }];

    let result = store.batch_update(&updates);
    assert_eq!(result.successful, 0);
    assert!(!result.failed.is_empty());
}

#[test]
fn test_set_column_value_null() {
    let mut col = TypedColumn::Int(vec![Some(1), Some(2)]);
    let result = ColumnStore::set_column_value(&mut col, 0, ColumnValue::Null);
    assert!(result.is_ok());
    if let TypedColumn::Int(vec) = col {
        assert!(vec[0].is_none());
    }
}

#[test]
fn test_set_column_value_float() {
    let mut col = TypedColumn::Float(vec![Some(1.0), Some(2.0)]);
    let result = ColumnStore::set_column_value(&mut col, 0, ColumnValue::Float(3.5));
    assert!(result.is_ok());
}

#[test]
fn test_set_column_value_bool() {
    let mut col = TypedColumn::Bool(vec![Some(true), Some(false)]);
    let result = ColumnStore::set_column_value(&mut col, 1, ColumnValue::Bool(true));
    assert!(result.is_ok());
}

#[test]
fn test_set_column_value_out_of_bounds() {
    let mut col = TypedColumn::Int(vec![Some(1)]);
    let result = ColumnStore::set_column_value(&mut col, 10, ColumnValue::Int(5));
    assert!(result.is_err());
}

#[test]
fn test_set_column_value_null_float() {
    let mut col = TypedColumn::Float(vec![Some(1.0)]);
    let result = ColumnStore::set_column_value(&mut col, 0, ColumnValue::Null);
    assert!(result.is_ok());
}

#[test]
fn test_set_column_value_null_bool() {
    let mut col = TypedColumn::Bool(vec![Some(true)]);
    let result = ColumnStore::set_column_value(&mut col, 0, ColumnValue::Null);
    assert!(result.is_ok());
}

#[test]
fn test_set_column_value_null_string() {
    let mut col = TypedColumn::String(vec![None]);
    let result = ColumnStore::set_column_value(&mut col, 0, ColumnValue::Null);
    assert!(result.is_ok());
}
