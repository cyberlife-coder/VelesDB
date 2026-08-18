use super::*;

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

/// Creates a store with nodes [1..=count] and returns it.
fn store_with_nodes(count: u64) -> Arc<MobileGraphStore> {
    let store = MobileGraphStore::new();
    for i in 1..=count {
        store.add_node(person_node(i));
    }
    store
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
fn test_mobile_graph_store_add_nodes() {
    let store = store_with_nodes(1);
    assert_eq!(store.node_count(), 1);
}

#[test]
fn test_mobile_graph_store_add_edges() {
    let store = store_with_nodes(2);
    let result = store.add_edge(knows_edge(100, 1, 2));
    assert!(result.is_ok());
    assert_eq!(store.edge_count(), 1);
}

#[test]
fn test_mobile_graph_save_load_roundtrip() {
    let store = store_with_nodes(3);
    store.add_edge(knows_edge(100, 1, 2)).unwrap();
    store.add_edge(knows_edge(101, 2, 3)).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.json").to_string_lossy().to_string();
    store.save(path.clone()).unwrap();

    let restored = MobileGraphStore::load(path).unwrap();
    assert_eq!(restored.node_count(), 3);
    assert_eq!(restored.edge_count(), 2);
    assert_eq!(restored.get_node(1).unwrap().label, "Person");
    // Adjacency is rebuilt from the stored edges, not persisted directly.
    let outgoing: Vec<u64> = restored.get_outgoing(1).iter().map(|e| e.id).collect();
    assert_eq!(outgoing, vec![100]);
    assert_eq!(restored.get_outgoing(2).len(), 1);
}

#[test]
fn test_mobile_graph_load_missing_file_errors() {
    let result = MobileGraphStore::load("/nonexistent/velesdb-graph-missing.json".to_string());
    assert!(result.is_err());
}

#[test]
fn test_mobile_graph_store_duplicate_edge_error() {
    let store = store_with_nodes(2);
    let _ = store.add_edge(knows_edge(100, 1, 2));
    let result = store.add_edge(knows_edge(100, 1, 2));
    assert!(result.is_err());
}

#[test]
fn test_mobile_graph_store_get_outgoing() {
    let store = store_with_nodes(3);
    let _ = store.add_edge(knows_edge(100, 1, 2));
    let _ = store.add_edge(knows_edge(101, 1, 3));
    assert_eq!(store.get_outgoing(1).len(), 2);
}

#[test]
fn test_mobile_graph_store_bfs_traverse() {
    let store = store_with_nodes(4);

    // Create chain: 1 -> 2 -> 3 -> 4
    let _ = store.add_edge(knows_edge(100, 1, 2));
    let _ = store.add_edge(knows_edge(101, 2, 3));
    let _ = store.add_edge(knows_edge(102, 3, 4));

    let results = store.bfs_traverse(1, 3, 100);

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
    let store = store_with_nodes(2);
    let _ = store.add_edge(knows_edge(100, 1, 2));

    assert_eq!(store.node_count(), 2);
    assert_eq!(store.edge_count(), 1);

    store.remove_node(1);

    assert_eq!(store.node_count(), 1);
    assert_eq!(store.edge_count(), 0); // Edge should be removed too
}

#[test]
fn test_mobile_graph_store_remove_edge() {
    let store = store_with_nodes(2);
    let _ = store.add_edge(knows_edge(100, 1, 2));

    assert_eq!(store.edge_count(), 1);

    store.remove_edge(100);

    assert_eq!(store.edge_count(), 0);
    assert!(store.get_outgoing(1).is_empty());
    assert!(store.get_incoming(2).is_empty());
}

#[test]
fn test_mobile_graph_store_clear() {
    let store = store_with_nodes(2);
    let _ = store.add_edge(knows_edge(100, 1, 2));

    store.clear();

    assert_eq!(store.node_count(), 0);
    assert_eq!(store.edge_count(), 0);
}

/// Regression: `save()` must never deadlock against a concurrent mutator.
///
/// `save()` used to take `nodes` then `edges` (the reverse of every
/// mutator's `edges → … → nodes`), so a `save` on one UniFFI-called thread
/// racing a `remove_node`/`clear` on another was an ABBA deadlock with no
/// recovery. This runs both in tight loops from two threads and fails via a
/// watchdog if they lock up. On the pre-fix code the workers wedge and the
/// `recv_timeout` elapses; on the fixed code (save holds one lock at a
/// time) it completes in well under the timeout.
#[test]
fn save_never_deadlocks_against_concurrent_mutation() {
    use std::sync::mpsc;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.json").to_string_lossy().into_owned();
    let store = store_with_nodes(8);
    for i in 0..7 {
        let _ = store.add_edge(knows_edge(1000 + i, i + 1, i + 2));
    }

    let iters = 2_000;
    let (done_tx, done_rx) = mpsc::channel::<()>();

    let saver = {
        let store = Arc::clone(&store);
        let path = path.clone();
        let done_tx = done_tx.clone();
        std::thread::spawn(move || {
            for _ in 0..iters {
                let _ = store.save(path.clone());
            }
            let _ = done_tx.send(());
        })
    };
    let mutator = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            for i in 0..iters {
                // Alternate the two mutators that take edges→…→nodes, the
                // order save() used to invert.
                if i % 2 == 0 {
                    store.remove_node(((i % 8) + 1) as u64);
                    store.add_node(person_node(((i % 8) + 1) as u64));
                } else {
                    store.clear();
                    store.add_node(person_node(1));
                }
            }
            let _ = done_tx.send(());
        })
    };

    // Both workers must report within the watchdog window or the locks
    // deadlocked (parking_lot has no recovery — the threads never return).
    for _ in 0..2 {
        done_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("save/mutation deadlocked: worker did not finish within 30s");
    }
    saver.join().unwrap();
    mutator.join().unwrap();
}
