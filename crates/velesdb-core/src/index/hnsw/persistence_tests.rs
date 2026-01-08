//! Tests for `persistence` module

use super::inner::HnswInner;
use super::persistence::*;
use super::sharded_mappings::ShardedMappings;
use crate::distance::DistanceMetric;
use hnsw_rs::prelude::*;
use parking_lot::RwLock;
use std::mem::ManuallyDrop;
use tempfile::TempDir;

/// Test that `save_index` creates expected files
#[test]
fn test_save_creates_files() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();

    // Create test data with at least one vector (hnsw_rs requirement)
    let hnsw = Hnsw::new(16, 100, 16, 200, DistCosine);
    hnsw.insert((&[0.1_f32, 0.2, 0.3, 0.4], 0));
    let inner = HnswInner::Cosine(hnsw);
    let inner_lock = RwLock::new(ManuallyDrop::new(inner));
    let mappings = ShardedMappings::new();

    // Act
    let result = save_index(path, &inner_lock, &mappings);

    // Assert
    assert!(
        result.is_ok(),
        "save_index failed: {err:?}",
        err = result.err()
    );
    assert!(path.join("hnsw_index.hnsw.data").exists());
    assert!(path.join("hnsw_index.hnsw.graph").exists());
    assert!(path.join("id_mappings.bin").exists());
}

/// Test that `load_index` returns error for missing files
#[test]
fn test_load_missing_mappings_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();

    // Act - try to load from empty directory
    let result = load_index(path, DistanceMetric::Cosine);

    // Assert
    match result {
        Err(err) => assert_eq!(err.kind(), std::io::ErrorKind::NotFound),
        Ok(_) => panic!("Expected error but got Ok"),
    }
}

/// Test save then load roundtrip preserves mappings
#[test]
fn test_save_load_roundtrip_preserves_mappings() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();

    // Create test data with vectors (hnsw_rs requirement)
    let hnsw = Hnsw::new(16, 100, 16, 200, DistCosine);
    hnsw.insert((&[0.1_f32, 0.2, 0.3, 0.4], 0));
    hnsw.insert((&[0.2_f32, 0.3, 0.4, 0.5], 1));
    hnsw.insert((&[0.3_f32, 0.4, 0.5, 0.6], 2));
    let inner = HnswInner::Cosine(hnsw);
    let inner_lock = RwLock::new(ManuallyDrop::new(inner));
    let mappings = ShardedMappings::new();

    // Register some IDs
    mappings.register(100);
    mappings.register(200);
    mappings.register(300);

    // Save
    save_index(path, &inner_lock, &mappings).expect("Failed to save");

    // Load
    let loaded = load_index(path, DistanceMetric::Cosine).expect("Failed to load index");

    // Assert mappings preserved
    assert_eq!(loaded.mappings.len(), 3);
    assert!(loaded.mappings.get_idx(100).is_some());
    assert!(loaded.mappings.get_idx(200).is_some());
    assert!(loaded.mappings.get_idx(300).is_some());
}

/// Test load works for all distance metrics
#[test]
#[allow(clippy::match_same_arms)]
fn test_save_load_all_metrics() {
    let metrics = [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
        DistanceMetric::Hamming,
        DistanceMetric::Jaccard,
    ];

    let test_vector: [f32; 4] = [0.1, 0.2, 0.3, 0.4];

    for metric in metrics {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        // Create index with specific metric and insert vector
        let inner = match metric {
            DistanceMetric::Cosine => {
                let hnsw = Hnsw::new(16, 100, 16, 200, DistCosine);
                hnsw.insert((&test_vector, 0));
                HnswInner::Cosine(hnsw)
            }
            DistanceMetric::Euclidean | DistanceMetric::Hamming | DistanceMetric::Jaccard => {
                let hnsw = Hnsw::new(16, 100, 16, 200, DistL2);
                hnsw.insert((&test_vector, 0));
                HnswInner::Euclidean(hnsw)
            }
            DistanceMetric::DotProduct => {
                let hnsw = Hnsw::new(16, 100, 16, 200, DistDot);
                hnsw.insert((&test_vector, 0));
                HnswInner::DotProduct(hnsw)
            }
        };
        let inner_lock = RwLock::new(ManuallyDrop::new(inner));
        let mappings = ShardedMappings::new();

        // Save and load
        save_index(path, &inner_lock, &mappings)
            .unwrap_or_else(|e| panic!("Save failed for {metric:?}: {e:?}"));
        let result = load_index(path, metric);

        assert!(result.is_ok(), "Load failed for metric {metric:?}");
    }
}
