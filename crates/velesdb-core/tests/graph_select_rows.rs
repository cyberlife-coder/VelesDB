#![cfg(feature = "persistence")]
//! A graph's SELECT rows are its stored nodes — an embedding, a payload or
//! both — the set `DESCRIBE`, `GraphCollection::len` and MATCH already
//! count. A graph with embeddings used to read only its embedded nodes on
//! the SELECT side (the scan hydrated through `Collection::get`, which needs
//! a vector), so `COUNT(*)`, `SELECT *` and a filter disagreed with MATCH on
//! the same graph.

use std::collections::HashMap;

use serde_json::{json, Value};
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::Database;

/// `kge`: a graph with 2-d embeddings holding two payload-only nodes (1, 2)
/// and one embedded node (3), nodes 1 → 2 linked. `kg`: a graph without
/// embeddings holding nodes 1 and 2, linked.
fn graphs() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    for sql in [
        "CREATE GRAPH COLLECTION kge (dimension = 2, metric = 'cosine')",
        "INSERT NODE INTO kge (id = 1, payload = '{\"name\":\"x\"}')",
        "INSERT NODE INTO kge (id = 2, payload = '{\"name\":\"y\"}')",
        "INSERT EDGE INTO kge (source = 1, target = 2, label = 'R')",
        "CREATE GRAPH COLLECTION kg",
        "INSERT NODE INTO kg (id = 1, payload = '{\"name\":\"x\"}')",
        "INSERT NODE INTO kg (id = 2, payload = '{\"name\":\"y\"}')",
        "INSERT EDGE INTO kg (source = 1, target = 2, label = 'R')",
    ] {
        run(&db, sql, &HashMap::new());
    }
    run(
        &db,
        "INSERT INTO kge (id, vector, name) VALUES (3, $v, 'z')",
        &HashMap::from([("v".to_string(), json!([1.0, 0.0]))]),
    );
    (dir, db)
}

fn run(
    db: &Database,
    sql: &str,
    params: &HashMap<String, Value>,
) -> Vec<velesdb_core::SearchResult> {
    let query = Parser::parse(sql).unwrap_or_else(|e| panic!("test: parse {sql}: {e}"));
    db.execute_query(&query, params)
        .unwrap_or_else(|e| panic!("test: {sql}: {e}"))
}

fn ids(db: &Database, sql: &str) -> Vec<u64> {
    let mut ids: Vec<u64> = run(db, sql, &HashMap::new())
        .iter()
        .map(|r| r.point.id)
        .collect();
    ids.sort_unstable();
    ids
}

fn count(db: &Database, sql: &str) -> Value {
    let query = Parser::parse(sql).expect("test: parse");
    db.execute_aggregate(&query, &HashMap::new())
        .expect("test: aggregate")["count"]
        .clone()
}

#[test]
fn count_star_counts_every_stored_node_of_a_graph() {
    let (_dir, db) = graphs();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM kge"), json!(3));
    assert_eq!(count(&db, "SELECT COUNT(*) FROM kg"), json!(2));
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM kge WHERE name = 'x'"),
        json!(1)
    );
}

#[test]
fn select_reads_every_stored_node_as_match_does() {
    let (_dir, db) = graphs();
    assert_eq!(ids(&db, "SELECT * FROM kge LIMIT 10"), vec![1, 2, 3]);
    let walked = run(
        &db,
        "MATCH (a) RETURN a",
        &HashMap::from([("_collection".to_string(), json!("kge"))]),
    );
    assert_eq!(walked.len(), 3, "MATCH walks the same three nodes");
}

#[test]
fn a_filter_reaches_a_payload_only_node() {
    let (_dir, db) = graphs();
    assert_eq!(ids(&db, "SELECT * FROM kge WHERE name = 'x'"), vec![1]);
    assert_eq!(ids(&db, "SELECT * FROM kge WHERE id IN (1, 3)"), vec![1, 3]);
    assert_eq!(ids(&db, "SELECT * FROM kge WHERE id = 2"), vec![2]);
}

/// ANALYZE's `graph_stats` (the MATCH planner's view) counts the nodes its
/// `row_count` counts: a graph without embeddings used to report
/// `total_nodes: 0` and `avg_degree: 0` beside `row_count: 2`.
#[test]
fn analyze_graph_stats_count_the_stored_nodes() {
    let (_dir, db) = graphs();
    for (graph, nodes) in [("kg", 2), ("kge", 3)] {
        let rows = run(&db, &format!("ANALYZE {graph}"), &HashMap::new());
        let stats = rows[0].point.payload.clone().expect("test: stats payload");
        assert_eq!(stats["row_count"], json!(nodes), "{graph}: {stats}");
        let graph_stats = &stats["graph_stats"];
        assert_eq!(graph_stats["total_nodes"], json!(nodes), "{graph}: {stats}");
        assert_eq!(graph_stats["total_edges"], json!(1), "{graph}: {stats}");
        let degree = graph_stats["avg_degree"].as_f64().expect("test: degree");
        assert!(
            (degree - 1.0 / f64::from(nodes)).abs() < 1e-9,
            "{graph}: {stats}"
        );
    }
}

fn q(vector: Value) -> HashMap<String, Value> {
    HashMap::from([("q".to_string(), vector)])
}

fn ids_with(db: &Database, sql: &str, params: &HashMap<String, Value>) -> Vec<u64> {
    let mut ids: Vec<u64> = run(db, sql, params).iter().map(|r| r.point.id).collect();
    ids.sort_unstable();
    ids
}

fn count_with(db: &Database, sql: &str, params: &HashMap<String, Value>) -> Value {
    let query = Parser::parse(sql).expect("test: parse");
    db.execute_aggregate(&query, params)
        .unwrap_or_else(|e| panic!("test: {sql}: {e}"))["count"]
        .clone()
}

/// A NEAR ranks the nodes that store an embedding: a payload-only node
/// cannot answer it, whichever path the planner picks. The metadata-first
/// path (`GraphFirst`) used to score every node the filter kept, a
/// payload-only one included, and the cosine kernel's length assertion
/// panicked on its empty vector.
#[test]
fn a_filtered_near_ranks_only_the_embedded_nodes() {
    let (_dir, db) = graphs();
    let params = q(json!([1.0, 0.0]));
    for (filter, expected) in [
        ("name = 'x'", vec![]),
        ("name IN ('x', 'z')", vec![3]),
        ("id = 1", vec![]),
        ("id IN (1, 3)", vec![3]),
        ("NOT name = 'q'", vec![3]),
    ] {
        let sql = format!("SELECT * FROM kge WHERE vector NEAR $q AND {filter} LIMIT 5");
        assert_eq!(ids_with(&db, &sql, &params), expected, "{sql}");
    }
}

/// A threshold under `OR` takes the exact scan, which reads every row: a
/// payload-only node the other branch holds for is a row, its own
/// `similarity()` UNKNOWN — as `COUNT(*)` already counted it.
#[test]
fn a_threshold_under_or_keeps_a_payload_only_node_the_other_branch_holds_for() {
    let (_dir, db) = graphs();
    let params = q(json!([1.0, 0.0]));
    let sql = "FROM kge WHERE name = 'x' OR similarity(vector, $q) > 0.5";
    assert_eq!(
        ids_with(&db, &format!("SELECT * {sql}"), &params),
        vec![1, 3]
    );
    assert_eq!(
        count_with(&db, &format!("SELECT COUNT(*) {sql}"), &params),
        json!(2)
    );
    // NOT UNKNOWN is UNKNOWN: a node without an embedding is no row of a
    // negated threshold, on either side.
    let negated = "FROM kge WHERE NOT similarity(vector, $q) > 0.5";
    assert_eq!(
        ids_with(&db, &format!("SELECT * {negated}"), &params),
        Vec::<u64>::new()
    );
    assert_eq!(
        count_with(&db, &format!("SELECT COUNT(*) {negated}"), &params),
        json!(0)
    );
}

/// A named-vector threshold reads the payload's vector, which a
/// payload-only node or a metadata row carries as well as an embedded one:
/// SELECT used to scan only the stored embeddings and found none of them,
/// while `COUNT(*)` counted them.
#[test]
fn a_named_vector_threshold_reads_rows_without_an_embedding() {
    let (_dir, db) = graphs();
    let emb = json!([1.0, 0.0, 0.0]);
    run(
        &db,
        "INSERT NODE INTO kge (id = 4, payload = '{\"name\":\"w\",\"emb\":[1.0,0.0,0.0]}')",
        &HashMap::new(),
    );
    run(
        &db,
        "INSERT NODE INTO kg (id = 4, payload = '{\"name\":\"w\",\"emb\":[1.0,0.0,0.0]}')",
        &HashMap::new(),
    );
    run(&db, "CREATE METADATA COLLECTION tags", &HashMap::new());
    for (id, vector) in [(1, json!([1.0, 0.0, 0.0])), (2, json!([0.0, 1.0, 0.0]))] {
        run(
            &db,
            &format!("INSERT INTO tags (id, emb) VALUES ({id}, $e)"),
            &HashMap::from([("e".to_string(), vector)]),
        );
    }
    let params = q(emb);
    for (collection, expected) in [("kge", vec![4]), ("kg", vec![4]), ("tags", vec![1])] {
        let sql = format!("FROM {collection} WHERE similarity(emb, $q) > 0.5");
        assert_eq!(
            ids_with(&db, &format!("SELECT * {sql}"), &params),
            expected,
            "{sql}"
        );
        assert_eq!(
            count_with(&db, &format!("SELECT COUNT(*) {sql}"), &params),
            json!(expected.len()),
            "{sql}"
        );
    }
    let either = "FROM tags WHERE similarity(emb, $q) > 0.5 OR id = 2";
    assert_eq!(
        ids_with(&db, &format!("SELECT * {either}"), &params),
        vec![1, 2]
    );
}

/// `docs` rows 5 and 6 point (`nid`) at node 3 (embedded) and node 1
/// (payload-only) of `kge`; row 7 points at nothing.
fn join_fixture() -> (TempDir, Database) {
    let (dir, db) = graphs();
    run(
        &db,
        "CREATE COLLECTION docs (dimension = 2)",
        &HashMap::new(),
    );
    for (id, nid) in [(5, 3), (6, 1), (7, 9)] {
        run(
            &db,
            &format!("INSERT INTO docs (id, vector, nid) VALUES ({id}, $v, {nid})"),
            &HashMap::from([("v".to_string(), json!([1.0, 0.0]))]),
        );
    }
    (dir, db)
}

/// A JOIN reads the graph's rows as SELECT does — a payload-only node
/// included — whichever side it is on: the joined side used to hydrate
/// through `Collection::get`, so the result depended on the join order.
#[test]
fn a_join_reads_payload_only_nodes_in_either_order() {
    let (_dir, db) = join_fixture();
    let joined = |sql: &str| {
        let mut pairs: Vec<(Value, Value)> = run(&db, sql, &HashMap::new())
            .iter()
            .map(|r| {
                let payload = r.point.payload.clone().expect("test: payload");
                (payload["docs"]["id"].clone(), payload["kge"]["id"].clone())
            })
            .collect();
        pairs.sort_by_key(|(d, _)| d.as_u64());
        pairs
    };
    let inner = vec![(json!(5), json!(3)), (json!(6), json!(1))];
    assert_eq!(
        joined("SELECT * FROM docs JOIN kge ON docs.nid = kge.id"),
        inner
    );
    assert_eq!(
        joined("SELECT * FROM kge JOIN docs ON kge.id = docs.nid"),
        inner
    );
    let mut left = inner.clone();
    left.push((json!(7), Value::Null));
    assert_eq!(
        joined("SELECT * FROM docs LEFT JOIN kge ON docs.nid = kge.id"),
        left
    );
    // ON docs.id = kge.id takes the primary-key lookup path.
    run(
        &db,
        "INSERT INTO docs (id, vector, nid) VALUES (1, $v, 0)",
        &HashMap::from([("v".to_string(), json!([1.0, 0.0]))]),
    );
    assert_eq!(
        joined("SELECT * FROM docs JOIN kge ON docs.id = kge.id"),
        vec![(json!(1), json!(1))]
    );
}

/// `ORDER BY similarity(vector, $q)` reads a row it cannot score — a
/// payload-only node — as a null key, which every ORDER BY sorts after every
/// value under `ASC` and before every value under `DESC`; such a row keeps
/// the score its plan gave it, never the metric's infinite worst (JSON
/// `null`), and no `vector_score`. It used to sort last under `DESC` and
/// first under `ASC` with a score of `-inf`.
#[test]
fn order_by_similarity_reads_an_unscorable_row_as_a_null_key() {
    let (_dir, db) = graphs();
    let params = q(json!([1.0, 0.0]));
    for (direction, expected) in [("DESC", vec![1, 2, 3]), ("ASC", vec![3, 1, 2])] {
        let rows = run(
            &db,
            &format!("SELECT * FROM kge ORDER BY similarity(vector, $q) {direction}"),
            &params,
        );
        let order: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
        assert_eq!(order, expected, "{direction}");
        for row in &rows {
            assert!(row.score.is_finite(), "{direction}: {row:?}");
            let tagged = row.component_scores.is_some();
            assert_eq!(tagged, row.point.id == 3, "{direction}: {row:?}");
        }
    }
}

/// A `NEAR` in an aggregate's WHERE reads the rows a `NEAR` can rank — those
/// storing a vector — and ranks none, so the aggregate still reads every
/// such row whatever its `LIMIT`: on a graph, a payload-only node no longer
/// counts. A vector collection's rows all store one.
#[test]
fn an_aggregate_with_near_reads_only_rows_storing_a_vector() {
    let (_dir, db) = graphs();
    let params = q(json!([1.0, 0.0]));
    assert_eq!(
        count_with(
            &db,
            "SELECT COUNT(*) FROM kge WHERE vector NEAR $q",
            &params
        ),
        json!(1)
    );
    assert_eq!(
        count_with(
            &db,
            "SELECT COUNT(*) FROM kge WHERE vector NEAR $q AND id > 0 LIMIT 5",
            &params
        ),
        json!(1)
    );
    run(
        &db,
        "CREATE COLLECTION docs (dimension = 2)",
        &HashMap::new(),
    );
    for (id, vector) in [(1, json!([1.0, 0.0])), (2, json!([0.0, 1.0]))] {
        run(
            &db,
            &format!("INSERT INTO docs (id, vector) VALUES ({id}, $v)"),
            &HashMap::from([("v".to_string(), vector)]),
        );
    }
    assert_eq!(
        count_with(
            &db,
            "SELECT COUNT(*) FROM docs WHERE vector NEAR $q LIMIT 1",
            &params
        ),
        json!(2)
    );
}

/// A MATCH `ORDER BY similarity(a.vector, $q)` reads a node without an
/// embedding as a null key too — first under `DESC`, last under `ASC` — as
/// every ORDER BY sorts a null: it sorted as least similar, the reverse.
#[test]
fn match_order_by_similarity_reads_a_node_without_an_embedding_as_a_null_key() {
    let (_dir, db) = graphs();
    let params = HashMap::from([
        ("_collection".to_string(), json!("kge")),
        ("q".to_string(), json!([1.0, 0.0])),
    ]);
    for (direction, expected) in [("DESC", vec![1, 2, 3]), ("ASC", vec![3, 1, 2])] {
        let rows = run(
            &db,
            &format!("MATCH (a) RETURN a ORDER BY similarity(a.vector, $q) {direction}"),
            &params,
        );
        let order: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
        assert_eq!(order, expected, "{direction}");
    }
}
