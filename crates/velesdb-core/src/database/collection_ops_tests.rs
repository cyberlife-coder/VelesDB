use super::*;
use crate::point::Point;
use crate::{CollectionType, DistanceMetric};
use tempfile::tempdir;

// =========================================================================
// Create + Get + Delete lifecycle
// =========================================================================

#[test]
fn test_create_get_delete_lifecycle() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // Create
    db.create_collection("lifecycle", 128, DistanceMetric::Cosine)
        .unwrap();
    assert_eq!(db.list_collections(), vec!["lifecycle"]);

    // Get
    let coll = db.get_vector_collection("lifecycle");
    assert!(coll.is_some());
    assert_eq!(coll.unwrap().config().dimension, 128);

    // Delete
    db.delete_collection("lifecycle").unwrap();
    assert!(db.list_collections().is_empty());
    assert!(db.get_vector_collection("lifecycle").is_none());
}

// =========================================================================
// Duplicate creation
// =========================================================================

#[test]
fn test_create_duplicate_returns_collection_exists() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("dup", 64, DistanceMetric::Euclidean)
        .unwrap();

    let err = db
        .create_collection("dup", 64, DistanceMetric::Euclidean)
        .unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}

#[test]
fn test_create_vector_duplicate_returns_error() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_vector_collection("vec_dup", 32, DistanceMetric::Cosine)
        .unwrap();

    let err = db
        .create_vector_collection("vec_dup", 32, DistanceMetric::Cosine)
        .unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}

#[test]
fn test_create_metadata_duplicate_returns_error() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_metadata_collection("meta_dup").unwrap();

    let err = db.create_metadata_collection("meta_dup").unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}

// =========================================================================
// Nonexistent lookups
// =========================================================================

#[test]
fn test_get_nonexistent_returns_none() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    assert!(db.get_any_collection("nope").is_none());
    assert!(db.get_vector_collection("nope").is_none());
    assert!(db.get_graph_collection("nope").is_none());
    assert!(db.get_metadata_collection("nope").is_none());
}

#[test]
fn test_delete_nonexistent_returns_not_found() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let err = db.delete_collection("absent").unwrap_err();
    assert!(matches!(err, crate::Error::CollectionNotFound(_)));
}

// =========================================================================
// Multi-collection isolation
// =========================================================================

#[test]
fn test_multi_collection_data_isolation() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("coll_a", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("coll_b", 4, DistanceMetric::Euclidean)
        .unwrap();

    // Insert into coll_a only.
    let coll_a = db.get_vector_collection("coll_a").unwrap();
    coll_a
        .upsert(vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"source": "a"})),
        )])
        .unwrap();

    // coll_b must remain empty.
    let coll_b = db.get_vector_collection("coll_b").unwrap();
    assert!(coll_b.get(&[1]).into_iter().flatten().next().is_none());
}

#[test]
fn test_list_collections_returns_sorted_names() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("zebra", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("alpha", 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("middle", 4, DistanceMetric::Cosine)
        .unwrap();

    let names = db.list_collections();
    assert_eq!(names, vec!["alpha", "middle", "zebra"]);
}

// =========================================================================
// Typed collection creation via create_collection_typed
// =========================================================================

#[test]
fn test_create_collection_typed_metadata_only() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    assert!(db.get_metadata_collection("products").is_some());
    assert_eq!(db.list_collections(), vec!["products"]);
}

#[test]
fn test_create_collection_typed_vector() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let kind = CollectionType::Vector {
        dimension: 128,
        metric: DistanceMetric::DotProduct,
        storage_mode: crate::StorageMode::Full,
    };
    db.create_collection_typed("embeddings", &kind).unwrap();

    let vc = db.get_vector_collection("embeddings").unwrap();
    assert_eq!(vc.inner.config().dimension, 128);
    assert_eq!(vc.inner.config().metric, DistanceMetric::DotProduct);
}

// =========================================================================
// Delete cleans up disk directory
// =========================================================================

#[test]
fn test_delete_collection_removes_disk_directory() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection("cleanup", 4, DistanceMetric::Cosine)
        .unwrap();
    let coll_path = dir.path().join("cleanup");
    assert!(coll_path.exists(), "collection dir should exist on disk");

    db.delete_collection("cleanup").unwrap();
    assert!(
        !coll_path.exists(),
        "collection dir should be removed after delete"
    );
}

// =========================================================================
// Cross-type name collision
// =========================================================================

#[test]
fn test_name_collision_across_collection_types() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_metadata_collection("shared_name").unwrap();

    // Attempting to create a vector collection with the same name must fail.
    let err = db
        .create_vector_collection("shared_name", 64, DistanceMetric::Cosine)
        .unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}

// =========================================================================
// Collection name validation — path traversal prevention (issue #381)
// =========================================================================

/// Helper: asserts that creating a collection with the given name produces
/// `InvalidCollectionName`.
fn assert_name_rejected(db: &Database, name: &str) {
    let err = db
        .create_collection(name, 4, DistanceMetric::Cosine)
        .unwrap_err();
    assert!(
        matches!(err, crate::Error::InvalidCollectionName { .. }),
        "Expected InvalidCollectionName for {:?}, got: {err}",
        name,
    );
}

#[test]
fn test_reject_empty_name() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert_name_rejected(&db, "");
}

#[test]
fn test_reject_dot_names() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert_name_rejected(&db, ".");
    assert_name_rejected(&db, "..");
}

#[test]
fn test_reject_path_traversal_unix() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert_name_rejected(&db, "../etc/evil");
    assert_name_rejected(&db, "../../passwd");
    assert_name_rejected(&db, "foo/bar");
    assert_name_rejected(&db, "a/b/c");
}

#[test]
fn test_reject_path_traversal_windows() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert_name_rejected(&db, r"..\evil");
    assert_name_rejected(&db, r"foo\bar");
    assert_name_rejected(&db, r"C:\Windows");
}

#[test]
fn test_reject_path_separators() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert_name_rejected(&db, "name/with/slash");
    assert_name_rejected(&db, "name\\with\\backslash");
}

#[test]
fn test_reject_special_characters() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert_name_rejected(&db, "name with spaces");
    assert_name_rejected(&db, "name@special");
    assert_name_rejected(&db, "name.dot");
    assert_name_rejected(&db, "name#hash");
    assert_name_rejected(&db, "name$dollar");
    assert_name_rejected(&db, "name:colon");
    assert_name_rejected(&db, "name*star");
    assert_name_rejected(&db, "name?question");
    assert_name_rejected(&db, "name<angle>");
    assert_name_rejected(&db, "name|pipe");
    assert_name_rejected(&db, "name\"quote");
}

#[test]
fn test_reject_unicode() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert_name_rejected(&db, "café");
    assert_name_rejected(&db, "日本語");
    assert_name_rejected(&db, "коллекция");
}

#[test]
fn test_reject_leading_hyphen() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert_name_rejected(&db, "-leading");
    assert_name_rejected(&db, "--double");
}

#[test]
fn test_reject_too_long_name() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let long_name = "a".repeat(129);
    assert_name_rejected(&db, &long_name);
}

#[test]
fn test_reject_windows_reserved_names() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    for reserved in &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM9", "LPT1", "LPT9", "con", "Con", "aux",
    ] {
        assert_name_rejected(&db, reserved);
    }
}

// =========================================================================
// Valid collection names — positive cases
// =========================================================================

#[test]
fn test_accept_valid_names() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    for name in &[
        "simple",
        "with_underscore",
        "with-hyphen",
        "CamelCase",
        "UPPERCASE",
        "a",
        "a1",
        "collection_v2",
        "test-123",
        &"a".repeat(128), // exactly at the limit
    ] {
        db.create_collection(name, 4, DistanceMetric::Cosine)
            .unwrap_or_else(|e| panic!("Expected name {:?} to be valid, got: {e}", name));
    }
}

// =========================================================================
// Validation on all collection types (vector, graph, metadata)
// =========================================================================

#[test]
fn test_reject_invalid_name_on_vector_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let err = db
        .create_vector_collection("../evil", 4, DistanceMetric::Cosine)
        .unwrap_err();
    assert!(matches!(err, crate::Error::InvalidCollectionName { .. }));
}

#[test]
fn test_reject_invalid_name_on_graph_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let err = db
        .create_graph_collection("../evil", crate::collection::GraphSchema::schemaless())
        .unwrap_err();
    assert!(matches!(err, crate::Error::InvalidCollectionName { .. }));
}

#[test]
fn test_reject_invalid_name_on_metadata_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let err = db.create_metadata_collection("../evil").unwrap_err();
    assert!(matches!(err, crate::Error::InvalidCollectionName { .. }));
}

// =========================================================================
// Validation on delete and stats paths
// =========================================================================

#[test]
fn test_reject_invalid_name_on_delete() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let err = db.delete_collection("../evil").unwrap_err();
    assert!(matches!(err, crate::Error::InvalidCollectionName { .. }));
}

#[test]
fn test_reject_invalid_name_on_get_stats() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let err = db.get_collection_stats("../evil").unwrap_err();
    assert!(matches!(err, crate::Error::InvalidCollectionName { .. }));
}

#[test]
fn test_reject_invalid_name_on_analyze() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let err = db.analyze_collection("../evil").unwrap_err();
    assert!(matches!(err, crate::Error::InvalidCollectionName { .. }));
}

// =========================================================================
// Disk fallback read paths reject invalid names
// =========================================================================

#[test]
fn test_get_vector_collection_rejects_traversal() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    // Should return None rather than attempting filesystem access.
    assert!(db.get_vector_collection("../evil").is_none());
}

#[test]
fn test_get_graph_collection_rejects_traversal() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert!(db.get_graph_collection("../evil").is_none());
}

#[test]
fn test_get_metadata_collection_rejects_traversal() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    assert!(db.get_metadata_collection("../evil").is_none());
}

// =========================================================================
// create_vector_collection_with_params — Wave 3 Commit 5
//
// Covers the new full-config constructor introduced to let Python (and
// any other edge crate) propagate every HnswParams field plus the
// pq_rescore_oversampling override in a single call, without having to
// decompose the params into the legacy (m, ef_construction) couple.
// =========================================================================

#[test]
fn test_create_with_params_preserves_alpha_and_max_elements() {
    use crate::index::hnsw::HnswParams;
    use crate::quantization::StorageMode;

    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // Build a fully explicit params object so every field is traceable.
    let params = HnswParams::custom(48, 400, 250_000).with_alpha(1.5);

    db.create_vector_collection_with_params(
        "full_config",
        128,
        DistanceMetric::Cosine,
        StorageMode::Full,
        params,
        Some(8),
    )
    .unwrap();

    let coll = db
        .get_vector_collection("full_config")
        .expect("collection should exist after creation");
    let cfg = coll.config();

    assert_eq!(cfg.hnsw_params.unwrap().max_connections, 48);
    assert_eq!(cfg.hnsw_params.unwrap().ef_construction, 400);
    assert_eq!(cfg.hnsw_params.unwrap().max_elements, 250_000);
    assert!((cfg.hnsw_params.unwrap().alpha - 1.5).abs() < f32::EPSILON);
    assert_eq!(cfg.pq_rescore_oversampling, Some(8));
}

#[test]
fn test_create_with_params_storage_mode_arg_wins_over_params_field() {
    use crate::index::hnsw::HnswParams;
    use crate::quantization::StorageMode;

    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // The params struct encodes StorageMode::Full by default; pass SQ8
    // as the explicit argument to verify the argument-level value wins.
    let params = HnswParams::custom(16, 200, 10_000);

    db.create_vector_collection_with_params(
        "sm_override",
        64,
        DistanceMetric::Euclidean,
        StorageMode::SQ8,
        params,
        None,
    )
    .unwrap();

    let coll = db.get_vector_collection("sm_override").unwrap();
    let cfg = coll.config();
    assert_eq!(cfg.storage_mode, StorageMode::SQ8);
    assert_eq!(cfg.hnsw_params.unwrap().storage_mode, StorageMode::SQ8);
}

#[test]
fn test_create_with_params_none_pq_rescore_is_persisted_as_none() {
    use crate::index::hnsw::HnswParams;
    use crate::quantization::StorageMode;

    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let params = HnswParams::custom(16, 200, 10_000);

    db.create_vector_collection_with_params(
        "pq_none",
        64,
        DistanceMetric::Cosine,
        StorageMode::Full,
        params,
        None,
    )
    .unwrap();

    let coll = db.get_vector_collection("pq_none").unwrap();
    // None must round-trip exactly as None, not be silently rewritten to
    // the engine default Some(4). Migrations rely on this to distinguish
    // "legacy collection, recompute the factor" from "user explicitly
    // chose no oversampling".
    assert_eq!(coll.config().pq_rescore_oversampling, None);
}

#[test]
fn test_create_with_params_duplicate_returns_collection_exists() {
    use crate::index::hnsw::HnswParams;
    use crate::quantization::StorageMode;

    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let params = HnswParams::custom(16, 200, 10_000);
    db.create_vector_collection_with_params(
        "dup_params",
        64,
        DistanceMetric::Cosine,
        StorageMode::Full,
        params,
        Some(4),
    )
    .unwrap();

    let err = db
        .create_vector_collection_with_params(
            "dup_params",
            64,
            DistanceMetric::Cosine,
            StorageMode::Full,
            params,
            Some(4),
        )
        .unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}
