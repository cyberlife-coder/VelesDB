#![cfg(feature = "persistence")]
//! End-to-end coverage for the graph metrics Prometheus exposition (#2091).
//!
//! `GraphMetrics` was updated on every edge write and traversal and read by
//! nothing: `to_prometheus` had no caller outside its own unit test, and the
//! server's `/metrics` handler assembled a different set of types entirely.
//! #2139 retired the per-operation clock reads that cost the most; what
//! remains — the atomic counters and the batch-fed histograms — is now
//! actually exported.
//!
//! These tests exercise the path the server takes, `Database ->
//! graph_metrics_prometheus`, rather than a `GraphMetrics` built by hand, so
//! they fail if the accessor chain breaks even while the formatter stays
//! correct.

use tempfile::TempDir;
use velesdb_core::collection::graph::{GraphEdge, GraphSchema};
use velesdb_core::Database;

/// A database with `names` as schemaless graph collections.
fn db_with_graphs(names: &[&str]) -> (TempDir, Database) {
    let dir = TempDir::new().expect("temp dir");
    let db = Database::open(dir.path()).expect("database");
    for name in names {
        db.create_graph_collection(name, GraphSchema::schemaless())
            .expect("create graph collection");
    }
    (dir, db)
}

/// The exposition names every graph collection and counts each one's edges.
///
/// This is the wiring the issue asked for: what the write path records has to
/// come back out under the collection it belongs to.
#[test]
fn the_exposition_reports_each_graph_collection_separately() {
    let (_dir, db) = db_with_graphs(&["alpha", "beta"]);

    let alpha = db.get_graph_collection("alpha").expect("alpha");
    let beta = db.get_graph_collection("beta").expect("beta");

    // Edges reference nodes, so the endpoints have to exist first.
    for id in 1..=3u64 {
        alpha
            .upsert_node_payload(id, &serde_json::json!({"n": id}))
            .expect("alpha node");
        beta.upsert_node_payload(id, &serde_json::json!({"n": id}))
            .expect("beta node");
    }

    alpha
        .add_edge(GraphEdge::new(1, 1, 2, "knows").expect("build edge"))
        .expect("edge");
    beta.add_edge(GraphEdge::new(2, 1, 2, "knows").expect("build edge"))
        .expect("edge");
    beta.add_edge(GraphEdge::new(3, 2, 3, "knows").expect("build edge"))
        .expect("edge");

    let output = db.graph_metrics_prometheus();

    assert!(
        output.contains("velesdb_graph_edges_total{collection=\"alpha\"} 1"),
        "alpha's single edge must be reported under its own name\n{output}"
    );
    assert!(
        output.contains("velesdb_graph_edges_total{collection=\"beta\"} 2"),
        "beta's two edges must be reported under its own name\n{output}"
    );
}

/// Each metric family is declared once, however many collections there are.
///
/// A per-collection block would repeat `# HELP`/`# TYPE` for a shared family
/// and publish two samples under the same label set — an exposition a scraper
/// rejects, and the specific hazard that made this a redesign rather than a
/// one-line call from the handler.
#[test]
fn families_are_declared_once_across_collections() {
    let (_dir, db) = db_with_graphs(&["one", "two", "three"]);

    let output = db.graph_metrics_prometheus();

    assert_eq!(
        output.matches("# HELP velesdb_graph_edges_total ").count(),
        1,
        "one HELP for the family regardless of collection count\n{output}"
    );
    assert_eq!(
        output.matches("# TYPE velesdb_graph_edges_total ").count(),
        1,
        "one TYPE for the family regardless of collection count\n{output}"
    );
    assert_eq!(
        output
            .matches("velesdb_graph_edges_total{collection=")
            .count(),
        3,
        "one sample per collection\n{output}"
    );
}

/// A database with no graph collection exports nothing at all.
///
/// Not an empty family set with a preamble: declaring families that carry no
/// sample reads on a dashboard as an idle graph rather than an absent one.
#[test]
fn a_database_without_graph_collections_exports_nothing() {
    let dir = TempDir::new().expect("temp dir");
    let db = Database::open(dir.path()).expect("database");

    assert!(db.graph_metrics_prometheus().is_empty());
}

/// Collections come out in a stable order.
///
/// The registry is a `HashMap`, so without an explicit sort the exposition
/// would reshuffle between scrapes. Nothing breaks in Prometheus, but every
/// diff of a captured scrape becomes noise.
#[test]
fn collections_are_emitted_in_a_stable_order() {
    let (_dir, db) = db_with_graphs(&["zulu", "alpha", "mike"]);

    let first = db.graph_metrics_prometheus();
    let second = db.graph_metrics_prometheus();
    assert_eq!(first, second, "repeated scrapes must agree");

    let alpha = first.find("collection=\"alpha\"").expect("alpha present");
    let mike = first.find("collection=\"mike\"").expect("mike present");
    let zulu = first.find("collection=\"zulu\"").expect("zulu present");
    assert!(
        alpha < mike && mike < zulu,
        "collections must be emitted sorted by name\n{first}"
    );
}
