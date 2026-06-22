//! BDD tests pinning bare-MATCH `ORDER BY` over ARITHMETIC and explicit
//! `similarity(field, $v)` expressions EXACTLY.
//!
//! These forms used to be silently dropped / rejected (VELES-018); they now
//! sort. Verified against source: the structured `OrderByExpr` is carried into
//! the MATCH AST (graph_pattern.rs `OrderByItem.expr`) and evaluated in
//! `match_exec/order_by.rs` by reusing `ordering::evaluate_arithmetic`
//! (arithmetic) and `extraction::resolve_vector` + the configured metric
//! (similarity). Aggregates without GROUP BY stay rejected (see
//! `velesql_reject_conformance.rs`).
//!
//! Dataset (`odocs`, 2-dim cosine, all `:Doc`):
//!   id 1: year 2005, vector [1.0, 0.0]   (low year, closest to [1,0])
//!   id 2: year 2020, vector [0.0, 1.0]   (high year, farthest from [1,0])
//!   id 3: year 2015, vector [0.7, 0.7]   (mid year, mid similarity)

use std::collections::HashMap;

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql_with_params};

/// Builds the fixed `odocs` collection described in the module doc-comment.
fn setup_order_docs(db: &Database) {
    db.create_vector_collection("odocs", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create odocs");
    let vc = db.get_vector_collection("odocs").expect("test: get odocs");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"_labels": ["Doc"], "name": "A", "year": 2005})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0],
            Some(json!({"_labels": ["Doc"], "name": "B", "year": 2020})),
        ),
        Point::new(
            3,
            vec![0.7, 0.7],
            Some(json!({"_labels": ["Doc"], "name": "C", "year": 2015})),
        ),
    ])
    .expect("test: upsert odocs");
}

/// Routes a bare-MATCH query to `odocs`, optionally binding `$v`.
fn run_ids(db: &Database, sql: &str, v: Option<&[f32]>) -> Vec<u64> {
    let mut params = HashMap::new();
    params.insert("_collection".to_string(), json!("odocs"));
    if let Some(v) = v {
        params.insert("v".to_string(), json!(v));
    }
    execute_sql_with_params(db, sql, &params)
        .expect("test: execute MATCH ORDER BY")
        .iter()
        .map(|r| r.point.id)
        .collect()
}

#[test]
fn scenario_match_order_by_arithmetic_property_sorts() {
    let (_dir, db) = create_test_db();
    setup_order_docs(&db);

    // `year - 2000` DESC => 2020(id2), 2015(id3), 2005(id1). The bound node's
    // `year` resolves against its payload via the reused arithmetic evaluator.
    let ids = run_ids(
        &db,
        "MATCH (d:Doc) RETURN d.name ORDER BY year - 2000 DESC LIMIT 10",
        None,
    );
    assert_eq!(ids, vec![2u64, 3, 1], "ORDER BY year - 2000 DESC");
}

#[test]
fn scenario_match_order_by_similarity_field_vec_sorts() {
    let (_dir, db) = create_test_db();
    setup_order_docs(&db);

    // similarity(embedding, [1,0]) DESC => id1 (1.0), id3 (~0.707), id2 (0.0).
    // `embedding` is a BARE field leaf: with one vector per node it scores the
    // matched node's vector (the leaf name is not used to pick among vectors).
    // An alias-qualified `b.embedding` would instead resolve the node bound to
    // `b` via the result bindings (see sort_match_by_similarity).
    let ids = run_ids(
        &db,
        "MATCH (d:Doc) RETURN d.name ORDER BY similarity(embedding, $v) DESC LIMIT 10",
        Some(&[1.0, 0.0]),
    );
    assert_eq!(
        ids,
        vec![1u64, 3, 2],
        "ORDER BY similarity(embedding, $v) DESC"
    );
}

#[test]
fn scenario_match_order_by_arithmetic_asc_reverses() {
    let (_dir, db) = create_test_db();
    setup_order_docs(&db);

    // ASC must reverse the DESC order, proving apply_direction is honored.
    let ids = run_ids(
        &db,
        "MATCH (d:Doc) RETURN d.name ORDER BY year - 2000 ASC LIMIT 10",
        None,
    );
    assert_eq!(ids, vec![1u64, 3, 2], "ORDER BY year - 2000 ASC");
}

#[test]
fn scenario_match_order_by_arithmetic_missing_property_uses_zero_fallback() {
    // Finding 12: arithmetic over a property MISSING on some nodes must use the
    // 0.0 fallback (ordering.rs resolve_payload_variable map_or(0.0, ..)) and still
    // produce a deterministic TOTAL order with a node_id tie-break — never a panic.
    let (_dir, db) = create_test_db();
    setup_order_docs(&db); // ids 1 (2005), 2 (2020), 3 (2015) all carry `year`.
    let vc = db.get_vector_collection("odocs").expect("test: get odocs");
    // ids 4 & 5 LACK `year` => year resolves to 0.0 => key -2000, sort last; they
    // tie at -2000 and must break by node_id ascending (4 before 5).
    vc.upsert(vec![
        Point::new(
            4,
            vec![0.3, 0.9],
            Some(json!({"_labels": ["Doc"], "name": "D"})),
        ),
        Point::new(
            5,
            vec![0.9, 0.3],
            Some(json!({"_labels": ["Doc"], "name": "E"})),
        ),
    ])
    .expect("test: upsert missing-year docs");

    let mut params = HashMap::new();
    params.insert("_collection".to_string(), json!("odocs"));
    let ids: Vec<u64> = execute_sql_with_params(
        &db,
        "MATCH (d:Doc) RETURN d.name ORDER BY year - 2000 DESC LIMIT 10",
        &params,
    )
    .expect("test: execute MATCH ORDER BY arithmetic with missing property")
    .iter()
    .map(|r| r.point.id)
    .collect();

    // year-2000 DESC: id2 (20) > id3 (15) > id1 (5) > [id4, id5 both -2000].
    // The two fallback rows tie at -2000 and break by node_id ascending (the
    // deterministic baseline in finalize_match_results), so 4 precedes 5.
    assert_eq!(
        ids,
        vec![2u64, 3, 1, 4, 5],
        "missing `year` => 0.0 fallback (key -2000), total order with node_id tie-break"
    );
}

#[test]
fn scenario_match_order_by_similarity_field_vec_sorts_euclidean() {
    // On a DISTANCE metric (Euclidean: lower = more similar), `similarity(...) DESC`
    // must still be most-similar-first — the metric direction is honored.
    let (_dir, db) = create_test_db();
    db.create_vector_collection("edist", 2, velesdb_core::DistanceMetric::Euclidean)
        .expect("test: create edist");
    let vc = db.get_vector_collection("edist").expect("test: get edist");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"_labels": ["Doc"], "name": "A"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0],
            Some(json!({"_labels": ["Doc"], "name": "B"})),
        ),
        Point::new(
            3,
            vec![0.7, 0.7],
            Some(json!({"_labels": ["Doc"], "name": "C"})),
        ),
    ])
    .expect("test: upsert edist");

    // Distances to [1,0]: id1=0, id3≈0.76, id2≈1.41 → DESC (most similar first)
    // = id1, id3, id2. The pre-fix raw-distance sort would invert this.
    let mut params = HashMap::new();
    params.insert("_collection".to_string(), json!("edist"));
    params.insert("v".to_string(), json!([1.0_f32, 0.0]));
    let ids: Vec<u64> = execute_sql_with_params(
        &db,
        "MATCH (d:Doc) RETURN d.name ORDER BY similarity(embedding, $v) DESC LIMIT 10",
        &params,
    )
    .expect("test: execute Euclidean similarity ORDER BY")
    .iter()
    .map(|r| r.point.id)
    .collect();
    assert_eq!(
        ids,
        vec![1u64, 3, 2],
        "Euclidean similarity DESC must be most-similar-first"
    );
}
