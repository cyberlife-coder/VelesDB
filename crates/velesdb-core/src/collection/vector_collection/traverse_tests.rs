//! Tests for `VectorCollection::traverse_bfs` (issue #1439).
//!
//! `VectorCollection` shares its edge store with `GraphCollection` and
//! `MetadataCollection` (docs/ARCHITECTURE.md F2.2, R1.2c): edges created via
//! REST `/relations` or the agent-memory wedge (`velesdb-memory`) land on the
//! backing `Collection`'s edge store regardless of the newtype used to reach
//! it. `traverse_bfs` must therefore be usable on a `VectorCollection`,
//! mirroring the already-shared edge primitives (`add_edge`, `remove_edge`,
//! `get_outgoing_edges` — `collection/vector_collection/crud.rs`).

use tempfile::TempDir;

use crate::collection::graph::{GraphEdge, TraversalConfig};
use crate::distance::DistanceMetric;
use crate::point::Point;
use crate::quantization::StorageMode;
use crate::VectorCollection;

/// Creates a 4-dim `VectorCollection` backed by a temporary directory.
fn create_test_vc() -> (VectorCollection, TempDir) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_coll");
    let coll = VectorCollection::create(path, "test", 4, DistanceMetric::Cosine, StorageMode::Full)
        .unwrap();
    (coll, dir)
}

#[test]
fn test_traverse_bfs_reaches_edges_added_via_shared_edge_store() {
    let (coll, _dir) = create_test_vc();

    for id in [1u64, 2, 3] {
        coll.upsert(vec![Point::new(
            id,
            vec![0.0; 4],
            Some(serde_json::json!({})),
        )])
        .unwrap();
    }

    // Same path REST `/relations` and the memory wedge use: `add_edge`
    // delegates to the shared `Collection` edge store.
    coll.add_edge(GraphEdge::new(1, 1, 2, "KNOWS").unwrap())
        .unwrap();
    coll.add_edge(GraphEdge::new(2, 2, 3, "KNOWS").unwrap())
        .unwrap();

    let config = TraversalConfig {
        max_depth: 3,
        min_depth: 1,
        ..TraversalConfig::default()
    };
    let results = coll.traverse_bfs(1, &config);

    let target_ids: std::collections::HashSet<u64> = results.iter().map(|r| r.target_id).collect();
    assert!(target_ids.contains(&2), "should reach node 2 at depth 1");
    assert!(target_ids.contains(&3), "should reach node 3 at depth 2");
}

#[test]
fn test_traverse_bfs_respects_max_depth_on_vector_collection() {
    let (coll, _dir) = create_test_vc();

    for id in [1u64, 2, 3] {
        coll.upsert(vec![Point::new(
            id,
            vec![0.0; 4],
            Some(serde_json::json!({})),
        )])
        .unwrap();
    }

    coll.add_edge(GraphEdge::new(1, 1, 2, "KNOWS").unwrap())
        .unwrap();
    coll.add_edge(GraphEdge::new(2, 2, 3, "KNOWS").unwrap())
        .unwrap();

    let config = TraversalConfig {
        max_depth: 1,
        min_depth: 1,
        ..TraversalConfig::default()
    };
    let results = coll.traverse_bfs(1, &config);

    let target_ids: std::collections::HashSet<u64> = results.iter().map(|r| r.target_id).collect();
    assert!(target_ids.contains(&2));
    assert!(
        !target_ids.contains(&3),
        "max_depth=1 should stop before node 3"
    );
}

#[test]
fn test_traverse_bfs_on_vector_collection_with_no_edges_returns_empty() {
    let (coll, _dir) = create_test_vc();

    let config = TraversalConfig::default();
    let results = coll.traverse_bfs(1, &config);

    assert!(results.is_empty());
}
