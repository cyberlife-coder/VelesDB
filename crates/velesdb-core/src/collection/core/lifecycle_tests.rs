#![cfg(all(test, feature = "persistence"))]

use crate::collection::types::CollectionConfig;
use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::index::hnsw::HnswParams;
use crate::quantization::StorageMode;
use std::path::PathBuf;

/// Verifies that custom HNSW params survive config round-trip serialization.
#[test]
fn test_hnsw_params_persisted_in_config_json() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let params = HnswParams::custom(64, 400, 50_000);

    let collection = Collection::create_with_hnsw_params(
        PathBuf::from(temp_dir.path()),
        128,
        DistanceMetric::Cosine,
        StorageMode::Full,
        params,
    )
    .expect("collection should be created");

    // Verify in-memory config holds the params
    let cfg = collection.config();
    assert_eq!(cfg.hnsw_params, Some(params));

    // Read config.json back from disk and verify round-trip
    let config_path = temp_dir.path().join("config.json");
    let raw = std::fs::read_to_string(&config_path).expect("config.json should exist");
    let deserialized: CollectionConfig =
        serde_json::from_str(&raw).expect("config.json should deserialize");
    assert_eq!(deserialized.hnsw_params, Some(params));
}

/// Verifies backward compatibility: config.json files without hnsw_params
/// deserialize to `None`.
#[test]
fn test_config_without_hnsw_params_loads_as_none() {
    let json = r#"{
        "name": "legacy",
        "dimension": 128,
        "metric": "Cosine",
        "point_count": 0,
        "storage_mode": "full",
        "metadata_only": false
    }"#;

    let cfg: CollectionConfig =
        serde_json::from_str(json).expect("legacy config should deserialize");
    assert!(cfg.hnsw_params.is_none());
}

/// Verifies that reopening a collection without hnsw.bin uses persisted
/// custom HNSW params instead of defaults.
#[test]
fn test_reopen_collection_uses_persisted_hnsw_params() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let params = HnswParams::custom(64, 400, 50_000);

    // Create collection with custom params (no vectors inserted, so no hnsw.bin)
    let _collection = Collection::create_with_hnsw_params(
        PathBuf::from(temp_dir.path()),
        128,
        DistanceMetric::Cosine,
        StorageMode::Full,
        params,
    )
    .expect("collection should be created");

    // Ensure hnsw.bin does NOT exist (empty collection)
    assert!(
        !temp_dir.path().join("hnsw.bin").exists(),
        "hnsw.bin should not exist for empty collection"
    );

    // Reopen the collection — should pick up custom params from config
    let reopened =
        Collection::open(PathBuf::from(temp_dir.path())).expect("collection should reopen");

    let cfg = reopened.config();
    assert_eq!(
        cfg.hnsw_params,
        Some(params),
        "reopened collection should preserve custom HNSW params"
    );
}

/// Collections created without custom HNSW params should have hnsw_params = None
/// and their config.json should NOT contain the field (skip_serializing_if).
#[test]
fn test_default_collection_omits_hnsw_params_from_json() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");

    let _collection =
        Collection::create(PathBuf::from(temp_dir.path()), 128, DistanceMetric::Cosine)
            .expect("collection should be created");

    let config_path = temp_dir.path().join("config.json");
    let raw = std::fs::read_to_string(&config_path).expect("config.json should exist");

    assert!(
        !raw.contains("hnsw_params"),
        "config.json should not contain hnsw_params when None"
    );
}

// ── Dimension validation tests (VELES-032) ──────────────────────────

/// Helper: extracts the error from a `Result<Collection, Error>`, panicking
/// if the result is `Ok`. We cannot use `unwrap_err` because `Collection`
/// does not implement `Debug`.
fn expect_err(result: crate::error::Result<Collection>) -> crate::Error {
    match result {
        Err(e) => e,
        Ok(_) => panic!("expected Err, got Ok"),
    }
}

/// Dimension 0 must be rejected.
#[test]
fn test_create_rejects_zero_dimension() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let result = Collection::create(PathBuf::from(temp_dir.path()), 0, DistanceMetric::Cosine);
    let err = expect_err(result);
    assert_eq!(err.code(), "VELES-032");
}

/// Dimension above `MAX_DIMENSION` must be rejected.
#[test]
fn test_create_rejects_oversized_dimension() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let result = Collection::create(
        PathBuf::from(temp_dir.path()),
        100_000,
        DistanceMetric::Cosine,
    );
    let err = expect_err(result);
    assert_eq!(err.code(), "VELES-032");
}

/// Minimum valid dimension (1) must be accepted.
#[test]
fn test_create_accepts_min_dimension() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let result = Collection::create(PathBuf::from(temp_dir.path()), 1, DistanceMetric::Cosine);
    assert!(result.is_ok(), "dimension 1 should be accepted");
}

/// Maximum valid dimension (65,536) must pass validation.
///
/// Note: we test `validate_dimension` directly rather than `Collection::create`
/// because allocating a full HNSW index at dim=65536 may exceed CI runner memory.
#[test]
fn test_create_accepts_max_dimension() {
    use crate::validation::validate_dimension;

    assert!(
        validate_dimension(65_536).is_ok(),
        "dimension 65_536 should pass validation"
    );

    // Also verify the boundary: 65_537 must be rejected.
    assert!(
        validate_dimension(65_537).is_err(),
        "dimension 65_537 should be rejected"
    );
}

/// `create_with_hnsw_params` must also validate dimension.
#[test]
fn test_create_with_hnsw_params_rejects_zero_dimension() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let params = HnswParams::custom(16, 200, 10_000);
    let result = Collection::create_with_hnsw_params(
        PathBuf::from(temp_dir.path()),
        0,
        DistanceMetric::Cosine,
        StorageMode::Full,
        params,
    );
    let err = expect_err(result);
    assert_eq!(err.code(), "VELES-032");
}

/// Graph collection with `Some(0)` embedding dim must be rejected.
#[test]
fn test_graph_collection_rejects_zero_embedding_dim() {
    use crate::collection::graph::GraphSchema;

    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let schema = GraphSchema::new();
    let result = Collection::create_graph_collection(
        PathBuf::from(temp_dir.path()),
        "test_graph",
        schema,
        Some(0),
        DistanceMetric::Cosine,
    );
    let err = expect_err(result);
    assert_eq!(err.code(), "VELES-032");
}

/// Graph collection with `None` embedding dim must be accepted (no vectors).
#[test]
fn test_graph_collection_accepts_none_embedding_dim() {
    use crate::collection::graph::GraphSchema;

    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let schema = GraphSchema::new();
    let result = Collection::create_graph_collection(
        PathBuf::from(temp_dir.path()),
        "test_graph",
        schema,
        None,
        DistanceMetric::Cosine,
    );
    assert!(
        result.is_ok(),
        "embedding_dim None should be accepted for graph collections"
    );
}

// ── Regression: stale point_count on reopen ──────────────────────────

/// Regression test: `Collection::open()` must reconcile `point_count` from
/// actual vector storage, not trust the (potentially stale) `config.json`.
///
/// Before the fix, `config.json` was only written at creation time (count=0)
/// and on explicit `flush()`. If the process exited after `upsert()` but
/// before `flush()`, the reopened collection would report `len() == 0`
/// despite vectors being present on disk.
#[test]
fn test_reopen_collection_reconciles_point_count_from_storage() {
    use crate::point::Point;

    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let n = 25_usize;

    // 1. Create collection and upsert N points
    let collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection should be created");

    #[allow(clippy::cast_precision_loss)]
    let points: Vec<Point> = (0..n)
        .map(|i| {
            let f = i as f32 / n as f32;
            Point::without_payload(i as u64, vec![f, 1.0 - f, 0.5, 0.1])
        })
        .collect();
    collection.upsert(points).expect("upsert should succeed");

    // Verify in-memory state is correct before drop
    assert_eq!(collection.len(), n, "in-memory len should equal N");

    // 2. Intentionally do NOT call flush() — simulates an unclean shutdown
    //    where config.json still has the stale count from creation (0).
    drop(collection);

    // 3. Reopen and verify reconciliation
    let reopened =
        Collection::open(PathBuf::from(temp_dir.path())).expect("collection should reopen");

    assert_eq!(
        reopened.config().point_count,
        n,
        "config.point_count must be reconciled from storage on open"
    );
    assert_eq!(
        reopened.len(),
        n,
        "len() must reflect actual vector count after reopen"
    );
}

// ── Bug B0.3: flush() must drain delta buffer ────────────────────────

/// Regression test: `flush()` must drain the delta buffer into HNSW before
/// persisting the index. Without this, a graceful shutdown during an active
/// rebuild loses buffered vectors — they exist in vector storage but are
/// absent from the persisted HNSW graph.
#[test]
fn test_flush_drains_delta_buffer_into_hnsw() {
    use crate::index::VectorIndex;
    use crate::point::Point;

    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection should be created");

    // 1. Insert initial points so the HNSW index has some content
    let initial_points = vec![
        Point::without_payload(1, vec![1.0, 0.0, 0.0, 0.0]),
        Point::without_payload(2, vec![0.0, 1.0, 0.0, 0.0]),
    ];
    collection.upsert(initial_points).expect("initial upsert");

    // 2. Store vectors in MmapStorage first (real application flow: vectors
    //    are persisted to storage before being delta-buffered).
    {
        use crate::storage::VectorStorage;
        let mut vs = collection.vector_storage.write();
        vs.store(10, &[0.5, 0.5, 0.0, 0.0]).expect("store 10");
        vs.store(11, &[0.0, 0.0, 0.5, 0.5]).expect("store 11");
    }

    // 3. Activate delta buffer (simulates an HNSW rebuild starting)
    collection.delta_buffer.activate();
    assert!(
        collection.delta_buffer.is_active(),
        "delta should be active"
    );

    // 4. Push vectors into the delta buffer (simulates upserts during rebuild)
    collection.delta_buffer.push(10, vec![0.5, 0.5, 0.0, 0.0]);
    collection.delta_buffer.push(11, vec![0.0, 0.0, 0.5, 0.5]);
    assert_eq!(
        collection.delta_buffer.len(),
        2,
        "delta should hold 2 entries"
    );

    // 5. Call flush — this should drain the delta buffer into HNSW
    collection.flush().expect("flush should succeed");

    // 6. Verify: delta buffer is now empty and inactive
    assert!(
        !collection.delta_buffer.is_active(),
        "delta buffer must be inactive after flush"
    );
    assert!(
        collection.delta_buffer.is_empty(),
        "delta buffer must be empty after flush"
    );

    // 7. Verify: the drained vectors are now in the HNSW index (searchable)
    let results = collection.index.search(&[0.5, 0.5, 0.0, 0.0], 5);
    let result_ids: Vec<u64> = results.iter().map(|r| r.id).collect();
    assert!(
        result_ids.contains(&10),
        "id=10 should be in HNSW after flush (was: {result_ids:?})"
    );
    assert!(
        result_ids.contains(&11),
        "id=11 should be in HNSW after flush (was: {result_ids:?})"
    );
}

/// Regression test: `flush()` on a collection with an inactive (empty) delta
/// buffer must succeed without errors and behave identically to before.
#[test]
fn test_flush_with_inactive_delta_buffer_is_noop() {
    use crate::point::Point;

    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection should be created");

    let points = vec![Point::without_payload(1, vec![1.0, 0.0, 0.0, 0.0])];
    collection.upsert(points).expect("upsert");

    // Delta buffer is NOT active — flush should behave normally
    assert!(!collection.delta_buffer.is_active());

    collection
        .flush()
        .expect("flush with inactive delta should succeed");

    // Verify the collection is still functional
    let results = collection.search(&[1.0, 0.0, 0.0, 0.0], 1).expect("search");
    assert_eq!(results.len(), 1, "search should still work after flush");
}

// ── WP-0C: EdgeStore gap recovery when edge_store.bin missing ───────

/// Regression test (WP-0C): `Collection::open()` must recover gracefully when
/// `edge_store.bin` does not exist on disk.
///
/// Collections created before the BUG-1 persistence fix never had an
/// `edge_store.bin` file. When such a collection is reopened, the
/// `load_or_default` pattern in `load_edge_store()` must fall back to an
/// empty `ConcurrentEdgeStore`, and graph operations must work immediately.
#[test]
fn test_open_without_edge_store_bin_recovers_gracefully() {
    use crate::collection::graph::{GraphEdge, GraphSchema};

    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let col_path = temp_dir.path().join("graph_col");

    // 1. Create a graph collection, flush to persist config.json, then drop.
    {
        let schema = GraphSchema::new();
        let collection = Collection::create_graph_collection(
            col_path.clone(),
            "graph_col",
            schema,
            None,
            DistanceMetric::Cosine,
        )
        .expect("graph collection should be created");

        let edge = GraphEdge::new(1, 100, 200, "KNOWS").expect("valid edge");
        collection.add_edge(edge).expect("add edge should succeed");

        collection.flush().expect("flush should succeed");
    }

    // 2. Verify edge_store.bin was created by flush, then delete it to
    //    simulate a pre-fix collection directory.
    let edge_store_path = col_path.join("edge_store.bin");
    assert!(
        edge_store_path.exists(),
        "edge_store.bin should exist after flush"
    );
    std::fs::remove_file(&edge_store_path).expect("remove edge_store.bin");
    assert!(
        !edge_store_path.exists(),
        "edge_store.bin should be deleted"
    );

    // 3. Reopen the collection — must NOT fail despite missing edge_store.bin.
    let reopened = Collection::open(col_path).expect("open should succeed without edge_store.bin");

    // 4. The recovered edge store should be empty (edges from before are lost,
    //    which is expected for pre-fix collections).
    assert_eq!(
        reopened.edge_count(),
        0,
        "edge store should be empty after recovery without edge_store.bin"
    );

    // 5. Graph operations must work on the recovered collection.
    let edge_a = GraphEdge::new(10, 1, 2, "LIKES").expect("valid edge");
    reopened
        .add_edge(edge_a)
        .expect("add edge should succeed after recovery");
    assert_eq!(reopened.edge_count(), 1, "edge count after add");

    let outgoing = reopened.get_outgoing_edges(1);
    assert_eq!(outgoing.len(), 1, "should have one outgoing edge");
    assert_eq!(outgoing[0].target(), 2, "target should be 2");
    assert_eq!(outgoing[0].label(), "LIKES", "label should be LIKES");

    // 6. Verify flush works on the recovered collection.
    reopened
        .flush()
        .expect("flush should succeed after recovery");
    assert!(
        edge_store_path.exists(),
        "edge_store.bin should be re-created after flush"
    );
}

// ── WP-2I: Schema version tests ────────────────────────────────────

/// New collections must have `schema_version == CURRENT_SCHEMA_VERSION`.
#[test]
fn test_schema_version_set_on_new_collection() {
    use crate::collection::types::CURRENT_SCHEMA_VERSION;

    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection should be created");

    let cfg = collection.config();
    assert_eq!(
        cfg.schema_version, CURRENT_SCHEMA_VERSION,
        "new collection must carry the current schema version"
    );
}

/// `schema_version` must survive a config.json round-trip.
#[test]
fn test_schema_version_persisted_in_config_json() {
    use crate::collection::types::CURRENT_SCHEMA_VERSION;

    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let _collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection should be created");

    let raw = std::fs::read_to_string(temp_dir.path().join("config.json"))
        .expect("config.json should exist");
    let deserialized: CollectionConfig =
        serde_json::from_str(&raw).expect("config.json should deserialize");
    assert_eq!(deserialized.schema_version, CURRENT_SCHEMA_VERSION);
}

/// Old config.json files without `schema_version` must deserialize to 1.
#[test]
fn test_schema_version_defaults_to_1_for_legacy_config() {
    let json = r#"{
        "name": "legacy_no_version",
        "dimension": 128,
        "metric": "Cosine",
        "point_count": 0,
        "storage_mode": "full",
        "metadata_only": false
    }"#;

    let cfg: CollectionConfig =
        serde_json::from_str(json).expect("legacy config should deserialize");
    assert_eq!(
        cfg.schema_version, 1,
        "missing schema_version must default to 1"
    );
}

/// Opening a collection with a future schema version must return VELES-036.
#[test]
fn test_open_rejects_future_schema_version() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");

    // 1. Create a valid collection so that storage files exist
    let collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection should be created");
    collection.flush().expect("flush should succeed");
    drop(collection);

    // 2. Tamper with config.json: set schema_version to 999
    let config_path = temp_dir.path().join("config.json");
    let raw = std::fs::read_to_string(&config_path).expect("read config");
    let mut cfg: serde_json::Value = serde_json::from_str(&raw).expect("parse config");
    cfg["schema_version"] = serde_json::Value::from(999);
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&cfg).expect("serialize"),
    )
    .expect("write tampered config");

    // 3. Attempt to open — must fail with VELES-036
    let result = Collection::open(PathBuf::from(temp_dir.path()));
    let err = expect_err(result);
    assert_eq!(err.code(), "VELES-036");
    let msg = err.to_string();
    assert!(
        msg.contains("999"),
        "error must mention the found version: {msg}"
    );
}

/// `schema_version == 0` must be treated as v1 (silent migration).
#[test]
fn test_schema_version_zero_treated_as_v1() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");

    // Create a valid collection
    let collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection should be created");
    collection.flush().expect("flush should succeed");
    drop(collection);

    // Set schema_version to 0 in config.json
    let config_path = temp_dir.path().join("config.json");
    let raw = std::fs::read_to_string(&config_path).expect("read config");
    let mut cfg: serde_json::Value = serde_json::from_str(&raw).expect("parse config");
    cfg["schema_version"] = serde_json::Value::from(0);
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&cfg).expect("serialize"),
    )
    .expect("write config with version 0");

    // Opening must succeed (0 is treated as v1)
    let _reopened = Collection::open(PathBuf::from(temp_dir.path()))
        .expect("collection with schema_version=0 should open");
}

/// `IncompatibleSchemaVersion` is not recoverable.
#[test]
fn test_incompatible_schema_version_is_not_recoverable() {
    let err = crate::Error::IncompatibleSchemaVersion {
        found: 99,
        supported: 1,
    };
    assert!(
        !err.is_recoverable(),
        "IncompatibleSchemaVersion must not be recoverable"
    );
}
