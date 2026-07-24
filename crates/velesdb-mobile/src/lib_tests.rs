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
// SearchQuality Tests
// =========================================================================

#[test]
fn test_search_quality_fast_conversion() {
    let q = SearchQuality::Fast;
    let core: CoreSearchQuality = q.into();
    assert!(matches!(core, CoreSearchQuality::Fast));
}

#[test]
fn test_search_quality_balanced_conversion() {
    let q = SearchQuality::Balanced;
    let core: CoreSearchQuality = q.into();
    assert!(matches!(core, CoreSearchQuality::Balanced));
}

#[test]
fn test_search_quality_accurate_conversion() {
    let q = SearchQuality::Accurate;
    let core: CoreSearchQuality = q.into();
    assert!(matches!(core, CoreSearchQuality::Accurate));
}

#[test]
fn test_search_quality_perfect_conversion() {
    let q = SearchQuality::Perfect;
    let core: CoreSearchQuality = q.into();
    assert!(matches!(core, CoreSearchQuality::Perfect));
}

#[test]
fn test_search_quality_custom_conversion() {
    let q = SearchQuality::Custom { ef: 256 };
    let core: CoreSearchQuality = q.into();
    assert!(matches!(core, CoreSearchQuality::Custom(256)));
}

#[test]
fn test_search_quality_adaptive_conversion() {
    let q = SearchQuality::Adaptive {
        min_ef: 32,
        max_ef: 512,
    };
    let core: CoreSearchQuality = q.into();
    assert!(matches!(
        core,
        CoreSearchQuality::Adaptive {
            min_ef: 32,
            max_ef: 512
        }
    ));
}

#[test]
fn test_search_quality_autotune_conversion() {
    let q = SearchQuality::AutoTune;
    let core: CoreSearchQuality = q.into();
    assert!(matches!(core, CoreSearchQuality::AutoTune));
}

#[test]
fn test_search_quality_default() {
    let q = SearchQuality::default();
    assert!(matches!(q, SearchQuality::Balanced));
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

#[test]
fn test_storage_mode_product_quantization_conversion() {
    let mode = StorageMode::ProductQuantization;
    let core: velesdb_core::StorageMode = mode.into();
    assert_eq!(core, velesdb_core::StorageMode::ProductQuantization);
}

#[test]
fn test_storage_mode_rabitq_conversion() {
    let mode = StorageMode::Rabitq;
    let core: velesdb_core::StorageMode = mode.into();
    assert_eq!(core, velesdb_core::StorageMode::RaBitQ);
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
fn test_collection_search_with_quality() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("quality_test".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db
        .get_collection("quality_test".to_string())
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

    // Test all named quality modes produce valid results
    let modes = [
        SearchQuality::Fast,
        SearchQuality::Balanced,
        SearchQuality::Accurate,
        SearchQuality::Perfect,
        SearchQuality::AutoTune,
        SearchQuality::Custom { ef: 200 },
        SearchQuality::Adaptive {
            min_ef: 32,
            max_ef: 512,
        },
    ];

    for quality in modes {
        let results = col
            .search_with_quality(vec![1.0, 0.0, 0.0, 0.0], 2, quality)
            .unwrap();
        assert!(!results.is_empty(), "quality mode should return results");
        assert_eq!(results[0].id, 1, "closest vector should be id=1");
    }
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
fn test_collection_compact_storage() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("compact_test".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let col = db
        .get_collection("compact_test".to_string())
        .unwrap()
        .unwrap();

    for id in 0..5u64 {
        col.upsert(VelesPoint {
            id,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: None,
        })
        .unwrap();
    }
    col.delete(0).unwrap();
    col.delete(1).unwrap();

    // Compaction must succeed and return the reclaimed byte count; live
    // points are unaffected.
    let freed = col.compact_storage().unwrap();
    assert!(
        freed > 0,
        "compact_storage should reclaim bytes after 2 deletes"
    );
    assert_eq!(col.count(), 3);
}

#[test]
fn test_collection_guardrails_update_and_read() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("gr".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let col = db.get_collection("gr".to_string()).unwrap().unwrap();

    db.update_guardrails(MobileQueryLimits {
        max_depth: 7,
        max_cardinality: 1000,
        memory_limit_bytes: 1_048_576,
        timeout_ms: 5000,
        rate_limit_qps: 50,
        circuit_failure_threshold: 3,
        circuit_recovery_seconds: 30,
    });

    let read = col.guard_rails();
    assert_eq!(read.max_depth, 7);
    assert_eq!(read.timeout_ms, 5000);
    assert_eq!(read.rate_limit_qps, 50);
    assert_eq!(read.memory_limit_bytes, 1_048_576);
}

#[test]
fn test_advanced_config_record_conversions() {
    let deferred: velesdb_core::collection::streaming::DeferredIndexerConfig =
        MobileDeferredIndexerConfig {
            enabled: true,
            merge_threshold: 512,
            max_buffer_age_ms: 3000,
        }
        .into();
    assert!(deferred.enabled);
    assert_eq!(deferred.merge_threshold, 512);
    assert_eq!(deferred.max_buffer_age_ms, 3000);

    let async_builder: velesdb_core::collection::streaming::AsyncIndexBuilderConfig =
        MobileAsyncIndexBuilderConfig {
            merge_threshold: 20_000,
            segment_count: Some(4),
        }
        .into();
    assert_eq!(async_builder.merge_threshold, 20_000);
    assert_eq!(async_builder.segment_count, Some(4));
}

#[test]
fn test_collection_apply_advanced_config() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("adv".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let col = db.get_collection("adv".to_string()).unwrap().unwrap();

    // All three fields set: must persist and be readable back.
    col.apply_advanced_config(MobileAdvancedConfig {
        pq_rescore_oversampling: Some(8),
        deferred_indexing: Some(MobileDeferredIndexerConfig {
            enabled: true,
            merge_threshold: 512,
            max_buffer_age_ms: 3000,
        }),
        async_index_builder: Some(MobileAsyncIndexBuilderConfig {
            merge_threshold: 20_000,
            segment_count: None,
        }),
    })
    .unwrap();

    let cfg = col.inner.config();
    assert_eq!(cfg.pq_rescore_oversampling, Some(8));
    let aib = cfg
        .async_index_builder
        .as_ref()
        .expect("async_index_builder set");
    assert_eq!(aib.merge_threshold, 20_000);
    assert_eq!(aib.segment_count, None);
    let d = cfg
        .deferred_indexing
        .as_ref()
        .expect("deferred_indexing set");
    assert!(d.enabled);
    assert_eq!(d.merge_threshold, 512);
    assert_eq!(d.max_buffer_age_ms, 3000);

    // None fields leave configuration unchanged (no-op contract).
    col.apply_advanced_config(MobileAdvancedConfig {
        pq_rescore_oversampling: None,
        deferred_indexing: None,
        async_index_builder: None,
    })
    .unwrap();

    let cfg2 = col.inner.config();
    assert_eq!(cfg2.pq_rescore_oversampling, Some(8));
    assert!(cfg2
        .async_index_builder
        .as_ref()
        .is_some_and(|a| a.merge_threshold == 20_000));
    assert!(cfg2
        .deferred_indexing
        .as_ref()
        .is_some_and(|d| d.enabled && d.merge_threshold == 512));
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

    let fetched = col.get_by_id(1).expect("point 1 should exist");
    let payload_str = fetched.payload.expect("payload should persist");
    let got: serde_json::Value =
        serde_json::from_str(&payload_str).expect("stored payload must be valid JSON");
    let expected: serde_json::Value = serde_json::json!({"title": "Hello", "category": "test"});
    assert_eq!(
        got, expected,
        "payload must round-trip through parse_point + get_by_id"
    );
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
// Storage Modes Integration Tests
// =========================================================================
//
// Covers the three storage modes that accept upserts without a prior
// training step (Full, Sq8, Binary). `ProductQuantization` and `Rabitq`
// require a trained quantizer before inserts succeed and are covered by
// pure conversion tests above — their end-to-end creation+upsert flow
// is exercised by core and server crates where training helpers live.

#[test]
fn test_untrained_storage_modes_full_upsert_flow() {
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
    let CoreFusionStrategy::Weighted {
        avg_weight,
        max_weight,
        hit_weight,
    } = core
    else {
        panic!("expected Weighted variant, got {core:?}");
    };
    assert!((avg_weight - 0.5).abs() < f32::EPSILON);
    assert!((max_weight - 0.3).abs() < f32::EPSILON);
    assert!((hit_weight - 0.2).abs() < f32::EPSILON);
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
    assert!(
        ids.contains(&1) && ids.contains(&2),
        "both query-aligned exact matches (id=1 for query[0], id=2 for query[1]) must survive RRF fusion within top-5"
    );
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
            .multi_query_search(vectors.clone(), 5, strategy.clone())
            .unwrap();
        // Only 2 points exist, limit 5 -> both returned.
        assert_eq!(results.len(), 2, "{strategy:?} should return both points");
        // Query 0 == id=1's vector exactly, and id=1 ties/beats id=2 on query 1,
        // so id=1 must rank first under every fusion strategy.
        assert_eq!(
            results[0].id, 1,
            "{strategy:?} must rank the exact-match id=1 first"
        );
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

#[test]
fn test_multi_query_search_ids_matches_core() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("mqs_ids".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db.get_collection("mqs_ids".to_string()).unwrap().unwrap();

    col.upsert_batch(vec![
        VelesPoint {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some("{\"k\":1}".to_string()),
        },
        VelesPoint {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some("{\"k\":2}".to_string()),
        },
        VelesPoint {
            id: 3,
            vector: vec![0.0, 0.0, 1.0, 0.0],
            payload: Some("{\"k\":3}".to_string()),
        },
    ])
    .unwrap();

    let vectors = vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0]];

    let id_results = col
        .multi_query_search_ids(vectors.clone(), 5, FusionStrategy::Rrf { k: 60 })
        .unwrap();
    let full_results = col
        .multi_query_search(vectors, 5, FusionStrategy::Rrf { k: 60 })
        .unwrap();

    // id-only twin returns the same IDs/scores as the payload-carrying path.
    // Compare as sorted sets: equal scores may tie-break in either order.
    let mut id_pairs: Vec<(u64, u32)> = id_results
        .iter()
        .map(|r| (r.id, r.score.to_bits()))
        .collect();
    let mut full_pairs: Vec<(u64, u32)> = full_results
        .iter()
        .map(|r| (r.id, r.score.to_bits()))
        .collect();
    id_pairs.sort_unstable();
    full_pairs.sort_unstable();
    assert_eq!(id_pairs, full_pairs);

    // Payloads are stripped from the id-only variant.
    assert!(id_results.iter().all(|r| r.payload.is_none()));
}

#[test]
fn test_multi_query_search_ids_empty_vectors_error() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("mqs_ids_empty".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db
        .get_collection("mqs_ids_empty".to_string())
        .unwrap()
        .unwrap();

    let result = col.multi_query_search_ids(vec![], 5, FusionStrategy::Rrf { k: 60 });

    assert!(result.is_err());
}

// =========================================================================
// Collection Diagnostics Tests
// =========================================================================

#[test]
fn test_collection_diagnostics_with_data() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("diag_test".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db.get_collection("diag_test".to_string()).unwrap().unwrap();

    // Empty collection: no vectors, not search-ready, index empty.
    let empty = col.diagnostics();
    assert!(!empty.has_vectors);
    assert!(!empty.search_ready);
    assert!(empty.dimension_configured);
    assert_eq!(empty.point_count, 0);
    assert_eq!(empty.index_health, "empty");

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

    // Populated collection: vectors present, search-ready, index healthy.
    let diag = col.diagnostics();
    assert!(diag.has_vectors);
    assert!(diag.search_ready);
    assert!(diag.dimension_configured);
    assert_eq!(diag.point_count, 2);
    assert_eq!(diag.index_health, "healthy");
    assert!(diag.index_health_detail.is_none());
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

    // Metadata collections are not vector collections, so get_collection
    // should return an error directing the caller to the typed API.
    let result = db.get_collection("meta_test".to_string());
    assert!(
        result.is_err(),
        "get_collection should reject non-vector collections"
    );

    // Verify the collection was created (visible in list)
    assert!(db.list_collections().contains(&"meta_test".to_string()));
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
// Graph Collection Tests
// =========================================================================

#[test]
fn test_create_graph_collection() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_graph_collection("kg".to_string()).unwrap();

    assert!(db.list_collections().contains(&"kg".to_string()));
}

#[test]
fn test_create_graph_collection_with_embeddings() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_graph_collection_with_embeddings("kg_emb".to_string(), 128, DistanceMetric::Cosine)
        .unwrap();

    assert!(db.list_collections().contains(&"kg_emb".to_string()));
}

#[test]
fn test_get_collection_rejects_graph_collection() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_graph_collection("kg_reject".to_string()).unwrap();

    // get_collection should reject a graph collection with a clear error
    let result = db.get_collection("kg_reject".to_string());
    assert!(
        result.is_err(),
        "get_collection should reject graph collections"
    );
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

// =========================================================================
// Core parity tests: stats/index/flush/all_ids
// =========================================================================

#[test]
fn test_collection_flush_and_all_ids() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("flush_ids".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db.get_collection("flush_ids".to_string()).unwrap().unwrap();
    col.upsert_batch(vec![
        VelesPoint {
            id: 9,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(r#"{"v":1}"#.to_string()),
        },
        VelesPoint {
            id: 4,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(r#"{"v":2}"#.to_string()),
        },
    ])
    .unwrap();

    col.flush().unwrap();

    let mut ids = col.all_ids();
    ids.sort_unstable();
    assert_eq!(ids, vec![4, 9]);
}

#[test]
fn test_collection_secondary_index_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("secondary_index".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db
        .get_collection("secondary_index".to_string())
        .unwrap()
        .unwrap();

    assert!(!col.has_secondary_index("category".to_string()));
    col.create_index("category".to_string()).unwrap();
    assert!(col.has_secondary_index("category".to_string()));
}

#[test]
fn test_collection_property_and_range_index_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("graph_indexes".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db
        .get_collection("graph_indexes".to_string())
        .unwrap()
        .unwrap();

    col.create_property_index("Doc".to_string(), "title".to_string())
        .unwrap();
    col.create_range_index("Doc".to_string(), "year".to_string())
        .unwrap();

    assert!(col.has_property_index("Doc".to_string(), "title".to_string()));
    assert!(col.has_range_index("Doc".to_string(), "year".to_string()));

    let indexes = col.list_indexes();
    assert!(indexes
        .iter()
        .any(|idx| idx.label == "Doc" && idx.property == "title" && idx.index_type == "hash"));
    assert!(indexes
        .iter()
        .any(|idx| idx.label == "Doc" && idx.property == "year" && idx.index_type == "range"));

    let usage = col.indexes_memory_usage();
    assert!(usage > 0);

    assert!(col
        .drop_index("Doc".to_string(), "title".to_string())
        .unwrap());
    assert!(!col.has_property_index("Doc".to_string(), "title".to_string()));
}

#[test]
fn test_collection_analyze_and_get_stats() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("stats".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let col = db.get_collection("stats".to_string()).unwrap().unwrap();
    col.upsert_batch(vec![
        VelesPoint {
            id: 1,
            vector: vec![1.0, 0.0, 0.0, 0.0],
            payload: Some(r#"{"category":"a"}"#.to_string()),
        },
        VelesPoint {
            id: 2,
            vector: vec![0.0, 1.0, 0.0, 0.0],
            payload: Some(r#"{"category":"b"}"#.to_string()),
        },
    ])
    .unwrap();

    let analyzed = col.analyze().unwrap();
    assert!(analyzed.total_points >= 2);

    let snapshot = col.get_stats();
    assert!(snapshot.total_points >= 2);
    assert!(snapshot.field_stats_count >= 1 || snapshot.column_stats_count >= 1);
}

// =========================================================================
// Streaming ingestion tests
// =========================================================================

#[test]
fn test_streaming_enable_and_insert_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("stream_rt".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let col = db.get_collection("stream_rt".to_string()).unwrap().unwrap();

    // enable with engine defaults
    col.enable_streaming(None).unwrap();
    // re-enable with an explicit config (replaces the ingester)
    col.enable_streaming(Some(MobileStreamingConfig {
        buffer_size: 256,
        batch_size: 32,
        flush_interval_ms: 10,
    }))
    .unwrap();

    let queued = col
        .stream_insert(vec![
            VelesPoint {
                id: 1,
                vector: vec![1.0, 0.0, 0.0, 0.0],
                payload: Some(r#"{"k":"v"}"#.to_string()),
            },
            VelesPoint {
                id: 2,
                vector: vec![0.0, 1.0, 0.0, 0.0],
                payload: None,
            },
        ])
        .unwrap();
    assert_eq!(queued, 2);
}

// =========================================================================
// Observer read-gate tests (audit F-5.4, #1392)
// =========================================================================

/// Test observer that denies every read with a fixed reason.
struct DenyAllObserver;

impl MobileObserver for DenyAllObserver {
    fn on_query_request(&self, _context: MobileQueryContext) -> MobileAccessDecision {
        MobileAccessDecision::Deny {
            reason: "test policy".to_string(),
        }
    }
}

/// Test observer that allows every read and records the contexts it saw, so a
/// test can assert the read actually passed through the gate.
#[derive(Default)]
struct RecordingObserver {
    seen: std::sync::Mutex<Vec<(String, MobileQueryOperationKind)>>,
}

impl MobileObserver for RecordingObserver {
    fn on_query_request(&self, context: MobileQueryContext) -> MobileAccessDecision {
        self.seen
            .lock()
            .unwrap()
            .push((context.collection, context.operation));
        MobileAccessDecision::Allow
    }
}

/// Opens a database with `observer`, creates a 4-dim `vectors` collection and
/// seeds one point. Writes are not gated, so this succeeds even under a denying
/// observer (only reads pass through `on_query_request`).
fn seed_gated_db(
    path: String,
    observer: std::sync::Arc<dyn MobileObserver>,
) -> std::sync::Arc<VelesDatabase> {
    let db = VelesDatabase::open_with_observer(path, observer).unwrap();
    db.create_collection("vectors".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let col = db.get_collection("vectors".to_string()).unwrap().unwrap();
    col.upsert(VelesPoint {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: Some(r#"{"cat":"a"}"#.to_string()),
    })
    .unwrap();
    db
}

#[test]
fn test_observer_allow_read_returns_results_and_sees_context() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let obs = std::sync::Arc::new(RecordingObserver::default());
    let db = seed_gated_db(path, obs.clone());
    let col = db.get_collection("vectors".to_string()).unwrap().unwrap();

    let results = col.search(vec![1.0, 0.0, 0.0, 0.0], 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 1);

    // The read passed through the gate: the observer saw a VectorSearch on
    // the `vectors` collection.
    let seen = obs.seen.lock().unwrap();
    assert!(
        seen.iter()
            .any(|(c, op)| c == "vectors" && *op == MobileQueryOperationKind::VectorSearch),
        "observer should have seen a VectorSearch on 'vectors', saw: {seen:?}"
    );
}

#[test]
fn test_observer_deny_blocks_vector_search() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = seed_gated_db(path, std::sync::Arc::new(DenyAllObserver));
    let col = db.get_collection("vectors".to_string()).unwrap().unwrap();

    let err = col.search(vec![1.0, 0.0, 0.0, 0.0], 1).unwrap_err();
    let VelesError::Database { message, .. } = err else {
        panic!("expected VelesError::Database, got {err:?}");
    };
    assert!(
        message.contains("denied by observer"),
        "denial should surface the observer reason, got: {message}"
    );
}

#[test]
fn test_observer_deny_blocks_text_hybrid_and_velesql() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = seed_gated_db(path, std::sync::Arc::new(DenyAllObserver));
    let col = db.get_collection("vectors".to_string()).unwrap().unwrap();

    assert!(
        col.text_search("hello".to_string(), 5).is_err(),
        "text_search must be gated"
    );
    assert!(
        col.hybrid_search(vec![1.0, 0.0, 0.0, 0.0], "hello".to_string(), 5, 0.5)
            .is_err(),
        "hybrid_search must be gated"
    );
    // VelesQL collection query is routed through the gated database facade, so
    // it is denied too.
    assert!(
        col.query("SELECT * FROM vectors LIMIT 10".to_string(), None)
            .is_err(),
        "VelesQL query must be gated"
    );
}

#[test]
fn test_observer_deny_blocks_sparse_and_multi_query() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = seed_gated_db(path, std::sync::Arc::new(DenyAllObserver));
    let col = db.get_collection("vectors".to_string()).unwrap().unwrap();

    // The gate is consulted before the leaf, so denial short-circuits even the
    // sparse path (whether or not a sparse index is configured).
    let sparse = VelesSparseVector {
        indices: vec![0],
        values: vec![1.0],
    };
    assert!(
        col.sparse_search(sparse, 5, None).is_err(),
        "sparse_search must be gated"
    );
    assert!(
        col.multi_query_search(
            vec![vec![1.0, 0.0, 0.0, 0.0]],
            5,
            FusionStrategy::Rrf { k: 60 }
        )
        .is_err(),
        "multi_query_search must be gated"
    );
}

#[test]
fn test_open_without_observer_reads_unchanged() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    // Baseline: the plain open() path registers no observer, so the gate is a
    // no-op and reads behave exactly as before.
    let db = VelesDatabase::open(path).unwrap();
    db.create_collection("vectors".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let col = db.get_collection("vectors".to_string()).unwrap().unwrap();
    col.upsert(VelesPoint {
        id: 7,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
    })
    .unwrap();

    let results = col.search(vec![1.0, 0.0, 0.0, 0.0], 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 7);
}

// =========================================================================
// Config-aware constructor tests (issue #1549, mobile surface)
// =========================================================================
//
// Before these constructors existed, `VelesDatabase` could only be opened
// through `open` / `open_with_observer`, which always run the core engine on
// default `VelesConfig` values — a TOML engine config (`[search]`/`[hnsw]`/
// `[storage]`/`[limits]`/`[quantization]`/`[wal_batch]`) had no way in.
// These tests prove the config is *enforced* by the engine (not merely
// parsed), that invalid input fails fast with a typed error, and that the
// pre-existing constructors keep their default behaviour.

/// The core proof required by issue #1549: a `limits.max_collections` cap in
/// the TOML string must be enforced by the opened database. First collection
/// succeeds (within the cap), second is refused by the engine.
#[test]
fn test_open_with_config_toml_limit_is_enforced() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db =
        VelesDatabase::open_with_config_toml(path, "[limits]\nmax_collections = 1\n".to_string())
            .unwrap();

    db.create_collection("first".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let err = db
        .create_collection("second".to_string(), 4, DistanceMetric::Cosine)
        .expect_err("second collection must be refused by the configured limit");
    assert!(
        err.to_string().contains("max_collections"),
        "expected the limit error to mention max_collections, got: {err}"
    );
}

/// Same proof through the file-path variant (`load_from_path_engine_only`).
#[test]
fn test_open_with_config_path_limit_is_enforced() {
    let tmp = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("velesdb.toml");
    std::fs::write(&config_path, "[limits]\nmax_collections = 1\n").unwrap();

    let db = VelesDatabase::open_with_config(
        tmp.path().to_str().unwrap().to_string(),
        config_path.to_str().unwrap().to_string(),
    )
    .unwrap();

    db.create_collection("first".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let err = db
        .create_collection("second".to_string(), 4, DistanceMetric::Cosine)
        .expect_err("second collection must be refused by the configured limit");
    assert!(
        err.to_string().contains("max_collections"),
        "expected the limit error to mention max_collections, got: {err}"
    );
}

/// Fail-fast: a syntactically invalid TOML string must abort the open with a
/// typed config error (core taxonomy code VELES-009), never fall back to
/// defaults silently.
#[test]
fn test_open_with_config_toml_invalid_toml_fails_fast() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let Err(err) = VelesDatabase::open_with_config_toml(path, "not [[[ toml".to_string()) else {
        panic!("invalid TOML must fail fast");
    };
    let VelesError::Database { code, message, .. } = &err else {
        panic!("expected VelesError::Database, got {err:?}");
    };
    assert_eq!(
        code, "VELES-009",
        "config errors carry the core config code"
    );
    assert!(
        message.contains("parse") || message.contains("Parse"),
        "message should surface the underlying ConfigError, got: {message}"
    );
}

/// Fail-fast: a TOML value rejected by `VelesConfig::validate` (here
/// `limits.max_collections = 0`, outside `[1, cap]`) must abort the open.
#[test]
fn test_open_with_config_toml_invalid_value_fails_fast() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let Err(err) =
        VelesDatabase::open_with_config_toml(path, "[limits]\nmax_collections = 0\n".to_string())
    else {
        panic!("out-of-range value must fail validation");
    };
    let VelesError::Database { code, message, .. } = &err else {
        panic!("expected VelesError::Database, got {err:?}");
    };
    assert_eq!(code, "VELES-009");
    assert!(
        message.contains("max_collections"),
        "message should name the offending key, got: {message}"
    );
}

/// Fail-fast: a config path that does not exist must abort the open with a
/// typed config error — never open on silent defaults.
#[test]
fn test_open_with_config_path_missing_file_fails_fast() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    let missing = tmp.path().join("does-not-exist.toml");

    let Err(err) = VelesDatabase::open_with_config(path, missing.to_str().unwrap().to_string())
    else {
        panic!("missing config file must fail fast");
    };
    let VelesError::Database { code, .. } = &err else {
        panic!("expected VelesError::Database, got {err:?}");
    };
    assert_eq!(code, "VELES-009");
}

/// Observer + config: the TOML limit must be enforced AND the observer read
/// gate must be active on the same handle.
#[test]
fn test_open_with_observer_and_config_toml_wires_both() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open_with_observer_and_config_toml(
        path,
        std::sync::Arc::new(DenyAllObserver),
        "[limits]\nmax_collections = 1\n".to_string(),
    )
    .unwrap();

    // Config wired: cap of 1 enforced.
    db.create_collection("first".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    let err = db
        .create_collection("second".to_string(), 4, DistanceMetric::Cosine)
        .expect_err("second collection must be refused by the configured limit");
    assert!(err.to_string().contains("max_collections"));

    // Observer wired: reads are denied by the gate (writes are not gated).
    let col = db.get_collection("first".to_string()).unwrap().unwrap();
    col.upsert(VelesPoint {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: None,
    })
    .unwrap();
    let err = col
        .search(vec![1.0, 0.0, 0.0, 0.0], 1)
        .expect_err("deny-all observer must block the read");
    assert!(
        err.to_string().contains("test policy"),
        "expected the observer's deny reason, got: {err}"
    );
}

/// Observer + config through the file-path variant.
#[test]
fn test_open_with_observer_and_config_path_wires_both() {
    let tmp = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("velesdb.toml");
    std::fs::write(&config_path, "[limits]\nmax_collections = 1\n").unwrap();

    let db = VelesDatabase::open_with_observer_and_config(
        tmp.path().to_str().unwrap().to_string(),
        std::sync::Arc::new(DenyAllObserver),
        config_path.to_str().unwrap().to_string(),
    )
    .unwrap();

    db.create_collection("first".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    assert!(
        db.create_collection("second".to_string(), 4, DistanceMetric::Cosine)
            .is_err(),
        "cap of 1 must be enforced"
    );
}

/// Regression guard: the pre-existing constructors keep running on core
/// defaults (`limits.max_collections = 1000`) — adding the config-aware
/// constructors must not change `open` / `open_with_observer` behaviour.
#[test]
fn test_open_without_config_keeps_default_limits() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let db = VelesDatabase::open(path).unwrap();
    // Well above a cap of 1: proves no stray config is applied.
    db.create_collection("first".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    db.create_collection("second".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();

    let tmp2 = TempDir::new().unwrap();
    let db2 = VelesDatabase::open_with_observer(
        tmp2.path().to_str().unwrap().to_string(),
        std::sync::Arc::new(RecordingObserver::default()),
    )
    .unwrap();
    db2.create_collection("first".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
    db2.create_collection("second".to_string(), 4, DistanceMetric::Cosine)
        .unwrap();
}
