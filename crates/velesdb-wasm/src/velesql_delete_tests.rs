use super::*;
use crate::database::DatabaseInner;
use crate::velesql_value::parse_params;
use velesdb_core::velesql::{DmlStatement, Parser};

fn parse_delete(sql: &str) -> DeleteStatement {
    let q = Parser::parse(sql).expect("test: parse");
    match q.dml.expect("test: has dml") {
        DmlStatement::Delete(s) => s,
        other => panic!("expected DELETE, got {other:?}"),
    }
}

fn seed_metadata_docs(db: &mut DatabaseInner) {
    db.create_metadata_collection("docs").expect("test: create");
    let store = db.get_shared_store("docs").expect("test: store");
    let mut borrowed = store.borrow_mut();
    for (id, cat) in [(1u64, "tech"), (2, "food"), (3, "tech")] {
        borrowed.ids.push(id);
        borrowed
            .payloads
            .push(Some(serde_json::json!({"cat": cat})));
    }
}

#[test]
fn test_delete_by_id() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_delete("DELETE FROM docs WHERE id = 2");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: delete");
    assert_eq!(n, 1);

    let store = db.get_shared_store("docs").expect("test: store");
    let borrowed = store.borrow();
    assert!(!borrowed.ids.contains(&2));
    assert_eq!(borrowed.ids.len(), 2);
}

#[test]
fn test_delete_by_payload_field() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_delete("DELETE FROM docs WHERE cat = 'tech'");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: delete");
    assert_eq!(n, 2);

    let store = db.get_shared_store("docs").expect("test: store");
    assert_eq!(store.borrow().ids.len(), 1);
}

#[test]
fn test_delete_no_match_returns_zero() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_delete("DELETE FROM docs WHERE id = 999");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: delete");
    assert_eq!(n, 0);
}

#[test]
fn test_delete_missing_collection_errors() {
    let db = DatabaseInner::new();
    let stmt = parse_delete("DELETE FROM ghost WHERE id = 1");
    let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
    assert!(err.is_err());
}

#[test]
fn test_delete_preserves_other_rows_indices() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    let stmt = parse_delete("DELETE FROM docs WHERE id IN (1, 3)");
    let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: delete");
    assert_eq!(n, 2);

    let store = db.get_shared_store("docs").expect("test: store");
    let borrowed = store.borrow();
    assert_eq!(borrowed.ids, vec![2]);
}

/// A VelesQL DELETE cascades to the deleted points' edges, as core does.
#[test]
fn a_delete_removes_the_edges_of_the_points_it_deletes() {
    let mut db = DatabaseInner::new();
    seed_metadata_docs(&mut db);
    {
        let graph = db.graph_store("docs");
        let mut graph = graph.borrow_mut();
        graph
            .insert_edge(Some(10), 1, 2, "R".into(), None)
            .expect("edge");
        graph
            .insert_edge(Some(11), 2, 3, "R".into(), None)
            .expect("edge");
        graph
            .insert_edge(Some(12), 3, 1, "R".into(), None)
            .expect("edge");
    }
    let stmt = parse_delete("DELETE FROM docs WHERE id = 2");
    execute(&db, &stmt, &parse_params(None).expect("test: params")).expect("test: delete");

    let left: Vec<u64> = db
        .get_graph_store("docs")
        .expect("graph")
        .borrow()
        .edges()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(left, [12]);
}
