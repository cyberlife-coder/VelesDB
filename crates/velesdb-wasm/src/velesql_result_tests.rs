use super::*;
use velesdb_core::velesql::Parser;

#[test]
fn test_classify_select() {
    let q = Parser::parse("SELECT * FROM docs LIMIT 10").expect("test: parse");
    assert_eq!(classify_query(&q), QueryResultKind::Rows);
}

#[test]
fn test_classify_insert() {
    let q = Parser::parse("INSERT INTO docs (id, t) VALUES (1, 'a')").expect("test: parse");
    assert_eq!(classify_query(&q), QueryResultKind::Mutation);
}

#[test]
fn test_classify_update() {
    let q = Parser::parse("UPDATE docs SET t = 'x' WHERE id = 1").expect("test: parse");
    assert_eq!(classify_query(&q), QueryResultKind::Mutation);
}

#[test]
fn test_classify_delete() {
    let q = Parser::parse("DELETE FROM docs WHERE id = 1").expect("test: parse");
    assert_eq!(classify_query(&q), QueryResultKind::Deletion);
}

#[test]
fn test_classify_ddl() {
    let q = Parser::parse("CREATE COLLECTION c (dimension = 4, metric = 'cosine')")
        .expect("test: parse");
    assert_eq!(classify_query(&q), QueryResultKind::Ddl);
}

#[test]
fn test_classify_admin_flush() {
    let q = Parser::parse("FLUSH FULL").expect("test: parse");
    assert_eq!(classify_query(&q), QueryResultKind::Admin);
}

#[test]
fn test_classify_show() {
    let q = Parser::parse("SHOW COLLECTIONS").expect("test: parse");
    assert_eq!(classify_query(&q), QueryResultKind::Rows);
}

#[test]
fn test_build_message_rows() {
    assert_eq!(build_message(QueryResultKind::Rows, 3), "3 row(s) returned");
}

#[test]
fn test_build_message_mutation() {
    assert_eq!(
        build_message(QueryResultKind::Mutation, 2),
        "2 row(s) affected"
    );
}

#[test]
fn test_build_message_deletion() {
    assert_eq!(
        build_message(QueryResultKind::Deletion, 1),
        "1 row(s) deleted"
    );
}

#[test]
fn test_build_message_ddl() {
    assert_eq!(
        build_message(QueryResultKind::Ddl, 0),
        "DDL statement executed successfully"
    );
}

#[test]
fn test_kind_as_str_is_stable() {
    assert_eq!(QueryResultKind::Rows.as_str(), "rows");
    assert_eq!(QueryResultKind::Mutation.as_str(), "mutation");
    assert_eq!(QueryResultKind::Deletion.as_str(), "deletion");
    assert_eq!(QueryResultKind::Ddl.as_str(), "ddl");
    assert_eq!(QueryResultKind::Train.as_str(), "train");
    assert_eq!(QueryResultKind::Admin.as_str(), "admin");
}

#[test]
fn test_row_build_without_payload() {
    let row = QueryResultRow::build(42, 0.95, None).expect("test: build");
    assert_eq!(row.id, 42);
    assert!((row.score - 0.95).abs() < f32::EPSILON);
    assert!(row.data_json.contains("\"id\":42"));
    assert!(row.data_json.contains("\"score\":"));
}

#[test]
fn test_row_build_with_payload_merges_top_level() {
    let payload = serde_json::json!({"title": "hello", "tag": "t"});
    let row = QueryResultRow::build(7, 0.5, Some(&payload)).expect("test: build");
    assert!(row.data_json.contains("\"title\":\"hello\""));
    assert!(row.data_json.contains("\"tag\":\"t\""));
    assert!(row.data_json.contains("\"id\":7"));
}

#[test]
fn test_row_build_with_payload_does_not_shadow_id_or_score() {
    // Payload keys that would conflict with id/score must be filtered out.
    let payload = serde_json::json!({"id": 999, "score": -1.0, "ok": true});
    let row = QueryResultRow::build(42, 0.3, Some(&payload)).expect("test: build");
    assert!(row.data_json.contains("\"id\":42"));
    assert!(!row.data_json.contains("\"id\":999"));
    assert!(row.data_json.contains("\"ok\":true"));
}

#[test]
fn test_from_parts_assembles_message_and_count() {
    let rows = vec![
        QueryResultRow::build(1, 0.0, None).expect("test: row 1"),
        QueryResultRow::build(2, 0.0, None).expect("test: row 2"),
    ];
    let result = QueryResult::from_parts(QueryResultKind::Rows, rows);
    assert_eq!(result.message, "2 row(s) returned");
    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_synthetic_row_preserves_data() {
    let row = QueryResultRow::synthetic(serde_json::json!({"name": "docs", "dim": 4}))
        .expect("test: synthetic");
    assert_eq!(row.id, 0);
    assert_eq!(row.score, 0.0);
    assert!(row.data_json.contains("\"name\":\"docs\""));
    assert!(row.data_json.contains("\"dim\":4"));
}
