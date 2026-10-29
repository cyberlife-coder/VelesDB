use super::*;
use crate::{
    DistanceMetric, MobileAccessDecision, MobileObserver, MobileQueryContext, VelesDatabase,
};
use tempfile::TempDir;

/// Creates a test node with the given ID and "Person" label.
fn person_node(id: u64) -> MobileGraphNode {
    MobileGraphNode {
        id,
        label: "Person".to_string(),
        properties_json: None,
        vector: None,
    }
}

/// Creates a test edge with the given ID, source, and target ("KNOWS" label).
fn knows_edge(id: u64, source: u64, target: u64) -> MobileGraphEdge {
    MobileGraphEdge {
        id,
        source,
        target,
        label: "KNOWS".to_string(),
        properties_json: None,
    }
}

/// Opens a database in a fresh temp dir with an empty `kg` graph collection.
fn graph_db() -> (TempDir, Arc<VelesDatabase>, Arc<MobileGraphStore>) {
    let dir = tempfile::tempdir().unwrap();
    let db = VelesDatabase::open(dir.path().to_string_lossy().into_owned()).unwrap();
    db.create_graph_collection("kg".to_string()).unwrap();
    let store = db.get_graph_store("kg".to_string()).unwrap().unwrap();
    (dir, db, store)
}

/// Creates a store with nodes [1..=count] and returns it.
fn store_with_nodes(count: u64) -> (TempDir, Arc<VelesDatabase>, Arc<MobileGraphStore>) {
    let (dir, db, store) = graph_db();
    for i in 1..=count {
        store.add_node(person_node(i)).unwrap();
    }
    (dir, db, store)
}

#[test]
fn test_mobile_graph_node_creation() {
    let node = MobileGraphNode {
        id: 1,
        label: "Person".to_string(),
        properties_json: Some(r#"{"name": "John"}"#.to_string()),
        vector: None,
    };
    assert_eq!(node.id, 1);
    assert_eq!(node.label, "Person");
}

#[test]
fn test_mobile_graph_edge_creation() {
    let edge = knows_edge(100, 1, 2);
    assert_eq!(edge.id, 100);
    assert_eq!(edge.source, 1);
    assert_eq!(edge.target, 2);
}

#[test]
fn get_graph_store_is_none_for_a_missing_collection() {
    let (_dir, db, _store) = graph_db();
    assert!(db.get_graph_store("absent".to_string()).unwrap().is_none());
}

#[test]
fn get_graph_store_rejects_a_vector_collection() {
    let (_dir, db, _store) = graph_db();
    db.create_collection("vectors".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let err = db.get_graph_store("vectors".to_string()).err();
    assert!(
        matches!(err, Some(VelesError::Collection { ref message }) if message.contains("not a graph collection")),
        "got {err:?}"
    );
}

#[test]
fn test_mobile_graph_store_add_nodes() {
    let (_dir, _db, store) = store_with_nodes(1);
    assert_eq!(store.node_count().unwrap(), 1);
    assert!(store.has_node(1).unwrap());
    assert!(!store.has_node(2).unwrap());
}

#[test]
fn node_round_trips_label_and_properties() {
    let (_dir, _db, store) = graph_db();
    store
        .add_node(MobileGraphNode {
            id: 7,
            label: "Person".to_string(),
            properties_json: Some(r#"{"name":"Ada","age":36}"#.to_string()),
            vector: None,
        })
        .unwrap();

    let node = store.get_node(7).unwrap().expect("node stored");
    assert_eq!(node.label, "Person");
    assert_eq!(node.vector, None);
    let props: serde_json::Value =
        serde_json::from_str(node.properties_json.as_deref().unwrap()).unwrap();
    assert_eq!(props, serde_json::json!({"name": "Ada", "age": 36}));
    assert!(store.get_node(8).unwrap().is_none());
}

#[test]
fn add_node_replaces_the_node_with_the_same_id() {
    let (_dir, _db, store) = store_with_nodes(1);
    let mut renamed = person_node(1);
    renamed.label = "Robot".to_string();
    store.add_node(renamed).unwrap();

    assert_eq!(store.node_count().unwrap(), 1);
    assert_eq!(store.get_node(1).unwrap().unwrap().label, "Robot");
    assert!(store
        .get_nodes_by_label("Person".to_string())
        .unwrap()
        .is_empty());
}

#[test]
fn add_node_refuses_properties_that_are_not_a_json_object() {
    let (_dir, _db, store) = graph_db();
    for bad in ["not json", "[1, 2]", r#"{"_labels": ["X"]}"#] {
        let mut node = person_node(1);
        node.properties_json = Some(bad.to_string());
        assert!(
            store.add_node(node).is_err(),
            "properties_json {bad:?} must be refused"
        );
    }
    assert_eq!(store.node_count().unwrap(), 0);
}

#[test]
fn node_vector_needs_a_collection_with_embeddings() {
    let (_dir, db, store) = graph_db();
    let mut node = person_node(1);
    node.vector = Some(vec![1.0, 0.0]);
    assert!(store.add_node(node.clone()).is_err());

    db.create_graph_collection_with_embeddings("kg_emb".to_string(), 2, DistanceMetric::Cosine)
        .unwrap();
    let embedded = db.get_graph_store("kg_emb".to_string()).unwrap().unwrap();
    embedded.add_node(node).unwrap();
    let stored = embedded.get_node(1).unwrap().unwrap();
    assert_eq!(stored.vector, Some(vec![1.0, 0.0]));
    assert_eq!(stored.label, "Person");
}

#[test]
fn test_mobile_graph_store_add_edges() {
    let (_dir, _db, store) = store_with_nodes(2);
    let mut edge = knows_edge(100, 1, 2);
    edge.properties_json = Some(r#"{"since":2020}"#.to_string());
    store.add_edge(edge).unwrap();

    assert_eq!(store.edge_count().unwrap(), 1);
    assert!(store.has_edge(100).unwrap());
    let stored = store.get_edge(100).unwrap().expect("edge stored");
    assert_eq!((stored.source, stored.target), (1, 2));
    assert_eq!(stored.label, "KNOWS");
    assert_eq!(stored.properties_json.as_deref(), Some(r#"{"since":2020}"#));
    assert!(store.get_edge(101).unwrap().is_none());
}

#[test]
fn test_mobile_graph_store_duplicate_edge_error() {
    let (_dir, _db, store) = store_with_nodes(2);
    store.add_edge(knows_edge(100, 1, 2)).unwrap();
    assert!(store.add_edge(knows_edge(100, 1, 2)).is_err());
}

#[test]
fn add_edge_refuses_an_endpoint_that_was_never_added() {
    let (_dir, _db, store) = store_with_nodes(1);
    let err = store.add_edge(knows_edge(100, 1, 2)).err();
    assert!(
        matches!(err, Some(VelesError::Database { ref code, .. }) if !code.is_empty()),
        "core's NodeNotFound should cross with its code, got {err:?}"
    );
    assert_eq!(store.edge_count().unwrap(), 0);
}

#[test]
fn test_mobile_graph_store_adjacency_and_degrees() {
    let (_dir, _db, store) = store_with_nodes(3);
    store.add_edge(knows_edge(100, 1, 2)).unwrap();
    store.add_edge(knows_edge(101, 1, 3)).unwrap();
    let mut likes = knows_edge(102, 3, 1);
    likes.label = "LIKES".to_string();
    store.add_edge(likes).unwrap();

    assert_eq!(store.get_outgoing(1).unwrap().len(), 2);
    assert_eq!(store.get_incoming(1).unwrap().len(), 1);
    assert_eq!(
        store
            .get_outgoing_by_label(3, "LIKES".to_string())
            .unwrap()
            .len(),
        1
    );
    let mut neighbors = store.get_neighbors(1).unwrap();
    neighbors.sort_unstable();
    assert_eq!(neighbors, vec![2, 3]);
    assert_eq!(store.out_degree(1).unwrap(), 2);
    assert_eq!(store.in_degree(1).unwrap(), 1);
    assert_eq!(
        store.get_edges_by_label("KNOWS".to_string()).unwrap().len(),
        2
    );
}

#[test]
fn get_nodes_by_label_matches_core_labels() {
    let (_dir, _db, store) = store_with_nodes(2);
    let mut project = person_node(3);
    project.label = "Project".to_string();
    store.add_node(project).unwrap();

    let mut people: Vec<u64> = store
        .get_nodes_by_label("Person".to_string())
        .unwrap()
        .iter()
        .map(|n| n.id)
        .collect();
    people.sort_unstable();
    assert_eq!(people, vec![1, 2]);
    assert_eq!(
        store
            .get_nodes_by_label("Project".to_string())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn test_mobile_graph_store_bfs_traverse() {
    let (_dir, _db, store) = store_with_nodes(4);

    // Create chain: 1 -> 2 -> 3 -> 4
    store.add_edge(knows_edge(100, 1, 2)).unwrap();
    store.add_edge(knows_edge(101, 2, 3)).unwrap();
    store.add_edge(knows_edge(102, 3, 4)).unwrap();

    let results = store.bfs_traverse(1, 3, 100).unwrap();

    // Should find nodes 2, 3, 4 at depths 1, 2, 3, each carrying the
    // edge-ID path mirroring core's TraversalResult::path.
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

/// Projects traversal results to comparable `(node, depth, path)` triples.
fn triples(results: Vec<TraversalResult>) -> Vec<(u64, u32, Vec<u64>)> {
    results
        .into_iter()
        .map(|r| (r.node_id, r.depth, r.path))
        .collect()
}

/// The store's traversals are core's: on the same collection, BFS, DFS and
/// multi-source BFS return exactly what core's `GraphCollection` returns.
/// The multi-source case is where the former in-memory fork disagreed — it
/// deduplicated by target node, so node 4 below came back once, where core
/// reports one path per source.
#[test]
fn traversals_are_cores() {
    let (_dir, db, store) = store_with_nodes(5);
    // Two sources (1, 2) converge on 3, which leads to 4 and 5.
    for (id, s, t) in [
        (100, 1, 3),
        (101, 2, 3),
        (102, 3, 4),
        (103, 3, 5),
        (104, 4, 5),
    ] {
        store.add_edge(knows_edge(id, s, t)).unwrap();
    }
    let core = db.inner.get_graph_collection("kg").unwrap();
    let config = velesdb_core::TraversalConfig::with_range(1, 3).with_limit(100);

    let to_mobile = |r: Vec<velesdb_core::TraversalResult>| {
        triples(r.into_iter().map(TraversalResult::from).collect())
    };
    assert_eq!(
        triples(store.bfs_traverse(1, 3, 100).unwrap()),
        to_mobile(core.traverse_bfs(1, &config))
    );
    assert_eq!(
        triples(store.dfs_traverse(1, 3, 100).unwrap()),
        to_mobile(core.traverse_dfs(1, &config))
    );
    let parallel = triples(store.bfs_traverse_parallel(vec![1, 2], 3, 100).unwrap());
    assert_eq!(
        parallel,
        to_mobile(core.traverse_bfs_parallel(&[1, 2], &config))
    );
    assert_eq!(
        parallel.iter().filter(|(node, _, _)| *node == 4).count(),
        2,
        "one path to node 4 per source"
    );
}

#[test]
fn test_traversal_result_from_core() {
    let core = velesdb_core::TraversalResult::new(7, vec![10, 20], 2);
    let mobile: TraversalResult = core.into();
    assert_eq!(mobile.node_id, 7);
    assert_eq!(mobile.path, vec![10, 20]);
    assert_eq!(mobile.depth, 2);
}

#[test]
fn test_graph_node_from_core() {
    let mut props = std::collections::HashMap::new();
    props.insert("name".to_string(), serde_json::json!("Alice"));
    let core = velesdb_core::GraphNode::new(1, "Person")
        .with_properties(props)
        .with_vector(vec![0.1, 0.2]);
    let mobile: MobileGraphNode = core.into();
    assert_eq!(mobile.id, 1);
    assert_eq!(mobile.label, "Person");
    assert_eq!(mobile.vector, Some(vec![0.1, 0.2]));
    assert!(mobile.properties_json.is_some());
}

#[test]
fn test_graph_edge_from_core() -> Result<(), velesdb_core::Error> {
    let core = velesdb_core::GraphEdge::new(100, 1, 2, "KNOWS")?;
    let mobile: MobileGraphEdge = core.into();
    assert_eq!(mobile.id, 100);
    assert_eq!(mobile.source, 1);
    assert_eq!(mobile.target, 2);
    assert_eq!(mobile.label, "KNOWS");
    assert_eq!(mobile.properties_json, None);
    Ok(())
}

#[test]
fn test_mobile_graph_store_remove_node() {
    let (_dir, _db, store) = store_with_nodes(2);
    store.add_edge(knows_edge(100, 1, 2)).unwrap();

    assert_eq!(store.node_count().unwrap(), 2);
    assert_eq!(store.edge_count().unwrap(), 1);

    store.remove_node(1).unwrap();

    assert_eq!(store.node_count().unwrap(), 1);
    assert_eq!(store.edge_count().unwrap(), 0); // Edge should be removed too
    assert!(store.get_incoming(2).unwrap().is_empty());
}

#[test]
fn test_mobile_graph_store_remove_edge() {
    let (_dir, _db, store) = store_with_nodes(2);
    store.add_edge(knows_edge(100, 1, 2)).unwrap();

    assert_eq!(store.edge_count().unwrap(), 1);

    store.remove_edge(100);
    store.remove_edge(100); // absent: no-op

    assert_eq!(store.edge_count().unwrap(), 0);
    assert!(store.get_outgoing(1).unwrap().is_empty());
    assert!(store.get_incoming(2).unwrap().is_empty());
}

#[test]
fn test_mobile_graph_store_clear() {
    let (_dir, _db, store) = store_with_nodes(2);
    store.add_edge(knows_edge(100, 1, 2)).unwrap();

    store.clear().unwrap();

    assert_eq!(store.node_count().unwrap(), 0);
    assert_eq!(store.edge_count().unwrap(), 0);
}

/// Replaces the removed `save`/`load` round trip: core persists the graph,
/// so a reopened database serves it with no explicit save.
#[test]
fn graph_survives_a_database_reopen() {
    let (dir, db, store) = store_with_nodes(3);
    store.add_edge(knows_edge(100, 1, 2)).unwrap();
    store.add_edge(knows_edge(101, 2, 3)).unwrap();
    drop(store);
    drop(db);

    let db = VelesDatabase::open(dir.path().to_string_lossy().into_owned()).unwrap();
    let restored = db.get_graph_store("kg".to_string()).unwrap().unwrap();
    assert_eq!(restored.node_count().unwrap(), 3);
    assert_eq!(restored.edge_count().unwrap(), 2);
    assert_eq!(restored.get_node(1).unwrap().unwrap().label, "Person");
    let outgoing: Vec<u64> = restored
        .get_outgoing(1)
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(outgoing, vec![100]);
}

/// Guards the `_labels` contract in `graph_payload.rs`: a node written
/// through the store is the node VelesQL `MATCH` sees.
#[test]
fn velesql_match_sees_store_nodes() {
    let (_dir, db, store) = store_with_nodes(2);
    let mut project = person_node(3);
    project.label = "Project".to_string();
    store.add_node(project).unwrap();
    store.add_edge(knows_edge(100, 1, 2)).unwrap();

    let result = db
        .execute_query(
            "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b".to_string(),
            Some(r#"{"_collection":"kg"}"#.to_string()),
        )
        .unwrap();
    assert_eq!(
        result.rows.len(),
        1,
        "one KNOWS edge between two Person nodes"
    );
}

/// Writes `json` as a snapshot file in `dir` and returns its path.
fn write_snapshot(dir: &TempDir, json: &serde_json::Value) -> String {
    let path = dir.path().join("graph.json");
    std::fs::write(&path, serde_json::to_vec(json).unwrap()).unwrap();
    path.to_string_lossy().into_owned()
}

/// A snapshot in the exact shape the removed `save` wrote.
fn legacy_snapshot() -> serde_json::Value {
    serde_json::json!({
        "nodes": [
            {"id": 1, "label": "Person", "properties_json": "{\"name\":\"Ada\"}", "vector": null},
            {"id": 2, "label": "Person", "properties_json": null, "vector": null},
            {"id": 3, "label": "Project", "properties_json": null, "vector": null}
        ],
        "edges": [
            {"id": 100, "source": 1, "target": 2, "label": "KNOWS", "properties_json": null},
            {"id": 101, "source": 2, "target": 3, "label": "WORKS_ON", "properties_json": null}
        ]
    })
}

#[test]
fn import_snapshot_migrates_a_legacy_save() {
    let (dir, _db, store) = graph_db();
    let path = write_snapshot(&dir, &legacy_snapshot());

    store.import_snapshot(path).unwrap();
    store.flush().unwrap();

    assert_eq!(store.node_count().unwrap(), 3);
    assert_eq!(store.edge_count().unwrap(), 2);
    let ada = store.get_node(1).unwrap().unwrap();
    assert_eq!(ada.label, "Person");
    assert_eq!(ada.properties_json.as_deref(), Some(r#"{"name":"Ada"}"#));
    let reached: Vec<u64> = store
        .bfs_traverse(1, 2, 10)
        .unwrap()
        .iter()
        .map(|r| r.node_id)
        .collect();
    assert_eq!(reached, vec![2, 3]);
}

#[test]
fn import_snapshot_refuses_before_writing_anything() {
    let (dir, _db, store) = store_with_nodes(1);
    store.add_node(person_node(2)).unwrap();
    store.add_edge(knows_edge(100, 1, 2)).unwrap();

    // Edge 100 is already in the store.
    let path = write_snapshot(&dir, &legacy_snapshot());
    assert!(store.import_snapshot(path).is_err());

    // Edge 200 points at node 9, in neither the snapshot nor the store.
    let mut dangling = legacy_snapshot();
    dangling["edges"] = serde_json::json!([
        {"id": 200, "source": 1, "target": 9, "label": "KNOWS", "properties_json": null}
    ]);
    let path = write_snapshot(&dir, &dangling);
    let err = store.import_snapshot(path).err();
    assert!(
        matches!(err, Some(VelesError::Database { ref message, .. }) if message.contains("node 9")),
        "got {err:?}"
    );

    assert_eq!(store.node_count().unwrap(), 2, "no snapshot node written");
    assert_eq!(store.edge_count().unwrap(), 1);
}

#[test]
fn import_snapshot_missing_file_errors() {
    let (_dir, _db, store) = graph_db();
    let result = store.import_snapshot("/nonexistent/velesdb-graph-missing.json".to_string());
    assert!(result.is_err());
}

/// Test observer that denies every read.
struct DenyAll;

impl MobileObserver for DenyAll {
    fn on_query_request(&self, _context: MobileQueryContext) -> MobileAccessDecision {
        MobileAccessDecision::Deny {
            reason: "graph reads are off".to_string(),
        }
    }
}

#[test]
fn graph_reads_go_through_the_read_gate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().into_owned();
    {
        let db = VelesDatabase::open(path.clone()).unwrap();
        db.create_graph_collection("kg".to_string()).unwrap();
    }
    let db = VelesDatabase::open_with_observer(path, Arc::new(DenyAll)).unwrap();
    let store = db.get_graph_store("kg".to_string()).unwrap().unwrap();

    // Writes are not reads: the gate does not see them.
    store.add_node(person_node(1)).unwrap();
    store.add_node(person_node(2)).unwrap();
    store.add_edge(knows_edge(100, 1, 2)).unwrap();

    assert!(store.bfs_traverse(1, 2, 10).is_err());
    assert!(store.dfs_traverse(1, 2, 10).is_err());
    assert!(store.get_node(1).is_err());
    assert!(store.get_outgoing(1).is_err());
    assert!(store.node_count().is_err());
}
