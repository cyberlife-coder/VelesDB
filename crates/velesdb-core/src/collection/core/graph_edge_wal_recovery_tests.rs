//! Crash-recovery tests for the graph edge WAL (durability).
//!
//! These prove that edge mutations (`add_edge`, `add_edges_batch`,
//! `remove_edge`, cascade `remove_node_edges`) survive a simulated crash
//! — dropping the `Collection` WITHOUT calling `flush()`, so the
//! `edge_store.bin` snapshot is never written — followed by reopen, where
//! the WAL is replayed on top of the (here, empty) snapshot.

#[cfg(test)]
mod tests {
    use crate::collection::graph::{GraphEdge, GraphSchema};
    use crate::collection::types::Collection;
    use crate::DistanceMetric;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_graph(path: PathBuf) -> Collection {
        Collection::create_graph_collection(
            path,
            "kg",
            GraphSchema::schemaless(),
            None,
            DistanceMetric::Cosine,
        )
        .expect("create graph collection")
    }

    fn make_edge(id: u64, source: u64, target: u64, label: &str) -> GraphEdge {
        GraphEdge::new(id, source, target, label).expect("valid edge")
    }

    #[test]
    fn test_edge_wal_survives_reopen_without_flush() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();

        {
            let coll = create_graph(path.clone());
            // Same-shard edge (source 100, target 356 → both shard 100).
            coll.add_edge(make_edge(1, 100, 356, "KNOWS")).unwrap();
            // Cross-shard edge (source 100 → shard 100, target 200 → shard 200).
            coll.add_edge(make_edge(2, 100, 200, "KNOWS")).unwrap();
            coll.add_edge(make_edge(3, 200, 300, "FOLLOWS")).unwrap();
            coll.store_node_payload(100, &serde_json::json!({"name": "A"}))
                .unwrap();
            // DROP without flush → edge_store.bin is never written (crash sim).
        }

        let reopened = Collection::open(path).expect("reopen");
        assert_eq!(reopened.edge_count(), 3, "all edges replayed from WAL");
        assert_eq!(reopened.get_outgoing_edges(100).len(), 2);
        assert_eq!(reopened.get_incoming_edges(200).len(), 1);
        assert_eq!(reopened.get_incoming_edges(300).len(), 1);
        assert_eq!(reopened.get_edges_by_label("KNOWS").len(), 2);
        assert_eq!(reopened.get_edges_by_label("FOLLOWS").len(), 1);
        // Node payload (payload WAL) also survives.
        let payload = reopened.get_node_payload(100).unwrap();
        assert_eq!(payload, Some(serde_json::json!({"name": "A"})));
    }

    #[test]
    fn test_edge_wal_remove_survives_reopen() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();

        {
            let coll = create_graph(path.clone());
            coll.add_edge(make_edge(1, 1, 2, "KNOWS")).unwrap();
            coll.add_edge(make_edge(2, 2, 3, "KNOWS")).unwrap();
            coll.add_edge(make_edge(3, 3, 4, "KNOWS")).unwrap();
            assert!(coll.remove_edge(2));
        }

        let reopened = Collection::open(path).expect("reopen");
        assert_eq!(reopened.edge_count(), 2, "removed edge stays removed");
        assert!(reopened.get_outgoing_edges(2).is_empty());
        assert_eq!(reopened.get_outgoing_edges(1).len(), 1);
        assert_eq!(reopened.get_outgoing_edges(3).len(), 1);
    }

    #[test]
    fn test_edge_wal_cascade_delete_survives_reopen() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();

        {
            let coll = create_graph(path.clone());
            coll.store_node_payload(50, &serde_json::json!({"n": "N"}))
                .unwrap();
            coll.add_edge(make_edge(1, 50, 60, "E")).unwrap();
            coll.add_edge(make_edge(2, 70, 50, "E")).unwrap();
            coll.add_edge(make_edge(3, 80, 90, "E")).unwrap();
            // Cascade-delete node 50 (removes edges 1 and 2).
            coll.delete(&[50]).unwrap();
            assert_eq!(coll.edge_count(), 1);
        }

        let reopened = Collection::open(path).expect("reopen");
        assert_eq!(reopened.edge_count(), 1, "cascade tombstone replayed");
        assert!(reopened.get_outgoing_edges(50).is_empty());
        assert!(reopened.get_incoming_edges(50).is_empty());
        assert_eq!(reopened.get_outgoing_edges(80).len(), 1);
    }

    #[test]
    fn test_add_edges_batch_durable() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();

        {
            let coll = create_graph(path.clone());
            let edges: Vec<GraphEdge> = (0..50)
                .map(|i| make_edge(i, i, i + 1000, "BATCH"))
                .collect();
            let added = coll.add_edges_batch(edges).unwrap();
            assert_eq!(added, 50);
        }

        let reopened = Collection::open(path).expect("reopen");
        assert_eq!(reopened.edge_count(), 50, "all batched edges replayed");
        assert_eq!(reopened.get_edges_by_label("BATCH").len(), 50);
    }

    #[test]
    fn test_flush_truncates_edge_wal() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();
        let wal = path.join("edges.wal");

        let coll = create_graph(path.clone());
        coll.add_edge(make_edge(1, 1, 2, "A")).unwrap();
        coll.add_edge(make_edge(2, 2, 3, "A")).unwrap();
        assert!(wal.metadata().unwrap().len() > 0, "WAL has entries");

        coll.flush_full().unwrap();
        assert_eq!(
            wal.metadata().unwrap().len(),
            0,
            "WAL truncated after snapshot"
        );
        assert!(path.join("edge_store.bin").exists(), "snapshot written");

        // Post-flush delta: add another edge, then crash (drop) without flush.
        coll.add_edge(make_edge(3, 3, 4, "A")).unwrap();
        drop(coll);

        let reopened = Collection::open(path).expect("reopen");
        // 2 from snapshot + 1 from WAL delta — no double counting (idempotent).
        assert_eq!(reopened.edge_count(), 3);
        assert_eq!(reopened.get_edges_by_label("A").len(), 3);
    }

    #[test]
    fn test_open_legacy_graph_without_edge_wal() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();

        {
            let coll = create_graph(path.clone());
            coll.add_edge(make_edge(1, 1, 2, "L")).unwrap();
            coll.flush_full().unwrap();
        }
        // Simulate a pre-feature DB: delete the edges.wal entirely.
        let wal = path.join("edges.wal");
        if wal.exists() {
            std::fs::remove_file(&wal).unwrap();
        }

        let reopened = Collection::open(path).expect("reopen legacy");
        assert_eq!(reopened.edge_count(), 1, "edges load from snapshot");
    }

    #[test]
    fn test_edge_wal_torn_tail_tolerated() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();

        {
            let coll = create_graph(path.clone());
            coll.add_edge(make_edge(1, 1, 2, "T")).unwrap();
        }
        // Append a truncated partial entry: a length prefix declaring 32
        // bytes but supplying only 4 — the torn-tail crash scenario.
        let wal = path.join("edges.wal");
        let mut f = std::fs::OpenOptions::new().append(true).open(&wal).unwrap();
        f.write_all(&32u32.to_le_bytes()).unwrap();
        f.write_all(&[0x01, 0x00, 0x00, 0x00]).unwrap();
        f.sync_all().unwrap();

        let reopened = Collection::open(path).expect("reopen tolerates torn tail");
        assert_eq!(reopened.edge_count(), 1, "complete entry survives");
    }

    #[test]
    fn test_edge_wal_replay_preserves_edge_properties() {
        use std::collections::HashMap;
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().to_path_buf();

        {
            let coll = create_graph(path.clone());
            let mut props = HashMap::new();
            props.insert("weight".to_string(), serde_json::json!(42));
            let edge = make_edge(1, 1, 2, "WEIGHTED").with_properties(props);
            coll.add_edge(edge).unwrap();
        }

        let reopened = Collection::open(path).expect("reopen");
        assert_eq!(reopened.edge_count(), 1);
        let edges = reopened.get_edges_by_label("WEIGHTED");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].property("weight"), Some(&serde_json::json!(42)));
    }
}
