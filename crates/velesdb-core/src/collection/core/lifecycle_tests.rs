#![cfg(all(test, feature = "persistence"))]

use crate::collection::types::CollectionConfig;
use crate::distance::DistanceMetric;
use crate::index::hnsw::HnswParams;
use crate::quantization::StorageMode;
use crate::Collection;
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

/// Maximum valid dimension (65,536) must be accepted.
#[test]
fn test_create_accepts_max_dimension() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let result = Collection::create(
        PathBuf::from(temp_dir.path()),
        65_536,
        DistanceMetric::Cosine,
    );
    assert!(result.is_ok(), "dimension 65_536 should be accepted");
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
