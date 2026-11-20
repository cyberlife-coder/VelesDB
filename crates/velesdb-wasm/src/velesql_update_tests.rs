use super::*;
use crate::database::DatabaseInner;
use crate::velesql_value::parse_params;
use velesdb_core::velesql::{DmlStatement, Parser};

fn parse_update(sql: &str) -> UpdateStatement {
    let q = Parser::parse(sql).expect("test: parse");
    match q.dml.expect("test: has dml") {
        DmlStatement::Update(s) => s,
        other => panic!("expected UPDATE, got {other:?}"),
    }
}

fn seed_metadata_docs(db: &mut DatabaseInner) {
    db.create_metadata_collection("docs").expect("test: create");
    let store = db.get_shared_store("docs").expect("test: store");
    let mut borrowed = store.borrow_mut();
    borrowed.ids.push(1);
    borrowed
        .payloads
        .push(Some(serde_json::json!({"title": "first", "cat": "tech"})));
    borrowed.ids.push(2);
    borrowed
        .payloads
        .push(Some(serde_json::json!({"title": "second", "cat": "food"})));
}

#[test]
fn test_update_sets_field_on_match() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET title = 'renamed' WHERE id = 1");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: update");
    assert_eq!(n, 1);

    let store = db.get_shared_store("docs").expect("test: store");
    let borrowed = store.borrow();
    let p = borrowed.payloads[0].as_ref().expect("test: payload");
    assert_eq!(p["title"], "renamed");
}

#[test]
fn test_update_without_where_affects_all_rows() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET cat = 'x'");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: update all");
    assert_eq!(n, 2);
}

#[test]
fn test_update_on_id_column_is_rejected() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET id = 99 WHERE id = 1");
    let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
    assert!(err.is_err());
    assert!(err.expect_err("test: err").to_string().contains("'id'"));
}

#[test]
fn a_vector_that_is_not_an_array_is_refused() {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    let stmt = parse_update("UPDATE vecs SET vector = 'x' WHERE id = 1");
    let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
    assert!(err.is_err());
    assert!(err.expect_err("test: err").to_string().contains("'vector'"));
}

#[test]
fn test_update_missing_collection_errors() {
    let db = DatabaseInner::new();
    let stmt = parse_update("UPDATE ghost SET x = 1 WHERE id = 1");
    let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
    assert!(err.is_err());
}

#[test]
fn test_update_no_rows_match_returns_zero() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET title = 'z' WHERE id = 999");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: update");
    assert_eq!(n, 0);
}

#[test]
fn test_update_with_param() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_update("UPDATE docs SET cat = $new WHERE id = 1");
    let params = parse_params(Some(r#"{"new": "gaming"}"#)).expect("test: p");
    let n = execute(&db, &stmt, &params).expect("test: update");
    assert_eq!(n, 1);

    let store = db.get_shared_store("docs").expect("test: store");
    let borrowed = store.borrow();
    let p = borrowed.payloads[0].as_ref().expect("test: payload");
    assert_eq!(p["cat"], "gaming");
}

/// `SET vector = $v` replaces the embedding, as core's UPDATE does: the
/// row's NEAR rank follows its new vector.
#[test]
fn set_vector_replaces_the_embedding() {
    let mut db = DatabaseInner::new();
    db.create_collection("vecs", 2, "cosine")
        .expect("test: create");
    for (id, v) in [(1, "[1.0, 0.0]"), (2, "[0.0, 1.0]")] {
        crate::velesql_exec::execute(
            &mut db,
            &format!("INSERT INTO vecs (id, vector) VALUES ({id}, $v)"),
            Some(&format!("{{\"v\": {v}}}")),
        )
        .expect("test: seed");
    }
    crate::velesql_exec::execute(
        &mut db,
        "UPDATE vecs SET vector = $v WHERE id = 2",
        Some(r#"{"v": [1.0, 0.1]}"#),
    )
    .expect("test: update the vector");
    let r = crate::velesql_exec::execute(
        &mut db,
        "SELECT * FROM vecs WHERE vector NEAR $q LIMIT 2",
        Some(r#"{"q": [0.0, 1.0]}"#),
    )
    .expect("test: NEAR");
    assert_eq!(
        r.row(0).expect("test: row").id(),
        2,
        "[1, 0.1] is nearer [0, 1] than [1, 0]"
    );
}

#[test]
fn set_vector_is_refused_on_a_metadata_collection_and_at_another_dimension() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("m").expect("test: create m");
    db.create_collection("vecs", 2, "cosine")
        .expect("test: create vecs");
    for sql in [
        "INSERT INTO m (id, k) VALUES (1, 'x')",
        "INSERT INTO vecs (id, vector) VALUES (1, $v)",
    ] {
        crate::velesql_exec::execute(&mut db, sql, Some(r#"{"v": [1.0, 0.0]}"#))
            .expect("test: row 1");
    }
    let err = execute(
        &db,
        &parse_update("UPDATE m SET vector = $v WHERE id = 1"),
        &parse_params(Some(r#"{"v": [1.0]}"#)).expect("test: p"),
    )
    .expect_err("test: metadata-only");
    assert!(
        err.to_string()
            .contains("[VELES-014] Vector not allowed on 'm': this write stores no vector"),
        "{err}"
    );

    let err = execute(
        &db,
        &parse_update("UPDATE vecs SET vector = $v WHERE id = 1"),
        &parse_params(Some(r#"{"v": [1.0, 0.0, 0.0]}"#)).expect("test: p"),
    )
    .expect_err("test: dimension");
    assert!(err.to_string().contains("dimension mismatch"), "{err}");
}
