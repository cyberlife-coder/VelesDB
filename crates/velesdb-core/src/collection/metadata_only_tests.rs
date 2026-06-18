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

    assert!(!ct.is_metadata_only());
    assert!(!ct.is_graph());
    assert_eq!(ct.dimension(), Some(768));
    assert!(ct.graph_schema().is_none());
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
    let coll = db.get_metadata_collection("products").unwrap();
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

    let coll = db.get_vector_collection("embeddings").unwrap();
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

    let coll = db.get_metadata_collection("products").unwrap();

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

    let coll = db.get_metadata_collection("products").unwrap();

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

    let coll = db.get_metadata_collection("products").unwrap();

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

    let coll = db.get_metadata_collection("products").unwrap();

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

    let coll = db.get_metadata_collection("products").unwrap();

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

    let coll = db.get_metadata_collection("products").unwrap();

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

    let coll = db.get_metadata_collection("products").unwrap();

    coll.upsert_metadata(vec![Point::metadata_only(1, json!({"name": "Test"}))])
        .unwrap();

    coll.flush().unwrap();

    // HNSW persistence files should NOT exist for metadata-only collections.
    let coll_dir = dir.path().join("products");
    for f in [
        "native_meta.bin",
        "native_mappings.bin",
        "native_vectors.bin",
        "native_hnsw.gen",
    ] {
        assert!(
            !coll_dir.join(f).exists(),
            "metadata-only collection must not persist HNSW file {f}"
        );
    }
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

    let coll = db.get_metadata_collection("products").unwrap();

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

    coll.flush().unwrap();
    let coll_dir = dir.path().join("products");

    // Metadata-only never writes vector data: the vector WAL stays empty and the
    // mmap data file stays at its fixed pre-allocated header size (16 MiB INITIAL_SIZE),
    // i.e. it is NOT grown by 100 * dim * 4 bytes of dummy vectors.
    let vectors_dat = coll_dir.join("vectors.dat");
    assert!(
        vectors_dat.exists(),
        "vectors.dat is always created by MmapStorage::new"
    );
    let dat_len = std::fs::metadata(&vectors_dat).unwrap().len();
    assert!(
        dat_len <= 16 * 1024 * 1024,
        "metadata-only vectors.dat must not grow past the fixed header (no dummy vector data), got {dat_len} bytes"
    );

    // The vector WAL must contain no records for a metadata-only collection.
    let vectors_wal = coll_dir.join("vectors.wal");
    let wal_len = std::fs::metadata(&vectors_wal).map_or(0, |m| m.len());
    assert_eq!(
        wal_len, 0,
        "metadata-only must write no vector WAL records, got {wal_len} bytes"
    );
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

        let coll = db.get_metadata_collection("products").unwrap();
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

        let coll = db.get_metadata_collection("products").unwrap();
        assert!(coll.is_metadata_only());
        assert_eq!(coll.len(), 2);

        let results = coll.get(&[1, 2]);
        assert!(results[0].is_some());
        assert!(results[1].is_some());
    }
}

// =============================================================================
// AC9: execute_query on metadata collections via Database::execute_query
// =============================================================================

#[test]
fn test_execute_query_on_metadata_collection() {
    use crate::velesql::Parser;

    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_metadata_collection("meta_items").unwrap();
    let coll = db.get_metadata_collection("meta_items").unwrap();

    // Insert a few items
    let points: Vec<Point> = (1u64..=5)
        .map(|i| Point::metadata_only(i, json!({"name": format!("item_{}", i)})))
        .collect();
    coll.upsert(points).unwrap();
    drop(coll);

    // Execute VelesQL SELECT via Database::execute_query
    let query_str = "SELECT * FROM meta_items LIMIT 5";
    let parsed = Parser::parse(query_str).unwrap();
    let results = db
        .execute_query(&parsed, &std::collections::HashMap::new())
        .unwrap();

    assert_eq!(results.len(), 5, "execute_query should return all 5 items");
}
