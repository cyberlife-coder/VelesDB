//! Internal unit tests for [`crate::velesql_graph`].
//!
//! Extracted to keep `velesql_graph.rs` under 500 NLOC. Integration-level
//! BDD tests live in `velesql_exec_graph_tests.rs`.

use velesdb_core::velesql::{
    DeleteEdgeStatement, DmlStatement, InsertEdgeStatement, InsertNodeStatement, Parser, Query,
    SelectEdgesStatement,
};

use crate::database::DatabaseInner;
use crate::velesql_graph::{delete_edge, execute_match, insert_edge, insert_node, select_edges};
use crate::velesql_value::Params;

fn parse_dml(sql: &str) -> DmlStatement {
    let q = Parser::parse(sql).expect("test: parse");
    q.dml.expect("test: has dml")
}

fn parse_match(sql: &str) -> Query {
    Parser::parse(sql).expect("test: parse")
}

#[test]
fn test_insert_node_creates_entry() {
    let mut db = DatabaseInner::new();
    let stmt = match parse_dml("INSERT NODE INTO kg (id = 42, payload = '{\"name\": \"Alice\"}')") {
        DmlStatement::InsertNode(s) => s,
        _ => panic!("test: expected InsertNode"),
    };
    insert_node(&mut db, &stmt).expect("test: insert");
    let store = db.get_graph_store("kg").expect("test: store");
    assert!(store.borrow().get_node(42).is_some());
}

#[test]
fn test_insert_edge_creates_entry() {
    let mut db = DatabaseInner::new();
    let stmt = match parse_dml("INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS')") {
        DmlStatement::InsertEdge(s) => s,
        _ => panic!("test: expected InsertEdge"),
    };
    insert_edge(&mut db, &stmt, &Params::new()).expect("test: insert edge");
    let store = db.get_graph_store("kg").expect("test: store");
    assert_eq!(store.borrow().edges().len(), 1);
}

#[test]
fn test_delete_edge_removes_entry() {
    let mut db = DatabaseInner::new();
    let ins = match parse_dml("INSERT EDGE INTO kg (source = 1, target = 2, label = 'KNOWS')") {
        DmlStatement::InsertEdge(s) => s,
        _ => panic!("test: expected InsertEdge"),
    };
    insert_edge(&mut db, &ins, &Params::new()).expect("test: insert");
    let del = DeleteEdgeStatement {
        collection: "kg".to_string(),
        edge_id: 1,
    };
    let n = delete_edge(&mut db, &del).expect("test: delete");
    assert_eq!(n, 1);
}

#[test]
fn test_select_edges_returns_filtered_list() {
    let mut db = DatabaseInner::new();
    for (s, t, l) in [(1u64, 2u64, "A"), (1, 3, "B"), (2, 3, "A")] {
        let e = InsertEdgeStatement {
            collection: "kg".to_string(),
            edge_id: None,
            source: s,
            target: t,
            label: l.to_string(),
            properties: Vec::new(),
        };
        insert_edge(&mut db, &e, &Params::new()).expect("test: insert");
    }
    let stmt = match parse_dml("SELECT EDGES FROM kg WHERE source = 1") {
        DmlStatement::SelectEdges(s) => s,
        _ => panic!("test: expected SelectEdges"),
    };
    let rows = select_edges(&db, &stmt, &Params::new()).expect("test: select edges");
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_match_1_hop_returns_pairs() {
    let mut db = DatabaseInner::new();
    for (id, name, labels) in [(1u64, "Alice", vec!["Person"]), (2, "Bob", vec!["Person"])] {
        let payload = serde_json::json!({"name": name, "labels": labels});
        let stmt = InsertNodeStatement {
            collection: "graph".to_string(),
            node_id: id,
            payload,
        };
        insert_node(&mut db, &stmt).expect("test: insert");
    }
    let edge = InsertEdgeStatement {
        collection: "graph".to_string(),
        edge_id: None,
        source: 1,
        target: 2,
        label: "KNOWS".to_string(),
        properties: Vec::new(),
    };
    insert_edge(&mut db, &edge, &Params::new()).expect("test: insert edge");
    let q = parse_match("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b LIMIT 10");
    let rows = execute_match(&mut db, &q, &Params::new()).expect("test: match");
    assert_eq!(rows.len(), 1);
}

#[test]
fn test_match_rejects_beyond_2_hop() {
    let mut db = DatabaseInner::new();
    let q =
        parse_match("MATCH (a:P)-[:R]->(b:P)-[:R]->(c:P)-[:R]->(d:P) RETURN a, b, c, d LIMIT 10");
    let err = execute_match(&mut db, &q, &Params::new());
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("more than 2 hops"));
}

#[test]
fn test_select_edges_on_missing_graph_errors() {
    let db = DatabaseInner::new();
    let stmt = SelectEdgesStatement {
        collection: "ghost".to_string(),
        where_clause: None,
        limit: None,
    };
    let err = select_edges(&db, &stmt, &Params::new());
    assert!(err.is_err());
}
