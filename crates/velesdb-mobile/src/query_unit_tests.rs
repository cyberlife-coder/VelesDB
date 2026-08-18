use super::*;

#[test]
fn test_classify_dml_insert() {
    let query =
        velesdb_core::velesql::Parser::parse("INSERT INTO docs (id, title) VALUES (1, 'hello')")
            .expect("test: parse INSERT");
    assert!(matches!(classify_query(&query), QueryResultKind::Mutation));
}

#[test]
fn test_classify_dml_delete() {
    let query = velesdb_core::velesql::Parser::parse("DELETE FROM docs WHERE id = 1")
        .expect("test: parse DELETE");
    assert!(matches!(classify_query(&query), QueryResultKind::Deletion));
}

#[test]
fn test_classify_ddl_create() {
    let query = velesdb_core::velesql::Parser::parse(
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine')",
    )
    .expect("test: parse CREATE");
    assert!(matches!(classify_query(&query), QueryResultKind::Ddl));
}

#[test]
fn test_classify_select() {
    let query = velesdb_core::velesql::Parser::parse("SELECT * FROM docs LIMIT 10")
        .expect("test: parse SELECT");
    assert!(matches!(classify_query(&query), QueryResultKind::Rows));
}

#[test]
fn test_classify_admin_flush() {
    let query = velesdb_core::velesql::Parser::parse("FLUSH FULL").expect("test: parse FLUSH");
    assert!(matches!(classify_query(&query), QueryResultKind::Admin));
}

#[test]
fn test_classify_introspection() {
    let query = velesdb_core::velesql::Parser::parse("SHOW COLLECTIONS").expect("test: parse SHOW");
    assert!(matches!(classify_query(&query), QueryResultKind::Rows));
}

#[test]
fn test_to_result_row_basic() {
    let sr =
        velesdb_core::SearchResult::new(velesdb_core::Point::new(42, vec![1.0, 2.0], None), 0.95);
    let row = to_result_row(&sr).expect("test: serialize row");
    assert_eq!(row.id, 42);
    assert!((row.score - 0.95).abs() < f32::EPSILON);
    assert!(row.data_json.contains("\"id\":42"));
}

#[test]
fn test_to_result_row_with_payload() {
    let payload = serde_json::json!({"title": "hello", "category": "test"});
    let sr =
        velesdb_core::SearchResult::new(velesdb_core::Point::new(1, vec![0.5], Some(payload)), 0.5);
    let row = to_result_row(&sr).expect("test: serialize row with payload");
    assert!(row.data_json.contains("\"title\":\"hello\""));
    assert!(row.data_json.contains("\"category\":\"test\""));
}

#[test]
fn test_parse_params_none() {
    let result = parse_params(None).expect("test: None params");
    assert!(result.is_empty());
}

#[test]
fn test_parse_params_valid_json() {
    let result = parse_params(Some(r#"{"k": 10}"#.to_string())).expect("test: valid params");
    assert_eq!(result.get("k"), Some(&serde_json::json!(10)));
}

#[test]
fn test_parse_params_invalid_json() {
    let result = parse_params(Some("not json".to_string()));
    assert!(result.is_err());
}

#[test]
fn test_build_message_rows() {
    let msg = build_message(&QueryResultKind::Rows, 5);
    assert_eq!(msg, "5 row(s) returned");
}

#[test]
fn test_build_message_mutation() {
    let msg = build_message(&QueryResultKind::Mutation, 3);
    assert_eq!(msg, "3 row(s) affected");
}

#[test]
fn test_build_message_ddl() {
    let msg = build_message(&QueryResultKind::Ddl, 0);
    assert_eq!(msg, "DDL statement executed successfully");
}
