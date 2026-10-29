//! Native coverage tests for under-exercised 3.0.0 mobile binding paths.
//!
//! These exercise the uniffi-exported methods directly as plain Rust:
//! - `MobileGraphStore::dfs_traverse` (visited dedup, path tracking, depth/limit
//!   cutoffs) — core's DFS, reached through the binding.
//! - `VelesCollection::stream_insert` not-configured error branch.
//! - `parse_point` invalid-JSON error branch (via `upsert`).
//! - `MobileCollectionDiagnostics` `From` `NeedsRebuild` arm.

use tempfile::TempDir;
use velesdb_mobile::{
    DistanceMetric, MobileCollectionDiagnostics, MobileGraphEdge, MobileGraphNode,
    MobileGraphStore, VelesDatabase, VelesPoint,
};

/// Builds a graph store with nodes 1..=`count` and the given KNOWS edges.
///
/// Each edge tuple is `(edge_id, source, target)`. The store is a graph
/// collection of a database in the returned temp dir, which must outlive it.
fn graph_with(
    count: u64,
    edges: &[(u64, u64, u64)],
) -> (TempDir, std::sync::Arc<MobileGraphStore>) {
    let tmp = TempDir::new().expect("tempdir");
    let db =
        VelesDatabase::open(tmp.path().to_str().expect("path utf8").to_string()).expect("open db");
    db.create_graph_collection("kg".to_string())
        .expect("create graph collection");
    let store = db
        .get_graph_store("kg".to_string())
        .expect("get graph store")
        .expect("graph store present");
    for id in 1..=count {
        store
            .add_node(MobileGraphNode {
                id,
                label: "Person".to_string(),
                properties_json: None,
                vector: None,
            })
            .expect("node insert should succeed");
    }
    for &(id, source, target) in edges {
        store
            .add_edge(MobileGraphEdge {
                id,
                source,
                target,
                label: "KNOWS".to_string(),
                properties_json: None,
            })
            .expect("edge insert should succeed");
    }
    (tmp, store)
}

#[test]
fn dfs_traverse_chain_tracks_depth_and_path() {
    // Chain 1 -> 2 -> 3 -> 4 via edges 100, 101, 102.
    let (_tmp, store) = graph_with(4, &[(100, 1, 2), (101, 2, 3), (102, 3, 4)]);

    let results = store.dfs_traverse(1, 3, 100).expect("dfs");

    // Source itself is not emitted (depth 0 skipped); 2/3/4 each carry the
    // accumulated edge-ID path mirroring core's TraversalResult::path.
    assert_eq!(results.len(), 3);
    assert!(results
        .iter()
        .any(|r| r.node_id == 2 && r.depth == 1 && r.path == vec![100]));
    assert!(results
        .iter()
        .any(|r| r.node_id == 3 && r.depth == 2 && r.path == vec![100, 101]));
    assert!(results
        .iter()
        .any(|r| r.node_id == 4 && r.depth == 3 && r.path == vec![100, 101, 102]));
}

#[test]
fn dfs_traverse_respects_max_depth() {
    // Chain 1 -> 2 -> 3 -> 4, but cap depth at 2: node 4 must not be reached.
    let (_tmp, store) = graph_with(4, &[(100, 1, 2), (101, 2, 3), (102, 3, 4)]);

    let results = store.dfs_traverse(1, 2, 100).expect("dfs");

    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.depth <= 2));
    assert!(results.iter().all(|r| r.node_id != 4));
}

#[test]
fn dfs_traverse_respects_limit() {
    // Star: 1 -> {2,3,4,5}. Limit to 2 results.
    let (_tmp, store) = graph_with(5, &[(100, 1, 2), (101, 1, 3), (102, 1, 4), (103, 1, 5)]);

    let results = store.dfs_traverse(1, 1, 2).expect("dfs");

    assert_eq!(results.len(), 2);
}

#[test]
fn dfs_traverse_dedups_visited_in_diamond() {
    // Diamond: 1 -> 2, 1 -> 3, 2 -> 4, 3 -> 4. Node 4 has two paths in but
    // core's DFS visits each node once, so it is emitted exactly once.
    let (_tmp, store) = graph_with(4, &[(100, 1, 2), (101, 1, 3), (102, 2, 4), (103, 3, 4)]);

    let results = store.dfs_traverse(1, 3, 100).expect("dfs");

    let node4_hits = results.iter().filter(|r| r.node_id == 4).count();
    assert_eq!(node4_hits, 1, "node 4 must be visited exactly once");
    // 2, 3 and 4 reachable; source 1 excluded.
    let mut reached: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    reached.sort_unstable();
    assert_eq!(reached, vec![2, 3, 4]);
}

#[test]
fn dfs_traverse_isolated_source_yields_nothing() {
    // Source with no outgoing edges: there is nothing to expand, so the
    // result set is empty.
    let (_tmp, store) = graph_with(3, &[(100, 2, 3)]);

    let results = store.dfs_traverse(1, 3, 100).expect("dfs");

    assert!(results.is_empty());
}

#[test]
fn stream_insert_without_enable_returns_error() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().to_str().expect("path utf8").to_string();

    let db = VelesDatabase::open(path).expect("open db");
    db.create_collection("no_stream".to_string(), 4, DistanceMetric::Cosine)
        .expect("create collection");
    let col = db
        .get_collection("no_stream".to_string())
        .expect("get collection")
        .expect("collection present");

    // No enable_streaming(): the not-configured branch must surface as an error
    // with the binding's "buffer full or not configured" message.
    let result = col.stream_insert(vec![VelesPoint {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
    }]);

    let err = result.expect_err("stream_insert without enable must fail");
    let velesdb_mobile::VelesError::Database { message, .. } = err else {
        panic!("expected VelesError::Database, got {err:?}");
    };
    assert!(
        message.contains("not configured") || message.contains("Stream insert failed"),
        "unexpected error message: {message}"
    );
}

#[test]
fn upsert_with_invalid_json_payload_errors() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().to_str().expect("path utf8").to_string();

    let db = VelesDatabase::open(path).expect("open db");
    db.create_collection("bad_payload".to_string(), 4, DistanceMetric::Cosine)
        .expect("create collection");
    let col = db
        .get_collection("bad_payload".to_string())
        .expect("get collection")
        .expect("collection present");

    // Malformed JSON in the payload must hit parse_point's error branch.
    let result = col.upsert(VelesPoint {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: Some("{not valid json".to_string()),
    });

    let err = result.expect_err("invalid JSON payload must fail");
    let velesdb_mobile::VelesError::Database { message, .. } = err else {
        panic!("expected VelesError::Database, got {err:?}");
    };
    assert!(
        message.contains("Invalid JSON payload"),
        "unexpected error message: {message}"
    );
    // The bad upsert must not have committed a point.
    assert_eq!(col.count(), 0);
}

#[test]
fn diagnostics_maps_needs_rebuild_branch() {
    // The core from_collection() path only ever yields Empty/Healthy, so the
    // NeedsRebuild arm of the mobile From impl is exercised by constructing the
    // core diagnostics directly with that health state.
    let core = velesdb_core::collection::CollectionDiagnostics {
        has_vectors: true,
        search_ready: false,
        dimension_configured: true,
        point_count: 7,
        index_health: velesdb_core::collection::IndexHealth::NeedsRebuild(
            "schema changed".to_string(),
        ),
    };

    let mobile: MobileCollectionDiagnostics = core.into();

    assert_eq!(mobile.index_health, "needs_rebuild");
    assert_eq!(
        mobile.index_health_detail,
        Some("schema changed".to_string())
    );
    assert!(mobile.has_vectors);
    assert!(!mobile.search_ready);
    assert!(mobile.dimension_configured);
    assert_eq!(mobile.point_count, 7);
}
