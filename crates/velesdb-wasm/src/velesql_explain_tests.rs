//! Native-target tests for the WASM EXPLAIN plan emitter (backlog #23).
//!
//! Asserts the emitted `operation` vocabulary is a strict subset of core's
//! `PlanStep::rest_operation()` taxonomy and that the REST `ExplainStep` wire
//! keys (`step`, `operation`, `description`, `estimated_rows`) are present.

use super::*;
use velesdb_core::velesql::Parser;

/// The exact set of `operation` strings core's `PlanStep::rest_operation()`
/// can emit (`{Type}Join` expands per join type). Mirrored here because the
/// core `explain` module is `persistence`-gated and unreachable from WASM.
const CORE_OPERATIONS: &[&str] = &[
    "VectorSearch",
    "FullScan",
    "IndexLookup",
    "Filter",
    "InnerJoin",
    "LeftJoin",
    "RightJoin",
    "FullJoin",
    "GroupBy",
    "Aggregate",
    "Sort",
    "Limit",
    "Offset",
    "MatchTraversal",
];

fn rows_json(db: &DatabaseInner, sql: &str) -> Vec<serde_json::Value> {
    let q = Parser::parse(sql).expect("test: parse");
    explain(db, &q)
        .expect("test: explain")
        .iter()
        .map(|r| serde_json::from_str(r.data_json_ref()).expect("test: row json"))
        .collect()
}

fn assert_operations_in_taxonomy(rows: &[serde_json::Value]) {
    for row in rows {
        let op = row["operation"].as_str().expect("test: operation key");
        assert!(
            CORE_OPERATIONS.contains(&op),
            "operation '{op}' is not in the core rest_operation() taxonomy"
        );
        assert!(row["step"].is_number(), "row must carry a step number");
        assert!(
            row["description"].is_string(),
            "row must carry a description"
        );
    }
}

#[test]
fn select_where_limit_offset_uses_core_vocabulary() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("t").expect("test: create");
    let rows = rows_json(&db, "SELECT * FROM t WHERE x = 1 LIMIT 5 OFFSET 2");
    assert_operations_in_taxonomy(&rows);

    let ops: Vec<&str> = rows
        .iter()
        .map(|r| r["operation"].as_str().unwrap())
        .collect();
    assert_eq!(ops.first(), Some(&"FullScan"));
    assert!(ops.contains(&"Filter"));
    assert!(ops.contains(&"Limit"));
    // OFFSET is folded into the Limit step (mirrors core); no standalone Offset.
    assert!(!ops.contains(&"Offset"));
}

#[test]
fn scan_row_carries_estimated_rows() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("t").expect("test: create");
    let rows = rows_json(&db, "SELECT * FROM t WHERE x = 1 LIMIT 5");
    let scan = &rows[0];
    assert_eq!(scan["operation"], "FullScan");
    assert!(
        scan["estimated_rows"].is_number(),
        "scan row must carry estimated_rows, got: {scan}"
    );
    assert_eq!(scan["estimation_method"], "row count");
}

#[test]
fn near_query_emits_vector_search() {
    let mut db = DatabaseInner::new();
    db.create_collection("v", 4, "cosine")
        .expect("test: create");
    let rows = rows_json(&db, "SELECT * FROM v WHERE vector NEAR $q LIMIT 3");
    assert_eq!(rows[0]["operation"], "VectorSearch");
    assert_operations_in_taxonomy(&rows);
}

#[test]
fn group_by_emits_groupby_and_aggregate() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("t").expect("test: create");
    let rows = rows_json(&db, "SELECT cat, COUNT(*) FROM t GROUP BY cat");
    let ops: Vec<&str> = rows
        .iter()
        .map(|r| r["operation"].as_str().unwrap())
        .collect();
    assert!(ops.contains(&"GroupBy"));
    assert!(ops.contains(&"Aggregate"));
    assert_operations_in_taxonomy(&rows);
}

#[test]
fn join_emits_typed_join_operation() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("orders")
        .expect("test: create");
    db.create_metadata_collection("products")
        .expect("test: create");
    let rows = rows_json(
        &db,
        "SELECT * FROM orders JOIN products ON orders.product_id = products.id",
    );
    let ops: Vec<&str> = rows
        .iter()
        .map(|r| r["operation"].as_str().unwrap())
        .collect();
    assert!(
        ops.contains(&"InnerJoin"),
        "expected InnerJoin, got: {ops:?}"
    );
    assert_operations_in_taxonomy(&rows);
}

#[test]
fn match_query_emits_match_traversal() {
    let db = DatabaseInner::new();
    let rows = rows_json(&db, "MATCH (a:Person) RETURN a LIMIT 5");
    assert_eq!(rows[0]["operation"], "MatchTraversal");
    assert_operations_in_taxonomy(&rows);
}

#[test]
fn ddl_stays_within_taxonomy() {
    let db = DatabaseInner::new();
    let rows = rows_json(
        &db,
        "CREATE COLLECTION v (dimension = 4, metric = 'cosine')",
    );
    assert_eq!(rows.len(), 1);
    assert_operations_in_taxonomy(&rows);
}

#[test]
fn dml_insert_node_stays_within_taxonomy() {
    let db = DatabaseInner::new();
    let rows = rows_json(&db, "INSERT NODE INTO kg (id = 1, payload = '{}')");
    assert_operations_in_taxonomy(&rows);
    assert!(rows[0]["description"]
        .as_str()
        .unwrap()
        .contains("INSERT NODE"));
}
