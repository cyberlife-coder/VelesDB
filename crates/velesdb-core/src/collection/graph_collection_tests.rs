use super::*;
use crate::collection::graph::GraphSchema;
use crate::distance::DistanceMetric;
use std::collections::HashMap;
use tempfile::{tempdir, TempDir};

/// Creates a schemaless cosine `GraphCollection` in a fresh temp dir.
///
/// Returns the `TempDir` guard alongside the collection so the backing
/// directory outlives the test. `dimension` controls embedding support
/// (`None` for payload/edge-only collections, `Some(n)` for searchable ones).
fn make_test_collection(dimension: Option<usize>) -> (TempDir, GraphCollection) {
    let dir = tempdir().unwrap();
    let col = GraphCollection::create(
        dir.path().to_path_buf(),
        "kg",
        dimension,
        DistanceMetric::Cosine,
        GraphSchema::schemaless(),
    )
    .unwrap();
    (dir, col)
}

#[test]
fn test_all_node_ids_returns_ids_with_payload() {
    let (_dir, col) = make_test_collection(None);

    // Store payloads on two nodes
    col.upsert_node_payload(10, &serde_json::json!({"name": "Alice"}))
        .unwrap();
    col.upsert_node_payload(20, &serde_json::json!({"name": "Bob"}))
        .unwrap();

    let ids = col.all_node_ids();
    assert!(ids.contains(&10), "node 10 should be present");
    assert!(ids.contains(&20), "node 20 should be present");
    assert_eq!(ids.len(), 2);
}

#[test]
fn test_upsert_node_with_embedding_is_searchable() {
    let (_dir, col) = make_test_collection(Some(4));

    col.upsert_node(
        10,
        &serde_json::json!({"name": "Alice"}),
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    )
    .unwrap();

    assert_eq!(
        col.get_node_payload(10).unwrap(),
        Some(serde_json::json!({"name": "Alice"}))
    );
    let results = col.search_by_embedding(&[1.0, 0.0, 0.0, 0.0], 1).unwrap();
    assert_eq!(results[0].point.id, 10);
}

#[test]
fn test_edge_count_returns_correct_count() {
    let (_dir, col) = make_test_collection(None);

    assert_eq!(col.edge_count(), 0);
    for id in [10, 20, 30] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }

    let edge1 = crate::collection::graph::GraphEdge::new(1, 10, 20, "knows").unwrap();
    col.add_edge(edge1).unwrap();
    assert_eq!(col.edge_count(), 1);

    let edge2 = crate::collection::graph::GraphEdge::new(2, 20, 30, "likes").unwrap();
    col.add_edge(edge2).unwrap();
    assert_eq!(col.edge_count(), 2);
}

#[test]
fn test_traverse_bfs_parallel_through_graph_collection() {
    let (_dir, col) = make_test_collection(None);

    // Build chain: 1->2->3
    for id in [1, 2, 3] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(1, 1, 2, "NEXT").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(2, 2, 3, "NEXT").unwrap())
        .unwrap();

    let config = TraversalConfig {
        max_depth: 3,
        min_depth: 1,
        ..TraversalConfig::default()
    };
    let results = col.traverse_bfs_parallel(&[1], &config);
    let target_ids: std::collections::HashSet<u64> = results.iter().map(|r| r.target_id).collect();
    assert!(target_ids.contains(&2), "should reach node 2");
    assert!(target_ids.contains(&3), "should reach node 3");
}

#[test]
fn test_execute_match_finds_edges() {
    let (_dir, col) = make_test_collection(None);

    // Store node payloads with labels
    col.upsert_node_payload(
        10,
        &serde_json::json!({"_labels": ["Person"], "name": "Alice"}),
    )
    .unwrap();
    col.upsert_node_payload(
        20,
        &serde_json::json!({"_labels": ["Person"], "name": "Bob"}),
    )
    .unwrap();

    // Add edge: Alice -> Bob
    let edge = crate::collection::graph::GraphEdge::new(1, 10, 20, "KNOWS").unwrap();
    col.add_edge(edge).unwrap();

    // MATCH query through the GraphCollection delegate
    let match_clause = crate::velesql::MatchClause {
        patterns: vec![crate::velesql::GraphPattern {
            name: None,
            nodes: vec![
                crate::velesql::NodePattern::new().with_alias("a"),
                crate::velesql::NodePattern::new().with_alias("b"),
            ],
            relationships: vec![crate::velesql::RelationshipPattern::new(
                crate::velesql::Direction::Outgoing,
            )],
        }],
        where_clause: None,
        return_clause: crate::velesql::ReturnClause {
            items: vec![],
            order_by: None,
            limit: Some(10),
        },
    };

    let params = HashMap::new();
    let results = col.execute_match(&match_clause, &params).unwrap();
    assert!(
        !results.is_empty(),
        "execute_match should find the KNOWS edge"
    );
    assert_eq!(results[0].node_id, 20, "target should be Bob (id=20)");
}

#[test]
fn test_has_edge_and_remove_edge() {
    let (_dir, col) = make_test_collection(None);
    assert!(!col.has_edge(7), "unknown edge id is absent");
    for id in [10, 20] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }

    col.add_edge(GraphEdge::new(7, 10, 20, "KNOWS").unwrap())
        .unwrap();
    assert!(col.has_edge(7), "edge present after add");

    assert!(col.remove_edge(7), "removing an existing edge returns true");
    assert!(!col.has_edge(7), "edge gone after remove");
    assert!(!col.remove_edge(7), "removing a missing edge returns false");
}

#[test]
fn test_try_remove_edge_tells_a_removal_from_an_absent_edge() {
    let (_dir, col) = make_test_collection(None);
    for id in [10, 20] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(7, 10, 20, "KNOWS").unwrap())
        .unwrap();

    assert!(
        col.try_remove_edge(7).unwrap(),
        "an existing edge is removed"
    );
    assert!(!col.has_edge(7), "edge gone after remove");
    assert!(
        !col.try_remove_edge(7).unwrap(),
        "removing an absent edge is a no-op, not an error"
    );
}

#[cfg(feature = "persistence")]
#[test]
fn test_try_remove_edge_reports_a_refused_wal_append_as_a_storage_error() {
    let (_dir, col) = make_test_collection(None);
    for id in [10, 20] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(7, 10, 20, "KNOWS").unwrap())
        .unwrap();
    col.break_edge_wal_for_test().unwrap();

    let err = col
        .try_remove_edge(7)
        .expect_err("a refused WAL append is an error, not an absent edge");
    assert_eq!(err.code(), "VELES-006", "core codes it as storage: {err}");
    assert!(
        err.to_string().contains("write-ahead log refused"),
        "the reason reaches the caller: {err}"
    );
    assert!(col.has_edge(7), "the edge is left in place");
    assert!(
        !col.remove_edge(7),
        "the bool API folds the same failure into `false`"
    );
}

#[cfg(feature = "persistence")]
#[test]
fn test_truncate_removes_edges_then_nodes_and_counts_them() {
    let (_dir, col) = make_test_collection(None);
    for id in [10, 20, 30] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(1, 10, 20, "KNOWS").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(2, 20, 30, "KNOWS").unwrap())
        .unwrap();

    assert_eq!(col.truncate().unwrap(), (2, 3));
    assert_eq!(col.edge_count(), 0);
    assert!(col.all_node_ids().is_empty());
    assert_eq!(
        col.truncate().unwrap(),
        (0, 0),
        "an empty graph truncates to nothing"
    );
}

#[cfg(feature = "persistence")]
#[test]
fn test_truncate_keeps_the_nodes_when_an_edge_removal_is_refused() {
    let (_dir, col) = make_test_collection(None);
    for id in [10, 20] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(7, 10, 20, "KNOWS").unwrap())
        .unwrap();
    col.break_edge_wal_for_test().unwrap();

    let err = col
        .truncate()
        .expect_err("a refused edge removal fails the truncate");
    assert_eq!(err.code(), "VELES-006", "{err}");
    assert!(err.to_string().contains("failed on edge 7"), "{err}");
    assert!(col.has_edge(7), "the edge is left in place");
    assert_eq!(
        col.all_node_ids().len(),
        2,
        "no node is deleted under a surviving edge"
    );
}

#[test]
fn test_get_edge_returns_the_edge_by_id() {
    let (_dir, col) = make_test_collection(None);
    assert!(col.get_edge(7).is_none(), "unknown edge id is absent");
    for id in [10, 20] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }

    let mut props = HashMap::new();
    props.insert("since".to_string(), serde_json::json!(2020));
    col.add_edge(
        GraphEdge::new(7, 10, 20, "KNOWS")
            .unwrap()
            .with_properties(props),
    )
    .unwrap();

    let edge = col.get_edge(7).expect("edge present after add");
    assert_eq!(
        (edge.id(), edge.source(), edge.target(), edge.label()),
        (7, 10, 20, "KNOWS")
    );
    assert_eq!(edge.property("since"), Some(&serde_json::json!(2020)));

    assert!(col.remove_edge(7));
    assert!(col.get_edge(7).is_none(), "edge gone after remove");
}

#[test]
fn test_upsert_node_without_vector_stores_payload_only() {
    // No embeddings: the `None`-vector branch delegates to upsert_node_payload.
    let (_dir, col) = make_test_collection(None);
    col.upsert_node(42, &serde_json::json!({"name": "Carol"}), None)
        .unwrap();
    assert_eq!(
        col.get_node_payload(42).unwrap(),
        Some(serde_json::json!({"name": "Carol"}))
    );
    assert!(col.all_node_ids().contains(&42));
    assert!(!col.has_embeddings(), "no embeddings without a dimension");
}

#[test]
fn test_get_edges_filtered_by_label() {
    let (_dir, col) = make_test_collection(None);
    for id in [10, 20, 30, 40] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(1, 10, 20, "KNOWS").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(2, 20, 30, "LIKES").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(3, 30, 40, "KNOWS").unwrap())
        .unwrap();

    let knows = col.get_edges(Some("KNOWS"));
    assert_eq!(knows.len(), 2, "two KNOWS edges");
    assert!(knows.iter().all(|e| e.label() == "KNOWS"));

    let all = col.get_edges(None);
    assert_eq!(all.len(), 3, "three edges total");
}

#[test]
fn test_node_degree_and_directional_edges() {
    let (_dir, col) = make_test_collection(None);
    for id in [10, 20, 30, 40] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(1, 10, 20, "NEXT").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(2, 30, 20, "NEXT").unwrap())
        .unwrap();
    col.add_edge(GraphEdge::new(3, 20, 40, "NEXT").unwrap())
        .unwrap();

    // Node 20: 2 incoming (from 10, 30), 1 outgoing (to 40).
    assert_eq!(col.node_degree(20), (2, 1));
    assert_eq!(col.get_incoming(20).len(), 2);
    let outgoing = col.get_outgoing(20);
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].target(), 40);
}

#[test]
fn test_delete_removes_node_payload() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(10, &serde_json::json!({"k": 1}))
        .unwrap();
    col.upsert_node_payload(20, &serde_json::json!({"k": 2}))
        .unwrap();
    assert_eq!(col.all_node_ids().len(), 2);

    col.delete(&[10]).unwrap();
    assert!(col.get(&[10])[0].is_none(), "deleted node is gone");
    assert!(
        !col.all_node_ids().contains(&10),
        "deleted node leaves the id set"
    );
    assert!(col.get_node_payload(20).unwrap().is_some(), "node 20 stays");
}

#[test]
fn test_scroll_batch_paginates_embedded_nodes() {
    // scroll_batch iterates the point (vector) store, so use embeddings.
    let (_dir, col) = make_test_collection(Some(2));
    for id in [1u64, 2, 3] {
        col.upsert_node(id, &serde_json::json!({"id": id}), Some(vec![1.0, 0.0]))
            .unwrap();
    }
    assert!(!col.is_empty());
    assert_eq!(col.len(), 3);

    let first = col.scroll_batch(None, 2, None).unwrap();
    assert_eq!(first.points.len(), 2, "first page has 2 of 3 nodes");
    let cursor = first.next_cursor.expect("non-empty page yields a cursor");
    let second = col.scroll_batch(Some(cursor), 2, None).unwrap();
    assert_eq!(second.points.len(), 1, "second page has the last node");
    // A page past the end returns no points (and therefore no cursor).
    let tail_cursor = second.next_cursor.expect("page yields a cursor");
    let third = col.scroll_batch(Some(tail_cursor), 2, None).unwrap();
    assert!(third.points.is_empty(), "no points past the end");
    assert!(third.next_cursor.is_none(), "empty page yields no cursor");

    // batch_size 0 is rejected.
    assert!(col.scroll_batch(None, 0, None).is_err());
}

#[test]
fn test_flush_and_flush_full_succeed() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(1, &serde_json::json!({"k": 1}))
        .unwrap();
    col.upsert_node_payload(2, &serde_json::json!({})).unwrap();
    col.add_edge(GraphEdge::new(1, 1, 2, "NEXT").unwrap())
        .unwrap();
    col.flush().expect("fast-path flush succeeds");
    col.flush_full().expect("full durability flush succeeds");
}

#[test]
fn test_reopen_recovers_edges_and_payloads() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();
    {
        let col = GraphCollection::create(
            path.clone(),
            "kg",
            None,
            DistanceMetric::Cosine,
            GraphSchema::schemaless(),
        )
        .unwrap();
        col.upsert_node_payload(1, &serde_json::json!({"name": "A"}))
            .unwrap();
        col.upsert_node_payload(2, &serde_json::json!({})).unwrap();
        col.add_edge(GraphEdge::new(5, 1, 2, "NEXT").unwrap())
            .unwrap();
        col.flush_full().unwrap();
    }
    let reopened = GraphCollection::open(path).unwrap();
    assert_eq!(reopened.name(), "kg");
    assert!(reopened.has_edge(5), "edge survives reopen");
    assert_eq!(
        reopened.get_node_payload(1).unwrap(),
        Some(serde_json::json!({"name": "A"}))
    );
}

/// An edge's properties survive a snapshot: `flush` writes `edge_store.bin`
/// and truncates the edge WAL, so the reopened collection reads the edge back
/// from the snapshot alone. Properties were `serde_json::Value`s that postcard
/// cannot deserialize, so such a snapshot failed to load and the collection
/// reopened with no edge at all.
#[cfg(feature = "persistence")]
#[test]
fn test_edge_properties_survive_a_flushed_snapshot_and_reopen() {
    let (dir, col) = make_test_collection(Some(4));
    for id in [1, 2] {
        col.upsert_node(
            id,
            &serde_json::json!({"_labels": ["N"]}),
            Some(vec![1.0, 0.0, 0.0, 0.0]),
        )
        .unwrap();
    }
    let properties = HashMap::from([
        ("since".to_string(), serde_json::json!("2024")),
        ("weight".to_string(), serde_json::json!(0.5)),
        (
            "tags".to_string(),
            serde_json::json!(["a", {"nested": null}]),
        ),
    ]);
    col.add_edge(
        GraphEdge::new(7, 1, 2, "KNOWS")
            .unwrap()
            .with_properties(properties.clone()),
    )
    .unwrap();
    col.add_edge(GraphEdge::new(8, 2, 1, "PLAIN").unwrap())
        .unwrap();
    col.flush().unwrap();
    drop(col);

    let reopened = GraphCollection::open(dir.path().to_path_buf()).unwrap();

    assert_eq!(reopened.edge_count(), 2);
    let edge = reopened
        .get_edge(7)
        .expect("the edge with properties is back");
    assert_eq!(edge.properties(), &properties);
    assert!(reopened.get_edge(8).is_some());
    let reindexed = reopened
        .inner
        .graph
        .edge_range_indexes
        .read()
        .contains_key("KNOWS.since");
    assert!(
        reindexed,
        "edge properties are re-indexed from the snapshot"
    );
}

/// The files of `dir` that an open moved aside as unreadable edge snapshots.
#[cfg(feature = "persistence")]
fn quarantined_edge_snapshots(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("edge_store.bin.unreadable-"))
        })
        .collect();
    found.sort();
    found
}

/// An `edge_store.bin` that cannot be decoded is moved aside, bytes intact,
/// and the collection opens with the edges its WAL still holds: left in
/// place, the file would be overwritten by the next flush.
#[cfg(feature = "persistence")]
#[test]
fn test_an_undecodable_edge_snapshot_is_quarantined_and_the_wal_replayed() {
    const GARBAGE: &[u8] = b"\xff\xff\xff\xffnot postcard";
    let (dir, col) = make_test_collection(None);
    for id in [1, 2] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    col.add_edge(GraphEdge::new(7, 1, 2, "IN_SNAPSHOT").unwrap())
        .unwrap();
    col.flush().unwrap();
    col.add_edge(GraphEdge::new(8, 2, 1, "IN_WAL").unwrap())
        .unwrap();
    drop(col);
    std::fs::write(dir.path().join("edge_store.bin"), GARBAGE).unwrap();

    let reopened = GraphCollection::open(dir.path().to_path_buf()).unwrap();

    assert!(
        reopened.get_edge(7).is_none(),
        "the snapshot's edges are not invented"
    );
    assert!(
        reopened.get_edge(8).is_some(),
        "the WAL's edges are replayed"
    );
    assert!(!dir.path().join("edge_store.bin").exists());
    let quarantined = quarantined_edge_snapshots(dir.path());
    assert_eq!(quarantined.len(), 1, "{quarantined:?}");
    assert_eq!(std::fs::read(&quarantined[0]).unwrap(), GARBAGE);

    reopened.flush().unwrap();
    drop(reopened);
    let again = GraphCollection::open(dir.path().to_path_buf()).unwrap();
    assert!(again.get_edge(8).is_some());
    assert_eq!(
        std::fs::read(&quarantined[0]).unwrap(),
        GARBAGE,
        "a flush keeps it"
    );
}

/// A snapshot that 7.1.0 wrote with an edge property (the properties as a
/// postcard map) is quarantined on the first 8.0 open, bytes intact.
#[cfg(feature = "persistence")]
#[test]
fn test_a_snapshot_written_with_properties_by_7_1_is_quarantined_intact() {
    /// The 7.1.0 layout of `edge_store.bin`, one edge with a property.
    #[derive(serde::Serialize)]
    struct Edge71 {
        id: u64,
        source: u64,
        target: u64,
        label: String,
        properties: HashMap<String, serde_json::Value>,
    }
    #[derive(serde::Serialize)]
    struct Store71 {
        edges: HashMap<u64, Edge71>,
        outgoing: HashMap<u64, Vec<u64>>,
        incoming: HashMap<u64, Vec<u64>>,
    }
    let edge = Edge71 {
        id: 7,
        source: 1,
        target: 2,
        label: "KNOWS".to_string(),
        properties: HashMap::from([("since".to_string(), serde_json::json!("2024"))]),
    };
    let bytes = postcard::to_allocvec(&Store71 {
        edges: HashMap::from([(7, edge)]),
        outgoing: HashMap::from([(1, vec![7])]),
        incoming: HashMap::from([(2, vec![7])]),
    })
    .unwrap();
    let (dir, col) = make_test_collection(None);
    drop(col);
    std::fs::write(dir.path().join("edge_store.bin"), &bytes).unwrap();

    let reopened = GraphCollection::open(dir.path().to_path_buf()).unwrap();

    assert_eq!(reopened.edge_count(), 0);
    let quarantined = quarantined_edge_snapshots(dir.path());
    assert_eq!(quarantined.len(), 1, "{quarantined:?}");
    assert_eq!(std::fs::read(&quarantined[0]).unwrap(), bytes);
}

/// A snapshot that cannot be read at all (here a directory in its place) is
/// an I/O error: nothing is known about its content, so nothing is moved.
#[cfg(feature = "persistence")]
#[test]
fn test_an_unreadable_edge_snapshot_path_fails_the_open() {
    let (dir, col) = make_test_collection(None);
    drop(col);
    std::fs::create_dir(dir.path().join("edge_store.bin")).unwrap();

    let err = GraphCollection::open(dir.path().to_path_buf())
        .err()
        .expect("an unreadable snapshot path is an error");

    assert!(matches!(err, crate::Error::Io(_)), "{err}");
    assert!(dir.path().join("edge_store.bin").is_dir());
    assert!(quarantined_edge_snapshots(dir.path()).is_empty());
}

/// Properties too deep to be read back are refused before anything is
/// written, one edge or a batch.
#[test]
fn test_edges_with_properties_too_deep_to_read_back_are_refused() {
    let (dir, col) = make_test_collection(None);
    for id in [1, 2] {
        col.upsert_node_payload(id, &serde_json::json!({})).unwrap();
    }
    let mut deep = serde_json::json!(1);
    for _ in 0..crate::validation::MAX_VALUE_DEPTH {
        deep = serde_json::json!([deep]);
    }
    let too_deep = GraphEdge::new(7, 1, 2, "DEEP")
        .unwrap()
        .with_properties(HashMap::from([("k".to_string(), deep)]));
    let fine = GraphEdge::new(8, 1, 2, "FINE").unwrap();

    let single = col.add_edge(too_deep.clone()).unwrap_err();
    let batch = col.add_edges_batch(vec![fine, too_deep]).unwrap_err();

    assert!(
        matches!(single, crate::Error::SchemaValidation(_)),
        "{single}"
    );
    assert!(
        matches!(batch, crate::Error::SchemaValidation(_)),
        "{batch}"
    );
    assert_eq!(col.edge_count(), 0, "the batch writes nothing");
    #[cfg(feature = "persistence")]
    {
        let wal = crate::collection::graph::edge_wal::wal_path_for_edges(dir.path());
        let logged = std::fs::metadata(wal).map_or(0, |m| m.len());
        assert_eq!(logged, 0, "nothing reaches the edge WAL");
    }
    #[cfg(not(feature = "persistence"))]
    drop(dir);
}

// -------------------------------------------------------------------------
// Graph range indexes: derived from every node payload, like the label index
// -------------------------------------------------------------------------
//
// A MATCH range predicate (`n.age > 10`) takes its candidates from the
// `label.property` range index whenever that index exists, so an index that
// misses a node hides it from the result with no error. Each test asserts both
// what MATCH returns and what the index itself holds: a stale entry only
// widens the candidate set, which the WHERE re-check then narrows, so the
// index read is what tells a correct index from a merely harmless one.

/// The ids `MATCH (n:P) WHERE n.age > {floor}` returns, sorted.
fn matched_older_than(collection: &Collection, floor: i64) -> Vec<u64> {
    let sql = format!("MATCH (n:P) WHERE n.age > {floor} RETURN n LIMIT 100");
    let mut ids: Vec<u64> = collection
        .execute_query_str(&sql, &HashMap::new())
        .unwrap()
        .iter()
        .map(|result| result.point.id)
        .collect();
    ids.sort_unstable();
    ids
}

/// The ids the `P.age` range index resolves `> floor` to, sorted: the
/// candidate set the MATCH prefilter reads. Empty when no such index exists.
fn indexed_older_than(collection: &Collection, floor: i64) -> Vec<u64> {
    let mut ids = collection
        .graph_range_lookup_gt("P", "age", &serde_json::json!(floor))
        .unwrap_or_default();
    ids.sort_unstable();
    ids
}

fn assert_older_than(collection: &Collection, floor: i64, expected: &[u64]) {
    assert_eq!(
        matched_older_than(collection, floor),
        expected,
        "MATCH (n:P) WHERE n.age > {floor}"
    );
    assert_eq!(
        indexed_older_than(collection, floor),
        expected,
        "the P.age range index above {floor}"
    );
}

fn aged(age: i64) -> serde_json::Value {
    serde_json::json!({ "_labels": ["P"], "age": age })
}

#[test]
fn a_node_written_after_a_flushed_reopen_leaves_the_earlier_nodes_matchable() {
    let dir = tempdir().unwrap();
    {
        let col = GraphCollection::create(
            dir.path().to_path_buf(),
            "kg",
            None,
            DistanceMetric::Cosine,
            GraphSchema::schemaless(),
        )
        .unwrap();
        col.upsert_node_payload(1, &aged(30)).unwrap();
        col.flush().unwrap();
    }
    let col = GraphCollection::open(dir.path().to_path_buf()).unwrap();
    assert_older_than(&col.inner, 10, &[1]);

    col.upsert_node_payload(2, &aged(40)).unwrap();

    assert_older_than(&col.inner, 10, &[1, 2]);
}

#[test]
fn a_node_written_after_an_unflushed_reopen_leaves_the_replayed_nodes_matchable() {
    let dir = tempdir().unwrap();
    {
        let col = GraphCollection::create(
            dir.path().to_path_buf(),
            "kg",
            Some(4),
            DistanceMetric::Cosine,
            GraphSchema::schemaless(),
        )
        .unwrap();
        col.upsert_node_payload(1, &aged(30)).unwrap();
        col.upsert_node(2, &aged(40), Some(vec![1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        // Dropped without a flush: both payloads come back from the WAL.
    }
    let col = GraphCollection::open(dir.path().to_path_buf()).unwrap();
    assert_older_than(&col.inner, 10, &[1, 2]);

    col.upsert_node_payload(3, &aged(50)).unwrap();

    assert_older_than(&col.inner, 10, &[1, 2, 3]);
}

#[test]
fn a_node_upserted_with_a_vector_is_matched_by_a_range_predicate() {
    let (_dir, col) = make_test_collection(Some(4));
    col.upsert_node_payload(1, &aged(30)).unwrap();

    col.upsert_node(2, &aged(40), Some(vec![1.0, 0.0, 0.0, 0.0]))
        .unwrap();

    assert_older_than(&col.inner, 10, &[1, 2]);
}

#[test]
fn points_upserted_in_a_batch_are_matched_by_a_range_predicate() {
    let (_dir, col) = make_test_collection(Some(4));
    col.inner
        .upsert([
            Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(aged(30))),
            Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(aged(40))),
        ])
        .unwrap();

    col.upsert_node_payload(3, &aged(50)).unwrap();

    assert_older_than(&col.inner, 10, &[1, 2, 3]);
}

#[test]
fn a_node_upserted_as_metadata_is_matched_by_a_range_predicate() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(1, &aged(30)).unwrap();

    col.inner
        .upsert_metadata([Point::metadata_only(2, aged(40))])
        .unwrap();

    assert_older_than(&col.inner, 10, &[1, 2]);
}

#[test]
fn a_node_bulk_upserted_is_matched_by_a_range_predicate() {
    let (_dir, col) = make_test_collection(Some(4));
    col.upsert_node_payload(1, &aged(30)).unwrap();

    col.inner
        .upsert_bulk(&[Point::new(2, vec![1.0, 0.0, 0.0, 0.0], Some(aged(40)))])
        .unwrap();

    assert_older_than(&col.inner, 10, &[1, 2]);
}

#[test]
fn a_node_bulk_upserted_from_raw_vectors_is_matched_by_a_range_predicate() {
    let (_dir, col) = make_test_collection(Some(4));
    col.upsert_node_payload(1, &aged(30)).unwrap();

    col.inner
        .upsert_bulk_from_raw(&[1.0, 0.0, 0.0, 0.0], &[2], 4, Some(&[Some(aged(40))]))
        .unwrap();

    assert_older_than(&col.inner, 10, &[1, 2]);
}

#[test]
fn an_update_moves_one_node_out_of_the_range_and_another_into_it() {
    let (_dir, col) = make_test_collection(Some(4));
    col.upsert_node_payload(1, &aged(30)).unwrap();
    col.upsert_node_payload(2, &aged(50)).unwrap();
    col.upsert_node_payload(3, &aged(5)).unwrap();

    col.upsert_node(1, &aged(5), Some(vec![1.0, 0.0, 0.0, 0.0]))
        .unwrap();
    col.upsert_node(3, &aged(70), Some(vec![0.0, 1.0, 0.0, 0.0]))
        .unwrap();

    assert_older_than(&col.inner, 10, &[2, 3]);
}

#[test]
fn a_deleted_node_leaves_the_range_index() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(1, &aged(30)).unwrap();
    col.upsert_node_payload(2, &aged(40)).unwrap();

    col.delete(&[1]).unwrap();

    assert_older_than(&col.inner, 10, &[2]);
}

#[test]
fn a_node_deleted_from_a_metadata_only_collection_leaves_the_range_index() {
    let dir = tempdir().unwrap();
    let collection = Collection::create_metadata_only(dir.path().to_path_buf(), "meta").unwrap();
    collection.store_node_payload(1, &aged(30)).unwrap();
    collection.store_node_payload(2, &aged(40)).unwrap();

    collection.delete(&[1]).unwrap();

    assert_older_than(&collection, 10, &[2]);
}

// -------------------------------------------------------------------------
// The range prefilter is a superset of what WHERE accepts
// -------------------------------------------------------------------------
//
// The prefilter only rejects, so every node it drops must be one WHERE would
// have rejected too. Each case below is one where the index ordering and the
// WHERE comparison used to disagree.

/// The ids `MATCH (n:P) [WHERE {condition}]` returns, sorted.
fn matched_where(collection: &Collection, condition: Option<&str>) -> Vec<u64> {
    let filter = condition.map_or_else(String::new, |c| format!("WHERE {c} "));
    let sql = format!("MATCH (n:P) {filter}RETURN n LIMIT 100");
    let mut ids: Vec<u64> = collection
        .execute_query_str(&sql, &HashMap::new())
        .unwrap()
        .iter()
        .map(|result| result.point.id)
        .collect();
    ids.sort_unstable();
    ids
}

fn labelled(properties: serde_json::Value) -> serde_json::Value {
    let mut payload = properties;
    payload["_labels"] = serde_json::json!(["P"]);
    payload
}

#[test]
fn a_float_equality_finds_every_value_within_the_where_tolerance() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(1, &labelled(serde_json::json!({ "w": 0.5004 })))
        .unwrap();
    col.upsert_node_payload(2, &labelled(serde_json::json!({ "w": 0.5 })))
        .unwrap();
    col.upsert_node_payload(3, &labelled(serde_json::json!({ "w": 0.7 })))
        .unwrap();

    assert_eq!(matched_where(&col.inner, Some("n.w = 0.5")), vec![1, 2]);
}

#[test]
fn an_integer_bound_above_2_pow_53_does_not_hide_the_next_integer() {
    let (_dir, col) = make_test_collection(None);
    // 2^53 + 1 rounds onto the bound 2^53 as an f64; 2^53 + 10 does not.
    col.upsert_node_payload(1, &aged(9_007_199_254_740_993))
        .unwrap();
    col.upsert_node_payload(2, &aged(9_007_199_254_741_002))
        .unwrap();

    assert_eq!(
        matched_where(&col.inner, Some("n.age > 9007199254740992")),
        vec![1, 2]
    );
    assert_eq!(
        matched_where(&col.inner, Some("n.age < 9007199254741002")),
        vec![1]
    );
}

#[test]
fn a_negative_zero_satisfies_a_zero_bound() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(1, &labelled(serde_json::json!({ "w": -0.0 })))
        .unwrap();
    col.upsert_node_payload(2, &labelled(serde_json::json!({ "w": 1.5 })))
        .unwrap();

    assert_eq!(matched_where(&col.inner, Some("n.w >= 0.0")), vec![1, 2]);
}

#[test]
fn a_top_level_dotted_key_does_not_hide_a_nested_property() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(1, &labelled(serde_json::json!({ "a": { "b": 5 } })))
        .unwrap();
    // WHERE reads `n.a.b` as a nested path, which this node does not have.
    col.upsert_node_payload(2, &labelled(serde_json::json!({ "a.b": 7 })))
        .unwrap();

    assert_eq!(matched_where(&col.inner, Some("n.a.b > 1")), vec![1]);
    assert!(
        !col.inner
            .graph
            .graph_range_indexes
            .read()
            .contains_key("P.a.b"),
        "a dotted top-level key is unreachable by WHERE and must not be indexed"
    );
}

#[test]
fn only_scalar_properties_are_range_indexed() {
    let (_dir, col) = make_test_collection(None);
    col.upsert_node_payload(
        1,
        &labelled(serde_json::json!({
            "age": 30,
            "name": "ada",
            "gone": null,
            "tags": [1, 2],
            "meta": { "x": 1 },
        })),
    )
    .unwrap();

    let mut keys: Vec<String> = col
        .inner
        .graph
        .graph_range_indexes
        .read()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(keys, ["P.age", "P.gone", "P.name"]);
    // An array or an object never satisfies an index-eligible predicate.
    assert!(matched_where(&col.inner, Some("n.tags = 1")).is_empty());
    assert_eq!(matched_where(&col.inner, Some("n.age = 30")), vec![1]);
}
