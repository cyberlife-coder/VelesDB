//! TDD tests for metadata-only collections (EPIC-CORE-002).
//!
//! These tests define the expected behavior for collections
//! that store metadata without vectors.

use crate::collection::CollectionType;
use crate::error::Error;
use crate::point::Point;
use crate::Database;
use serde_json::json;
use tempfile::tempdir;

// =============================================================================
// AC1: CollectionType enum
// =============================================================================

#[test]
fn test_collection_type_metadata_only_exists() {
    // CollectionType::MetadataOnly should exist
    let ct = CollectionType::MetadataOnly;
    assert!(ct.is_metadata_only());
}

#[test]
fn test_collection_type_vector_exists() {
    use crate::distance::DistanceMetric;
    use crate::quantization::StorageMode;

    // CollectionType::Vector should contain dimension, metric, storage_mode
    let ct = CollectionType::Vector {
        dimension: 768,
        metric: DistanceMetric::Cosine,
        storage_mode: StorageMode::Full,
    };

    match ct {
        CollectionType::Vector {
            dimension,
            metric,
            storage_mode,
        } => {
            assert_eq!(dimension, 768);
            assert_eq!(metric, DistanceMetric::Cosine);
            assert_eq!(storage_mode, StorageMode::Full);
        }
        CollectionType::MetadataOnly => panic!("Expected Vector variant"),
    }
}

// =============================================================================
// AC2: Database::create_collection_typed API
// =============================================================================

#[test]
fn test_create_metadata_only_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // Create a metadata-only collection
    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    // Verify it exists
    let collections = db.list_collections();
    assert!(collections.contains(&"products".to_string()));

    // Verify we can get it
    let coll = db.get_collection("products").unwrap();
    assert!(coll.is_metadata_only());
}

#[test]
fn test_create_vector_collection_typed() {
    use crate::distance::DistanceMetric;
    use crate::quantization::StorageMode;

    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // Create a vector collection using the typed API
    db.create_collection_typed(
        "embeddings",
        &CollectionType::Vector {
            dimension: 768,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
    )
    .unwrap();

    let coll = db.get_collection("embeddings").unwrap();
    assert!(!coll.is_metadata_only());
    assert_eq!(coll.config().dimension, 768);
}

// =============================================================================
// AC3: Upsert without vector on metadata-only collections
// =============================================================================

#[test]
fn test_metadata_only_upsert_without_vector() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    let coll = db.get_collection("products").unwrap();

    // Upsert points without vectors (metadata-only point)
    let result = coll.upsert_metadata(vec![
        Point::metadata_only(
            1,
            json!({
                "code_produit": "PROD001",
                "pays": "France",
                "nom_produit": "Séjour Paris",
                "prix": 1500.0
            }),
        ),
        Point::metadata_only(
            2,
            json!({
                "code_produit": "PROD002",
                "pays": "Espagne",
                "nom_produit": "Circuit Andalousie",
                "prix": 2000.0
            }),
        ),
    ]);

    assert!(result.is_ok());
    assert_eq!(coll.len(), 2);
}

#[test]
fn test_metadata_only_rejects_vector() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    let coll = db.get_collection("products").unwrap();

    // Attempt to upsert a point WITH a vector should fail
    let result = coll.upsert(vec![Point::new(
        1,
        vec![0.1; 768],
        Some(json!({"title": "Test"})),
    )]);

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::VectorNotAllowed(collection_name) => {
            assert_eq!(collection_name, "products");
        }
        e => panic!("Expected VectorNotAllowed error, got: {e:?}"),
    }
}

// =============================================================================
// AC4: Supported operations on metadata-only collections
// =============================================================================

#[test]
fn test_metadata_only_get_by_id() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    let coll = db.get_collection("products").unwrap();

    coll.upsert_metadata(vec![Point::metadata_only(
        42,
        json!({"name": "Test Product", "price": 99.99}),
    )])
    .unwrap();

    // Get by ID should work
    let results = coll.get(&[42]);
    assert_eq!(results.len(), 1);
    assert!(results[0].is_some());

    let point = results[0].as_ref().unwrap();
    assert_eq!(point.id, 42);
    assert!(point.vector.is_empty()); // No vector
    assert!(point.payload.is_some());
}

#[test]
fn test_metadata_only_delete() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    let coll = db.get_collection("products").unwrap();

    coll.upsert_metadata(vec![
        Point::metadata_only(1, json!({"name": "Product 1"})),
        Point::metadata_only(2, json!({"name": "Product 2"})),
    ])
    .unwrap();

    assert_eq!(coll.len(), 2);

    // Delete should work
    coll.delete(&[1]).unwrap();
    assert_eq!(coll.len(), 1);
}

#[test]
fn test_metadata_only_count() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    let coll = db.get_collection("products").unwrap();

    assert_eq!(coll.len(), 0);
    assert!(coll.is_empty());

    coll.upsert_metadata(vec![
        Point::metadata_only(1, json!({"name": "A"})),
        Point::metadata_only(2, json!({"name": "B"})),
        Point::metadata_only(3, json!({"name": "C"})),
    ])
    .unwrap();

    assert_eq!(coll.len(), 3);
    assert!(!coll.is_empty());
}

#[test]
fn test_metadata_only_search_returns_error() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    let coll = db.get_collection("products").unwrap();

    coll.upsert_metadata(vec![Point::metadata_only(1, json!({"name": "Test"}))])
        .unwrap();

    // search() MUST return an explicit error
    let query_vector = vec![0.1; 768];
    let result = coll.search(&query_vector, 10);

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::SearchNotSupported(collection_name) => {
            assert_eq!(collection_name, "products");
        }
        e => panic!("Expected SearchNotSupported error, got: {e:?}"),
    }
}

// =============================================================================
// AC5: No HNSW index created for metadata-only
// =============================================================================

#[test]
fn test_metadata_only_no_hnsw_file() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    let coll = db.get_collection("products").unwrap();

    coll.upsert_metadata(vec![Point::metadata_only(1, json!({"name": "Test"}))])
        .unwrap();

    coll.flush().unwrap();

    // HNSW index file should NOT exist
    let hnsw_path = dir.path().join("products").join("hnsw.bin");
    assert!(
        !hnsw_path.exists(),
        "HNSW index should not be created for metadata-only collections"
    );
}

// =============================================================================
// AC6: Memory efficiency (no dummy vectors)
// =============================================================================

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_metadata_only_memory_efficient() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_collection_typed("products", &CollectionType::MetadataOnly)
        .unwrap();

    let coll = db.get_collection("products").unwrap();

    // Insert 100 metadata-only points
    let points: Vec<_> = (0..100_u64)
        .map(|i| {
            let price = (i as f64) * 10.0; // Safe: i < 100, no precision loss
            Point::metadata_only(
                i,
                json!({
                    "id": i,
                    "name": format!("Product {i}"),
                    "price": price
                }),
            )
        })
        .collect();

    coll.upsert_metadata(points).unwrap();

    // No vector storage file should be created or should be minimal
    let vectors_path = dir.path().join("products").join("vectors.bin");
    if vectors_path.exists() {
        let size = std::fs::metadata(&vectors_path).unwrap().len();
        // Should be essentially empty (just header, not 768*4*100 = 307KB)
        assert!(
            size < 1000,
            "Vector storage should be minimal for metadata-only, got {size} bytes"
        );
    }
}

// =============================================================================
// Persistence: reopen metadata-only collection
// =============================================================================

#[test]
fn test_metadata_only_persistence() {
    let dir = tempdir().unwrap();

    // Create and populate
    {
        let db = Database::open(dir.path()).unwrap();
        db.create_collection_typed("products", &CollectionType::MetadataOnly)
            .unwrap();

        let coll = db.get_collection("products").unwrap();
        coll.upsert_metadata(vec![
            Point::metadata_only(1, json!({"name": "Product 1"})),
            Point::metadata_only(2, json!({"name": "Product 2"})),
        ])
        .unwrap();
        coll.flush().unwrap();
    }

    // Reopen and verify
    {
        let db = Database::open(dir.path()).unwrap();
        // Load existing collections from disk
        db.load_collections().unwrap();

        let coll = db.get_collection("products").unwrap();
        assert!(coll.is_metadata_only());
        assert_eq!(coll.len(), 2);

        let results = coll.get(&[1, 2]);
        assert!(results[0].is_some());
        assert!(results[1].is_some());
    }
}
