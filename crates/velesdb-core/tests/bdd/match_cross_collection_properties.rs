//! BDD scenarios: a `@collection`-annotated MATCH node carries that
//! collection's properties everywhere the MATCH reads properties.
//!
//! `(w:Warehouse@pricing)` binds a graph node whose properties are the union
//! of its own and those of the `pricing` point with the same id — the node's
//! own fields winning a collision, so its identity and labels are never
//! overwritten. WHERE, ORDER BY, RETURN and the pattern's inline properties
//! all read that one union; before, only a post-hoc pass merged the
//! annotation into the returned rows, so `WHERE w.price < 100` matched
//! nothing and the annotated node's labels were never checked.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::json;
use velesdb_core::velesql::{MatchClause, Parser};
use velesdb_core::{
    AccessDecision, Database, DatabaseObserver, GraphEdge, Point, QueryAccessContext, SearchResult,
};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params};

/// `catalog` (graph): product 1 STORED_IN warehouses 2 (Paris) and 3 (Lyon).
/// `pricing` (metadata): 2 → price 99.99, stock 50, and a `name` the graph
/// node shadows; 3 → price 150, stock 0.
fn setup(db: &Database) {
    execute_sql(
        db,
        "CREATE GRAPH COLLECTION catalog (dimension = 4, metric = 'cosine') SCHEMALESS;",
    )
    .expect("test: CREATE catalog");
    let gc = db.get_graph_collection("catalog").expect("test: catalog");
    gc.upsert_node_payload(1, &json!({"_labels": ["Product"], "name": "Headphones"}))
        .expect("test: node 1");
    gc.upsert_node_payload(2, &json!({"_labels": ["Warehouse"], "name": "Paris"}))
        .expect("test: node 2");
    gc.upsert_node_payload(3, &json!({"_labels": ["Warehouse"], "name": "Lyon"}))
        .expect("test: node 3");
    for (id, target) in [(1, 2), (2, 3)] {
        gc.add_edge(GraphEdge::new(id, 1, target, "STORED_IN").expect("test: edge"))
            .expect("test: add edge");
    }

    execute_sql(db, "CREATE METADATA COLLECTION pricing;").expect("test: CREATE pricing");
    let pricing = db
        .get_metadata_collection("pricing")
        .expect("test: pricing");
    pricing
        .upsert(vec![
            Point::metadata_only(2, json!({"price": 99.99, "stock": 50, "name": "SHADOWED"})),
            Point::metadata_only(3, json!({"price": 150.0, "stock": 0})),
        ])
        .expect("test: upsert pricing");
}

fn on_catalog() -> HashMap<String, serde_json::Value> {
    HashMap::from([("_collection".to_string(), json!("catalog"))])
}

fn run(db: &Database, sql: &str) -> Vec<SearchResult> {
    execute_sql_with_params(db, sql, &on_catalog()).expect("test: MATCH")
}

/// The `w` binding of each row, in row order.
fn warehouses(rows: &[SearchResult]) -> Vec<u64> {
    rows.iter()
        .map(|r| {
            r.point.payload.as_ref().expect("test: payload")["_bindings"]["w"]
                .as_u64()
                .expect("test: w binding")
        })
        .collect()
}

fn field<'a>(row: &'a SearchResult, key: &str) -> Option<&'a serde_json::Value> {
    row.point.payload.as_ref()?.get(key)
}

/// GIVEN warehouses priced in `pricing`
/// WHEN WHERE filters on an annotated property
/// THEN only the warehouse the annotated collection prices below 100 matches.
#[test]
fn where_reads_the_annotated_collection() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let rows = run(
        &db,
        "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing) WHERE w.price < 100 RETURN w.name",
    );
    assert_eq!(warehouses(&rows), vec![2]);
}

/// GIVEN the same rows
/// WHEN ORDER BY sorts on an annotated property
/// THEN the order follows the annotated collection's values.
#[test]
fn order_by_reads_the_annotated_collection() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let rows = run(
        &db,
        "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing) RETURN w.name ORDER BY w.price DESC",
    );
    assert_eq!(warehouses(&rows), vec![3, 2]);
}

/// GIVEN a property both the graph node and the annotated point carry
/// WHEN RETURN projects it and an annotated-only property
/// THEN the node's own value wins the collision, the other comes from the
///      annotated collection.
#[test]
fn return_projects_the_union_with_the_node_winning() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let rows = run(
        &db,
        "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing) WHERE w.price < 100 \
         RETURN w.name, w.price",
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(field(&rows[0], "w.name"), Some(&json!("Paris")));
    assert_eq!(field(&rows[0], "w.price"), Some(&json!(99.99)));
}

/// GIVEN a pattern whose annotated node carries an inline property
/// WHEN the property lives in the annotated collection
/// THEN it filters like WHERE does.
#[test]
fn inline_pattern_properties_read_the_annotated_collection() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let rows = run(
        &db,
        "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing {stock: 0}) RETURN w.name",
    );
    assert_eq!(warehouses(&rows), vec![3]);
}

/// GIVEN annotated nodes labelled `Warehouse` in the graph
/// WHEN the pattern asks for another label on the annotated node
/// THEN nothing matches: an annotation adds properties, it does not waive
///      the node's label check.
#[test]
fn an_annotated_node_still_checks_its_labels() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let rows = run(
        &db,
        "MATCH (p:Product)-[:STORED_IN]->(w:Product@pricing) RETURN w.name",
    );
    assert!(warehouses(&rows).is_empty(), "{:?}", warehouses(&rows));
}

/// GIVEN a predicate mixing graph and annotated properties under NOT
/// WHEN the MATCH runs
/// THEN the three-valued result is the same as for any other property.
#[test]
fn negations_over_annotated_properties_are_three_valued() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let rows = run(
        &db,
        "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing) \
         WHERE NOT (w.price < 100 OR w.missing = 1) RETURN w.name",
    );
    // w=2: price < 100 is TRUE → NOT TRUE = FALSE. w=3: FALSE OR UNKNOWN =
    // UNKNOWN → excluded.
    let got: HashSet<u64> = warehouses(&rows).into_iter().collect();
    assert!(got.is_empty(), "{got:?}");
}

/// GIVEN the graph's range index on `Warehouse.stock`, which indexes its own
///      payloads — warehouse 4 carries `stock: 0` itself, warehouse 3 only
///      through `pricing`
/// WHEN WHERE filters the annotated alias on that property
/// THEN both match: the index cannot see the annotated collection, so it
///      must not narrow an annotated alias's candidates.
#[test]
fn an_index_on_the_graph_does_not_narrow_an_annotated_alias() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let gc = db.get_graph_collection("catalog").expect("test: catalog");
    gc.upsert_node_payload(
        4,
        &json!({"_labels": ["Warehouse"], "name": "Lille", "stock": 0}),
    )
    .expect("test: node 4");
    // A single-node pattern: the shape the index pre-filter narrows.
    let rows = run(
        &db,
        "MATCH (w:Warehouse@pricing) WHERE w.stock = 0 RETURN w.name",
    );
    let got: HashSet<u64> = warehouses(&rows).into_iter().collect();
    assert_eq!(got, HashSet::from([3, 4]));
}

fn clause(sql: &str) -> MatchClause {
    Parser::parse(sql)
        .expect("test: parse")
        .match_clause
        .expect("test: MATCH clause")
}

/// GIVEN the same graph and pricing
/// WHEN the MATCH arrives as a bare clause, as REST `/match` sends it
/// THEN it reads the annotated collection exactly as the `MATCH` statement
///      does: WHERE filters on it and RETURN projects it.
#[test]
fn the_match_api_reads_the_annotated_collection_like_the_statement() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let rows = db
        .match_query(
            "catalog",
            &clause(
                "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing) WHERE w.price < 100 \
                 RETURN w.name, w.price",
            ),
            &HashMap::new(),
            None,
        )
        .expect("test: match_query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].bindings.get("w"), Some(&2));
    assert_eq!(rows[0].projected.get("w.name"), Some(&json!("Paris")));
    assert_eq!(rows[0].projected.get("w.price"), Some(&json!(99.99)));
}

/// Denies every read of `pricing`.
struct DenyPricing;

impl DatabaseObserver for DenyPricing {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        Ok(if ctx.collection == "pricing" {
            AccessDecision::Deny(velesdb_core::Error::Query(
                "pricing denied by governance policy".to_string(),
            ))
        } else {
            AccessDecision::Allow
        })
    }
}

/// GIVEN a policy denying reads of the annotated collection
/// WHEN a MATCH annotates a node with it, as a statement or as a bare clause
/// THEN both are refused before anything is read: the annotation is a read
///      of that collection.
#[test]
fn a_denied_annotation_refuses_the_match_on_every_surface() {
    let dir = tempfile::tempdir().expect("test: dir");
    let db = Database::open_with_observer(dir.path(), Arc::new(DenyPricing)).expect("test: db");
    setup(&db);
    let sql = "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing) RETURN w.name";
    let statement =
        execute_sql_with_params(&db, sql, &on_catalog()).expect_err("test: statement refused");
    assert!(
        statement.to_string().contains("pricing denied"),
        "{statement}"
    );
    let bare = db
        .match_query("catalog", &clause(sql), &HashMap::new(), None)
        .expect_err("test: bare clause refused");
    assert!(bare.to_string().contains("pricing denied"), "{bare}");
}

/// GIVEN an annotation naming no collection
/// WHEN the MATCH runs
/// THEN it is refused: it cannot read properties from a collection that
///      does not exist.
#[test]
fn an_annotation_naming_no_collection_is_refused() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let err = execute_sql_with_params(
        &db,
        "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@nowhere) RETURN w.name",
        &on_catalog(),
    )
    .expect_err("test: refused");
    assert!(err.to_string().contains("nowhere"), "{err}");
}

/// GIVEN a SELECT whose WHERE tests a graph pattern
/// WHEN a node of that pattern carries a `@collection` annotation
/// THEN the query is refused: a predicate tests the FROM collection's own
///      graph and reads no other collection, so the annotation would
///      otherwise be silently ignored.
#[test]
fn an_annotation_in_a_graph_predicate_is_refused_not_ignored() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let err = execute_sql(
        &db,
        "SELECT * FROM catalog AS p \
         WHERE MATCH (p)-[:STORED_IN]->(w:Warehouse@pricing {stock: 0}) LIMIT 10",
    )
    .expect_err("test: refused");
    let msg = err.to_string();
    assert!(msg.contains("V011") && msg.contains("@pricing"), "{msg}");
}

/// GIVEN a `pricing` point that has expired but is not swept yet
/// WHEN a MATCH reads it through the annotation
/// THEN it is absent, as reading `pricing` directly answers: the graph node
///      keeps its own properties, the expired ones are not added.
#[test]
fn an_expired_annotated_point_adds_nothing() {
    let (_dir, db) = create_test_db();
    setup(&db);
    let pricing = db
        .get_metadata_collection("pricing")
        .expect("test: pricing");
    pricing
        .upsert(vec![Point::metadata_only(
            2,
            json!({"price": 99.99, velesdb_core::collection::EXPIRES_AT_KEY: 1}),
        )])
        .expect("test: expire point 2");
    assert!(
        pricing.get(&[2]).into_iter().flatten().next().is_none(),
        "test: pricing reads point 2 as absent"
    );
    let filtered = run(
        &db,
        "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing) WHERE w.price < 100 RETURN w.name",
    );
    assert!(
        warehouses(&filtered).is_empty(),
        "{:?}",
        warehouses(&filtered)
    );
    let all = run(
        &db,
        "MATCH (p:Product)-[:STORED_IN]->(w:Warehouse@pricing) RETURN w.name, w.price",
    );
    let paris = all
        .iter()
        .find(|r| field(r, "w.name") == Some(&json!("Paris")))
        .expect("test: the graph node stays");
    assert_eq!(field(paris, "w.price"), None);
}
