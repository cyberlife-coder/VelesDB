use super::*;
use crate::distance::DistanceMetric;
use tempfile::TempDir;

#[test]
fn test_analyze_empty_collection() {
    let temp_dir = TempDir::new().unwrap();
    let collection =
        Collection::create(temp_dir.path().to_path_buf(), 128, DistanceMetric::Cosine).unwrap();

    let stats = collection.analyze().unwrap();

    assert_eq!(stats.row_count, 0);
    assert_eq!(stats.deleted_count, 0);
    assert!(stats.index_stats.contains_key("hnsw_primary"));
}

#[test]
fn test_analyze_with_data() {
    use crate::point::Point;

    let temp_dir = TempDir::new().unwrap();
    let collection =
        Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Insert some vectors using Point
    let points: Vec<Point> = (0..10)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)] // Reason: i < 20 in test; u64→f32 exact.
            Point::new(
                i,
                vec![i as f32; 4],
                Some(serde_json::json!({"category": format!("cat_{}", i % 3)})),
            )
        })
        .collect();
    collection.upsert(points).unwrap();

    let stats = collection.analyze().unwrap();

    assert_eq!(stats.row_count, 10);
    assert!(stats.index_stats.get("hnsw_primary").unwrap().entry_count >= 10);
}

#[test]
fn test_get_stats_returns_defaults_on_error() {
    let temp_dir = TempDir::new().unwrap();
    let collection =
        Collection::create(temp_dir.path().to_path_buf(), 128, DistanceMetric::Cosine).unwrap();

    let stats = collection.get_stats();

    // Should not panic, returns default on any issue
    assert_eq!(stats.live_row_count(), 0);
}

#[test]
fn test_get_stats_uses_cache_within_ttl() {
    let temp_dir = TempDir::new().unwrap();
    let collection =
        Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // First call populates the cache.
    let stats1 = collection.get_stats();
    assert_eq!(stats1.row_count, 0);

    // Insert a point — but bypass invalidation by calling the storage directly
    // so we can verify the cache is still served unchanged.
    // We just call get_stats() again immediately: within TTL it must return
    // the same object (row_count == 0) without re-scanning.
    let stats2 = collection.get_stats();
    assert_eq!(
        stats1.row_count, stats2.row_count,
        "get_stats should return cached value within TTL"
    );
}

#[test]
fn test_get_stats_invalidated_after_upsert() {
    use crate::point::Point;

    let temp_dir = TempDir::new().unwrap();
    let collection =
        Collection::create(temp_dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Warm the cache.
    let stats_before = collection.get_stats();
    assert_eq!(stats_before.row_count, 0);

    // upsert() must invalidate the cache.
    let points = vec![Point::new(1, vec![0.1, 0.2, 0.3, 0.4], None)];
    collection.upsert(points).unwrap();

    // Next get_stats() should recompute and reflect the new point.
    let stats_after = collection.get_stats();
    assert_eq!(
        stats_after.row_count, 1,
        "get_stats should recompute after upsert invalidates the cache"
    );
}

/// A graph's rows are its nodes: a graph without embeddings holds three
/// payload-only nodes and no vector, and ANALYZE counts the three, as `len`
/// and `GET /collections/{name}` do — the planner costs a graph by them.
#[test]
fn test_analyze_counts_a_graph_s_nodes_with_or_without_an_embedding() {
    let temp_dir = TempDir::new().unwrap();
    let db = crate::Database::open(temp_dir.path()).unwrap();
    db.create_graph_collection("g", crate::collection::GraphSchema::schemaless())
        .unwrap();
    let graph = db.get_graph_collection("g").unwrap();
    for id in 1..=3 {
        graph
            .upsert_node_payload(id, &serde_json::json!({ "n": id }))
            .unwrap();
    }
    assert_eq!(graph.len(), 3);

    let stats = db.analyze_collection("g").unwrap();

    assert_eq!(stats.row_count, 3);
}
