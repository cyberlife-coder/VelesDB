//! BDD reject-conformance tests: locking documented-correct VelesQL contracts.
//!
//! Each scenario asserts that an intentionally-invalid statement is rejected
//! through the full `execute_sql` pipeline (parse -> validate -> execute) AND
//! that the surfaced error carries the documented marker substring. Markers
//! are verified against source:
//!   - V010 subquery code: `velesql/validation_types.rs:145` (+ Display embeds
//!     `[V010]`), gate `velesql/validation.rs:reject_subqueries`.
//!   - JOIN `USING(single_column)` / `requires primary key`:
//!     `collection/search/query/join.rs:196,210`.
//!   - `Graph expansion exceeded: max=32`: `velesql/validation.rs:289`
//!     enforced inside `Parser::parse` (`parser/mod.rs:128`), cap 32 from
//!     `validation_types.rs:DEFAULT_MAX_GRAPH_EXPANSION`.
//!   - DELETE / SELECT EDGES markers: `database/dml_executor.rs:199,208,274,291,364`.
//!
//! Scenarios skipped (no reliably-constructible SQL trigger) are documented in
//! the module-level review notes, not coded as flaky tests.

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params};

// =========================================================================
// Setup helpers
// =========================================================================

/// Creates a 4-dim vector `products` collection with two points.
fn setup_products(db: &Database) {
    db.create_vector_collection("products", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create products");
    let products = db
        .get_vector_collection("products")
        .expect("test: get products");
    products
        .upsert(vec![
            Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"price": 10.0}))),
            Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"price": 20.0}))),
        ])
        .expect("test: upsert products");
}

/// Creates a `products` vector collection JOINed against an `inventory`
/// metadata collection so the JOIN reaches `validate_join_condition`.
fn setup_join_collections(db: &Database) {
    setup_products(db);
    execute_sql(db, "CREATE METADATA COLLECTION inventory;").expect("test: create inventory");
    let inventory = db
        .get_metadata_collection("inventory")
        .expect("test: get inventory");
    inventory
        .upsert(vec![
            Point::metadata_only(1, json!({"product_id": 1, "stock": 5})),
            Point::metadata_only(2, json!({"product_id": 2, "stock": 0})),
        ])
        .expect("test: upsert inventory");
}

/// Creates a schemaless graph collection `kg` with two nodes and one edge.
fn setup_graph(db: &Database) {
    execute_sql(
        db,
        "CREATE GRAPH COLLECTION kg (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: create graph");
    execute_sql(
        db,
        "INSERT EDGE INTO kg (id = 10, source = 1, target = 2, label = 'KNOWS');",
    )
    .expect("test: insert edge");
}

// =========================================================================
// 1. WHERE scalar subquery -> V010 SubqueryNotExecutable
// =========================================================================

#[test]
fn scenario_where_scalar_subquery_rejected_with_v010() {
    let (_dir, db) = create_test_db();
    setup_products(&db);

    // The subquery parses but is not executable; validation must reject it
    // instead of silently evaluating the predicate to NULL.
    let err = execute_sql(
        &db,
        "SELECT * FROM products WHERE price > (SELECT AVG(price) FROM products) LIMIT 10",
    )
    .expect_err("test: WHERE scalar subquery must be rejected");
    assert!(
        err.to_string().contains("V010"),
        "expected V010 subquery code, got: {err}"
    );
}

// =========================================================================
// 2. Multi-column USING JOIN -> must use ON or USING(single_column)
// =========================================================================

#[test]
fn scenario_multi_column_using_join_rejected() {
    let (_dir, db) = create_test_db();
    setup_join_collections(&db);

    // USING(id, product_id) yields two join columns; only a single-column
    // USING (or an ON condition) is supported.
    let err = execute_sql(
        &db,
        "SELECT * FROM products JOIN inventory USING (id, product_id) LIMIT 10",
    )
    .expect_err("test: multi-column USING JOIN must be rejected");
    assert!(
        err.to_string().contains("USING(single_column)"),
        "expected single-column USING marker, got: {err}"
    );
}

// =========================================================================
// 3. Non-primary-key JOIN column -> requires primary key
// =========================================================================

#[test]
fn scenario_non_primary_key_join_column_rejected() {
    let (_dir, db) = create_test_db();
    setup_join_collections(&db);

    // The join column on the target side resolves to `product_id`, but the
    // built join ColumnStore is keyed on the primary key `id`.
    let err = execute_sql(
        &db,
        "SELECT * FROM products JOIN inventory ON products.id = inventory.product_id LIMIT 10",
    )
    .expect_err("test: non-primary-key JOIN column must be rejected");
    assert!(
        err.to_string().contains("requires primary key"),
        "expected primary-key requirement marker, got: {err}"
    );
}

// =========================================================================
// 4. Over-cap graph hop -> Graph expansion exceeded: max=32
// =========================================================================

#[test]
fn scenario_over_cap_graph_hop_rejected() {
    let (_dir, db) = create_test_db();

    // The documented cap is 32; an upper bound of 40 exceeds the budget and
    // is rejected inside Parser::parse (surfaced as a Query error).
    let err = execute_sql(&db, "MATCH (a)-[:R*1..40]->(b) RETURN b LIMIT 10")
        .expect_err("test: over-cap graph hop must be rejected");
    assert!(
        err.to_string().contains("Graph expansion exceeded: max=32"),
        "expected graph-expansion cap marker, got: {err}"
    );
}

// =========================================================================
// 5. DELETE rejects
// =========================================================================

#[test]
fn scenario_delete_non_id_where_column_rejected() {
    let (_dir, db) = create_test_db();
    setup_products(&db);

    // DELETE only supports `id = N` or `id IN (...)`; a payload column WHERE
    // must be rejected rather than silently matching nothing.
    let err = execute_sql(&db, "DELETE FROM products WHERE price = 10")
        .expect_err("test: DELETE on non-id column must be rejected");
    assert!(
        err.to_string().contains("DELETE WHERE must use 'id = N'"),
        "expected DELETE id-pattern marker, got: {err}"
    );
}

#[test]
fn scenario_delete_non_eq_operator_on_id_rejected() {
    let (_dir, db) = create_test_db();
    setup_products(&db);

    // `id` is the supported column, but only the `=` operator is allowed.
    let err = execute_sql(&db, "DELETE FROM products WHERE id > 0")
        .expect_err("test: DELETE id with non-= operator must be rejected");
    assert!(
        err.to_string()
            .contains("DELETE WHERE id must use '=' operator"),
        "expected DELETE '=' operator marker, got: {err}"
    );
}

// =========================================================================
// 7. SELECT EDGES rejects
// =========================================================================

#[test]
fn scenario_select_edges_unsupported_column_rejected() {
    let (_dir, db) = create_test_db();
    setup_graph(&db);

    // Only source / target / label are queryable on edges.
    let err = execute_sql(&db, "SELECT EDGES FROM kg WHERE weight = 5")
        .expect_err("test: SELECT EDGES on unsupported column must be rejected");
    assert!(
        err.to_string().contains("does not support column"),
        "expected unsupported-column marker, got: {err}"
    );
}

#[test]
fn scenario_select_edges_non_eq_operator_rejected() {
    let (_dir, db) = create_test_db();
    setup_graph(&db);

    // A supported column with a non-`=` operator must be rejected.
    let err = execute_sql(&db, "SELECT EDGES FROM kg WHERE source > 1")
        .expect_err("test: SELECT EDGES with non-= operator must be rejected");
    assert!(
        err.to_string().contains("only supports '=' operator"),
        "expected '=' operator marker, got: {err}"
    );
}

#[test]
fn scenario_select_edges_nested_and_rejected() {
    let (_dir, db) = create_test_db();
    setup_graph(&db);

    // A third AND term nests an `And` on the filter side, which is not a
    // simple comparison; only a single AND of two comparisons is supported.
    let err = execute_sql(
        &db,
        "SELECT EDGES FROM kg WHERE source = 1 AND target = 2 AND label = 'KNOWS'",
    )
    .expect_err("test: SELECT EDGES with >2 AND terms must be rejected");
    assert!(
        err.to_string()
            .contains("AND condition must be a simple comparison"),
        "expected nested-AND marker, got: {err}"
    );
}

// =========================================================================
// 8. Bare-MATCH RETURN ORDER BY of an unsupported expression -> VELES-018
//
// `order_match_results` (collection/search/query/match_exec/order_by.rs)
// supports ORDER BY similarity(), similarity(field, $v), depth, a valid
// `alias.property` path, and arithmetic over a bare property identifier;
// aggregates (no GROUP BY) and bare aliases are rejected (VELES-018).
// Any other expression that the parser accepts (e.g. a bare alias `d`) used
// to be silently dropped (tracing::warn + no reorder), returning rows in
// traversal order. It must now error so callers never get mis-ordered rows.
// =========================================================================

/// Creates a 2-dim vector collection `kgdocs` with three `Doc`-labeled nodes,
/// so a bare MATCH binds results and reaches the ORDER BY code path.
fn setup_doc_nodes(db: &Database) {
    db.create_vector_collection("kgdocs", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create kgdocs");
    let docs = db
        .get_vector_collection("kgdocs")
        .expect("test: get kgdocs");
    docs.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"_labels": ["Doc"], "name": "Charlie"})),
        ),
        Point::new(
            2,
            vec![1.0, 0.0],
            Some(json!({"_labels": ["Doc"], "name": "Alice"})),
        ),
        Point::new(
            3,
            vec![1.0, 0.0],
            Some(json!({"_labels": ["Doc"], "name": "Bob"})),
        ),
    ])
    .expect("test: upsert kgdocs");
}

#[test]
fn scenario_match_unsupported_order_by_expression_rejected() {
    let (_dir, db) = create_test_db();
    setup_doc_nodes(&db);

    // A bare alias `d` parses as an ORDER BY expression but is neither
    // similarity(), depth, nor a valid `alias.property` path, so it cannot be
    // applied. It must be rejected (VELES-018) rather than silently ignored.
    let mut params = std::collections::HashMap::new();
    params.insert("_collection".to_string(), json!("kgdocs"));
    let err = execute_sql_with_params(
        &db,
        "MATCH (d:Doc) RETURN d.name ORDER BY d LIMIT 3",
        &params,
    )
    .expect_err("test: unsupported MATCH ORDER BY expression must be rejected");
    assert!(
        err.to_string().contains("VELES-018"),
        "expected VELES-018 graph-not-supported code, got: {err}"
    );
    assert!(
        err.to_string().contains("ORDER BY"),
        "expected ORDER BY context in the message, got: {err}"
    );
}

#[test]
fn scenario_match_order_by_aggregate_rejected() {
    let (_dir, db) = create_test_db();
    setup_doc_nodes(&db);

    // An aggregate in a bare-MATCH ORDER BY has no GROUP BY semantics, so it is
    // rejected (VELES-018) rather than silently ignored — unlike arithmetic and
    // similarity(field, $v), which now sort.
    let mut params = std::collections::HashMap::new();
    params.insert("_collection".to_string(), json!("kgdocs"));
    let err = execute_sql_with_params(
        &db,
        "MATCH (d:Doc) RETURN d.name ORDER BY COUNT(*) DESC LIMIT 3",
        &params,
    )
    .expect_err("test: MATCH ORDER BY aggregate must be rejected");
    assert!(
        err.to_string().contains("VELES-018"),
        "expected VELES-018 graph-not-supported code, got: {err}"
    );
    assert!(
        err.to_string().contains("ORDER BY"),
        "expected ORDER BY context in the message, got: {err}"
    );
}

// NEAR_FUSED via SQL now EXECUTES real multi-vector fusion (it is no longer
// rejected); its execution + ranking contract lives in near_fused_parse_only.rs.
