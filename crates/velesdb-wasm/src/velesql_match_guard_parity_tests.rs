//! The MATCH walk's row bound in the WASM executor against a real core
//! `Database`: the same bow-tie graph and the same statement on both engines
//! must be refused with the same error past the bound, and answer with the
//! same rows within it.
//!
//! Host-only (`velesql_match.rs`): the core side needs core's `persistence`
//! feature.

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, GraphEdge, GraphSchema};

use crate::database::DatabaseInner;
use crate::velesql_graph::execute_match;
use crate::velesql_value::Params;

/// `(a:A)-[:R]->(h:H)-[:R]->(c:C)` over `fan` `A`s, one hub, `fan` `C`s.
const BOW_TIE: &str = "MATCH (a:A)-[:R]->(h:H)-[:R]->(c:C) RETURN a, c";

/// `(id, name, label)` of every node, hub first; then `(edge id, source,
/// target)` of every `R` edge.
type BowTie = (Vec<(u64, String, &'static str)>, Vec<(u64, u64, u64)>);

fn bow_tie(fan: u64) -> BowTie {
    let mut nodes = vec![(0, "n0".to_string(), "H")];
    let mut edges = Vec::new();
    for i in 1..=fan {
        nodes.push((i, format!("n{i}"), "A"));
        nodes.push((fan + i, format!("n{}", fan + i), "C"));
        edges.push((2 * i, i, 0));
        edges.push((2 * i + 1, 0, fan + i));
    }
    (nodes, edges)
}

fn wasm_db(fan: u64) -> DatabaseInner {
    let (nodes, edges) = bow_tie(fan);
    let mut db = DatabaseInner::new();
    for (id, name, label) in nodes {
        let stmt = velesdb_core::velesql::InsertNodeStatement {
            collection: "graph".to_string(),
            node_id: id,
            payload: json!({ "name": name, "labels": [label] }),
        };
        crate::velesql_graph::insert_node(&mut db, &stmt).expect("test: wasm node");
    }
    for (edge_id, source, target) in edges {
        let stmt = velesdb_core::velesql::InsertEdgeStatement {
            collection: "graph".to_string(),
            edge_id: Some(edge_id),
            source,
            target,
            label: "R".to_string(),
            properties: Vec::new(),
        };
        crate::velesql_graph::insert_edge(&mut db, &stmt, &Params::new()).expect("test: wasm edge");
    }
    db
}

fn core_db(fan: u64) -> (tempfile::TempDir, Database) {
    let (nodes, edges) = bow_tie(fan);
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let db = Database::open(dir.path()).expect("test: open");
    db.create_graph_collection("graph", GraphSchema::schemaless())
        .expect("test: create graph");
    let graph = db.get_graph_collection("graph").expect("test: graph");
    let payloads: Vec<(u64, serde_json::Value)> = nodes
        .into_iter()
        .map(|(id, name, label)| (id, json!({ "name": name, "_labels": [label] })))
        .collect();
    let entries: Vec<(u64, &serde_json::Value)> = payloads
        .iter()
        .map(|(id, payload)| (*id, payload))
        .collect();
    graph
        .upsert_node_payloads(&entries)
        .expect("test: core nodes");
    let edges = edges
        .into_iter()
        .map(|(edge_id, source, target)| {
            GraphEdge::new(edge_id, source, target, "R").expect("test: core edge")
        })
        .collect();
    graph.add_edges_batch(edges).expect("test: core edges");
    (dir, db)
}

/// The `(a, c)` node ids of every row, in result order.
type Ends = Vec<(u64, u64)>;

fn wasm_run(db: &mut DatabaseInner, sql: &str) -> Result<Ends, String> {
    let query = Parser::parse(sql).expect("test: parse");
    execute_match(db, &query, &Params::new())
        .map_err(|e| e.to_string())
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    let data: serde_json::Value =
                        serde_json::from_str(row.data_json_ref()).expect("test: row json");
                    let id = |alias: &str| data[alias]["id"].as_u64().expect("test: alias id");
                    (id("a"), id("c"))
                })
                .collect()
        })
}

fn core_run(db: &Database, sql: &str) -> Result<Ends, String> {
    let clause = Parser::parse(sql)
        .expect("test: parse")
        .match_clause
        .expect("test: a MATCH statement");
    db.match_query("graph", &clause, &HashMap::new(), None)
        .map(|rows| {
            rows.iter()
                .map(|row| (row.bindings["a"], row.bindings["c"]))
                .collect()
        })
        .map_err(|e| e.to_string())
}

/// 317² = 100 489 rows: an `ORDER BY` needs every one, so both engines
/// refuse at the first row past `DEFAULT_MAX_CARDINALITY`, word for word.
/// Core used to stop its walk at `MAX_LIMIT` and sort that truncated set.
#[test]
fn order_by_past_the_bound_is_refused_alike() {
    let fan = 317;
    let sql = format!("{BOW_TIE} ORDER BY c.name DESC LIMIT 3");
    let mut wasm = wasm_db(fan);
    let (_dir, core) = core_db(fan);

    let wasm_err = wasm_run(&mut wasm, &sql).expect_err("wasm must refuse");
    let core_err = core_run(&core, &sql).expect_err("core must refuse");

    assert_eq!(core_err, wasm_err);
    assert!(core_err.contains("VELES-027"), "{core_err}");
}

/// Within the bound both engines rank the whole walk and keep the same
/// top rows.
#[test]
fn order_by_within_the_bound_returns_the_same_rows() {
    let fan = 12;
    let sql = format!("{BOW_TIE} ORDER BY a.name DESC, c.name DESC LIMIT 5");
    let mut wasm = wasm_db(fan);
    let (_dir, core) = core_db(fan);

    let wasm_rows = wasm_run(&mut wasm, &sql).expect("test: wasm");
    let core_rows = core_run(&core, &sql).expect("test: core");

    assert_eq!(core_rows.len(), 5);
    assert_eq!(core_rows, wasm_rows);
}
