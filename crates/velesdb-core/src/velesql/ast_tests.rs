//! Tests for `ast` module

use super::ast::*;

#[test]
fn test_column_new() {
    let col = Column::new("id");
    assert_eq!(col.name, "id");
    assert!(col.alias.is_none());
}

#[test]
fn test_column_with_alias() {
    let col = Column::with_alias("payload.title", "title");
    assert_eq!(col.name, "payload.title");
    assert_eq!(col.alias, Some("title".to_string()));
}

#[test]
fn test_value_from_integer() {
    let v: Value = 42i64.into();
    assert_eq!(v, Value::Integer(42));
}

#[test]
fn test_value_from_float() {
    let v: Value = 2.5f64.into();
    assert_eq!(v, Value::Float(2.5));
}

#[test]
fn test_value_from_string() {
    let v: Value = "hello".into();
    assert_eq!(v, Value::String("hello".to_string()));
}

#[test]
fn test_value_from_bool() {
    let v: Value = true.into();
    assert_eq!(v, Value::Boolean(true));
}

#[test]
fn test_query_serialization() {
    let query = Query {
        compound: None,
        select: SelectStatement {
            columns: SelectColumns::All,
            from: "documents".to_string(),
            joins: vec![],
            where_clause: None,
            order_by: None,
            limit: Some(10),
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        },
    };

    let json = serde_json::to_string(&query).unwrap();
    let parsed: Query = serde_json::from_str(&json).unwrap();
    assert_eq!(query, parsed);
}
