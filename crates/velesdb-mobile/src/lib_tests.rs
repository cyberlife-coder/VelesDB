//! Tests for VelesDB Mobile UniFFI bindings.

use super::*;
use tempfile::TempDir;

// =========================================================================
// DistanceMetric Tests
// =========================================================================

#[test]
fn test_distance_metric_cosine_conversion() {
    let metric = DistanceMetric::Cosine;
    let core: CoreDistanceMetric = metric.into();
    assert_eq!(core, CoreDistanceMetric::Cosine);
}

#[test]
fn test_distance_metric_euclidean_conversion() {
    let metric = DistanceMetric::Euclidean;
    let core: CoreDistanceMetric = metric.into();
    assert_eq!(core, CoreDistanceMetric::Euclidean);
}

#[test]
fn test_distance_metric_dot_product_conversion() {
    let metric = DistanceMetric::DotProduct;
    let core: CoreDistanceMetric = metric.into();
    assert_eq!(core, CoreDistanceMetric::DotProduct);
}

#[test]
fn test_distance_metric_hamming_conversion() {
    let metric = DistanceMetric::Hamming;
    let core: CoreDistanceMetric = metric.into();
    assert_eq!(core, CoreDistanceMetric::Hamming);
}

#[test]
fn test_distance_metric_jaccard_conversion() {
    let metric = DistanceMetric::Jaccard;
    let core: CoreDistanceMetric = metric.into();
    assert_eq!(core, CoreDistanceMetric::Jaccard);
}

// =========================================================================
// StorageMode Tests
// =========================================================================

#[test]
fn test_storage_mode_full_conversion() {
    let mode = StorageMode::Full;
    let core: velesdb_core::StorageMode = mode.into();
    assert_eq!(core, velesdb_core::StorageMode::Full);
}

#[test]
fn test_storage_mode_sq8_conversion() {
    let mode = StorageMode::Sq8;
    let core: velesdb_core::StorageMode = mode.into();
    assert_eq!(core, velesdb_core::StorageMode::SQ8);
}

#[test]
fn test_storage_mode_binary_conversion() {
    let mode = StorageMode::Binary;
    let core: velesdb_core::StorageMode = mode.into();
    assert_eq!(core, velesdb_core::StorageMode::Binary);
}

// =========================================================================
// VelesDatabase Tests
// =========================================================================

#[test]
fn test_database_open_and_create_collection() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("test".to_string(), 128, DistanceMetric::Cosine)
        .unwrap();

    let collections = db.list_collections();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0], "test");
}

#[test]
fn test_database_create_collection_with_storage() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection_with_storage(
        "sq8_collection".to_string(),
        384,
        DistanceMetric::Euclidean,
        StorageMode::Sq8,
    )
    .unwrap();

    let col = db.get_collection("sq8_collection".to_string()).unwrap();
    assert!(col.is_some());
    assert_eq!(col.unwrap().dimension(), 384);
}

#[test]
fn test_database_delete_collection() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("to_delete".to_string(), 64, DistanceMetric::DotProduct)
        .unwrap();

    assert_eq!(db.list_collections().len(), 1);

    db.delete_collection("to_delete".to_string()).unwrap();
    assert_eq!(db.list_collections().len(), 0);
}

// =========================================================================
// VelesCollection Tests
// =========================================================================

#[test]
fn test_collection_upsert_and_search() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("vectors".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db.get_collection("vectors".to_string()).unwrap().unwrap();

    let point = VelesPoint {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
    };
    col.upsert(point).unwrap();

    assert_eq!(col.count(), 1);

    let results = col.search(vec![1.0, 0.0, 0.0, 0.0], 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 1);
}

#[test]
fn test_collection_upsert_batch() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("batch".to_string(), 4, DistanceMetric::Euclidean)
        .unwrap();

    let col = db.get_collection("batch".to_string()).unwrap().unwrap();

    let points = vec![
        VelesPoint {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: None,
        },
        VelesPoint {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: None,
        },
        VelesPoint {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: None,
        },
    ];

    col.upsert_batch(points).unwrap();
    assert_eq!(col.count(), 3);
}

#[test]
fn test_collection_delete() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("delete_test".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db
        .get_collection("delete_test".to_string())
        .unwrap()
        .unwrap();

    col.upsert(VelesPoint {
        id: 42,
        vector: vec![1.0, 1.0, 1.0, 1.0],
        payload: None,
    })
    .unwrap();

    assert_eq!(col.count(), 1);

    col.delete(42).unwrap();
    assert_eq!(col.count(), 0);
}

#[test]
fn test_collection_with_json_payload() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("with_payload".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db
        .get_collection("with_payload".to_string())
        .unwrap()
        .unwrap();

    let point = VelesPoint {
        id: 1,
        vector: vec![0.5, 0.5, 0.5, 0.5],
        payload: Some(r#"{"title": "Hello", "category": "test"}"#.to_string()),
    };

    col.upsert(point).unwrap();
    assert_eq!(col.count(), 1);
}

// =========================================================================
// All 5 Metrics Integration Tests
// =========================================================================

#[test]
fn test_all_five_metrics() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let db = VelesDatabase::open(path).unwrap();

    let metrics = [
        ("cosine", DistanceMetric::Cosine),
        ("euclidean", DistanceMetric::Euclidean),
        ("dot", DistanceMetric::DotProduct),
        ("hamming", DistanceMetric::Hamming),
        ("jaccard", DistanceMetric::Jaccard),
    ];

    for (name, metric) in metrics {
        db.create_collection(name.to_string(), 4, metric).unwrap();
        let col = db.get_collection(name.to_string()).unwrap().unwrap();
        col.upsert(VelesPoint {
            id: 1,
            vector: vec![1.0, 0.0, 1.0, 0.0],
            payload: None,
        })
        .unwrap();
        assert_eq!(col.count(), 1, "Collection {name} should have 1 point");
    }

    assert_eq!(db.list_collections().len(), 5);
}

// =========================================================================
// All 3 Storage Modes Integration Tests
// =========================================================================

#[test]
fn test_all_three_storage_modes() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let db = VelesDatabase::open(path).unwrap();

    let modes = [
        ("full", StorageMode::Full),
        ("sq8", StorageMode::Sq8),
        ("binary", StorageMode::Binary),
    ];

    for (name, mode) in modes {
        db.create_collection_with_storage(name.to_string(), 128, DistanceMetric::Cosine, mode)
            .unwrap();

        let col = db.get_collection(name.to_string()).unwrap().unwrap();
        col.upsert(VelesPoint {
            id: 1,
            vector: vec![0.1; 128],
            payload: None,
        })
        .unwrap();
        assert_eq!(col.count(), 1, "Collection {name} should have 1 point");
    }

    assert_eq!(db.list_collections().len(), 3);
}

// =========================================================================
// FusionStrategy Tests
// =========================================================================

#[test]
fn test_fusion_strategy_average_conversion() {
    let strategy = FusionStrategy::Average;
    let core: CoreFusionStrategy = strategy.into();
    assert!(matches!(core, CoreFusionStrategy::Average));
}

#[test]
fn test_fusion_strategy_maximum_conversion() {
    let strategy = FusionStrategy::Maximum;
    let core: CoreFusionStrategy = strategy.into();
    assert!(matches!(core, CoreFusionStrategy::Maximum));
}

#[test]
fn test_fusion_strategy_rrf_conversion() {
    let strategy = FusionStrategy::Rrf { k: 30 };
    let core: CoreFusionStrategy = strategy.into();
    assert!(matches!(core, CoreFusionStrategy::RRF { k: 30 }));
}

#[test]
fn test_fusion_strategy_weighted_conversion() {
    let strategy = FusionStrategy::Weighted {
        avg_weight: 0.5,
        max_weight: 0.3,
        hit_weight: 0.2,
    };
    let core: CoreFusionStrategy = strategy.into();
    assert!(matches!(
        core,
        CoreFusionStrategy::Weighted {
            avg_weight: _,
            max_weight: _,
            hit_weight: _
        }
    ));
}

#[test]
fn test_fusion_strategy_default() {
    let strategy = FusionStrategy::default();
    assert!(matches!(strategy, FusionStrategy::Rrf { k: 60 }));
}

// =========================================================================
// Multi-Query Search Tests
// =========================================================================

#[test]
fn test_multi_query_search_basic() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("mqs_test".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db.get_collection("mqs_test".to_string()).unwrap().unwrap();

    col.upsert_batch(vec![
        VelesPoint {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: None,
        },
        VelesPoint {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: None,
        },
        VelesPoint {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: None,
        },
    ])
    .unwrap();

    let results = col
        .multi_query_search(
            vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0]],
            5,
            FusionStrategy::Rrf { k: 60 },
        )
        .unwrap();

    assert!(!results.is_empty());
    let ids: Vec<u64> = results.iter().map(|r| r.id).collect();
    assert!(ids.contains(&1) || ids.contains(&2));
}

#[test]
fn test_multi_query_search_all_strategies() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("mqs_strategies".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db
        .get_collection("mqs_strategies".to_string())
        .unwrap()
        .unwrap();

    col.upsert_batch(vec![
        VelesPoint {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: None,
        },
        VelesPoint {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: None,
        },
    ])
    .unwrap();

    let vectors = vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.5, 0.5, 0.0, 0.0]];

    let strategies = [
        FusionStrategy::Average,
        FusionStrategy::Maximum,
        FusionStrategy::Rrf { k: 60 },
        FusionStrategy::Weighted {
            avg_weight: 0.5,
            max_weight: 0.3,
            hit_weight: 0.2,
        },
    ];

    for strategy in strategies {
        let results = col
            .multi_query_search(vectors.clone(), 5, strategy)
            .unwrap();
        assert!(!results.is_empty(), "Strategy should return results");
    }
}

#[test]
fn test_multi_query_search_empty_vectors_error() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("mqs_empty".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db.get_collection("mqs_empty".to_string()).unwrap().unwrap();

    let result = col.multi_query_search(vec![], 5, FusionStrategy::Rrf { k: 60 });

    assert!(result.is_err());
}

// =========================================================================
// Metadata-Only Collection Tests
// =========================================================================

#[test]
fn test_create_metadata_collection() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_metadata_collection("meta_test".to_string())
        .unwrap();

    let col = db.get_collection("meta_test".to_string()).unwrap().unwrap();

    assert!(col.is_metadata_only());
    assert_eq!(col.dimension(), 0);
}

#[test]
fn test_regular_collection_not_metadata_only() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("vector_test".to_string(), 128, DistanceMetric::Cosine)
        .unwrap();

    let col = db
        .get_collection("vector_test".to_string())
        .unwrap()
        .unwrap();

    assert!(!col.is_metadata_only());
}

// =========================================================================
// Get by ID Tests
// =========================================================================

#[test]
fn test_get_by_id_existing() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("get_test".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db.get_collection("get_test".to_string()).unwrap().unwrap();

    col.upsert(VelesPoint {
        id: 42,
        vector: vec![1.0, 2.0, 3.0, 4.0],
        payload: Some(r#"{"name": "test"}"#.to_string()),
    })
    .unwrap();

    let result = col.get_by_id(42);
    assert!(result.is_some());
    let point = result.unwrap();
    assert_eq!(point.id, 42);
    assert_eq!(point.vector, vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn test_get_by_id_missing() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("get_missing".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db
        .get_collection("get_missing".to_string())
        .unwrap()
        .unwrap();

    let result = col.get_by_id(999);
    assert!(result.is_none());
}

#[test]
fn test_get_multiple_ids() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("get_multi".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db.get_collection("get_multi".to_string()).unwrap().unwrap();

    col.upsert_batch(vec![
        VelesPoint {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: None,
        },
        VelesPoint {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: None,
        },
        VelesPoint {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: None,
        },
    ])
    .unwrap();

    let results = col.get(vec![1, 2, 999]); // 999 doesn't exist
    assert_eq!(results.len(), 2); // Only 2 found
}
