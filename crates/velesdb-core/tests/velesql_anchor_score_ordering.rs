//! The number of graph anchors, including unembedded nodes, selects the NEAR scorer.
#![cfg(feature = "persistence")]

use std::collections::HashMap;
use velesdb_core::{collection::GraphSchema, velesql::Parser, Database, DistanceMetric};

fn ranked(db: &Database, key: &str, n: usize, extra: &str) -> Vec<(u64, u32)> {
    let sql = format!("SELECT * FROM nodes AS d WHERE vector NEAR [1,0] AND MATCH (d) {extra} ORDER BY {key} LIMIT {n}");
    let query = Parser::parse(&sql).expect("select");
    db.execute_query(&query, &HashMap::new())
        .expect("ranking")
        .into_iter()
        .map(|hit| (hit.point.id, hit.score.to_bits()))
        .collect()
}

fn graph_at_exact_boundary() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().expect("database directory");
    let db = Database::open(dir.path()).expect("database");
    db.create_graph_collection_with_embeddings(
        "nodes",
        GraphSchema::schemaless(),
        2,
        DistanceMetric::Jaccard,
    )
    .expect("graph");
    let graph = db.get_graph_collection("nodes").expect("collection");
    let payload = serde_json::json!({"kind":"row"});
    let nodes: Vec<_> = (1..=10_000).map(|id| (id, &payload)).collect();
    graph
        .upsert_node_payloads(&nodes)
        .expect("anchors with and without a vector");
    for id in 1..=128 {
        let x = match id {
            1 => 1e-9,
            2 => 1e-8,
            _ => 0.0,
        };
        graph
            .upsert_node(id, &payload, Some(vec![x, 0.0]))
            .expect("embedding");
    }
    (dir, db)
}

#[test]
fn anchored_order_uses_the_actual_scorer_on_both_sides_of_ten_thousand() {
    let (_dir, db) = graph_at_exact_boundary();
    let graph = db.get_graph_collection("nodes").expect("collection");
    let payload = serde_json::json!({"kind":"row"});
    let tight = ranked(&db, "similarity() DESC", 3, "");
    let direct = [(2, 1e-8_f32.to_bits()), (1, 1e-9_f32.to_bits()), (3, 0)];
    assert_eq!(
        tight, direct,
        "the 10 000 anchors are scored on their stored vectors"
    );
    assert_eq!(ranked(&db, "similarity() DESC, id ASC", 3, ""), direct);
    graph
        .upsert_node_payloads(&[(10_001, &payload)])
        .expect("one additional, graph-only anchor");
    let native = ranked(&db, "similarity() DESC", 3, "");
    assert_eq!(
        native,
        [(1, 0), (2, 0), (3, 0)],
        "the graph rounds both tiny contributions to zero"
    );
    assert_eq!(ranked(&db, "similarity() DESC, id ASC", 3, ""), native);
    assert_eq!(
        ranked(&db, "similarity(vector,[1,0]) DESC", 1, ""),
        direct[..1]
    );
    let wide = u64::from(u32::MAX) + 17;
    graph
        .upsert_node(wide, &payload, Some(vec![5e-9, 0.0]))
        .expect("u64 anchor");
    assert_eq!(
        ranked(
            &db,
            "similarity() DESC, id ASC",
            1,
            &format!("AND id = {wide}")
        ),
        [(wide, 5e-9_f32.to_bits())],
        "an admitted u64 anchor outside the native bitmap keeps its original score"
    );
}
