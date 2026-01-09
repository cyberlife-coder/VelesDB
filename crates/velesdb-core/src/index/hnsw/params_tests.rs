//! Tests for `params` module

use super::params::*;
use crate::quantization::StorageMode;

#[test]
fn test_hnsw_params_default() {
    let params = HnswParams::default();
    assert_eq!(params.max_connections, 32); // auto(768) -> optimized default
    assert_eq!(params.ef_construction, 400);
}

#[test]
fn test_hnsw_params_auto_small_dimension() {
    let params = HnswParams::auto(128);
    assert_eq!(params.max_connections, 24); // 0..=256 range
    assert_eq!(params.ef_construction, 300);
}

#[test]
fn test_hnsw_params_auto_large_dimension() {
    let params = HnswParams::auto(1024);
    assert_eq!(params.max_connections, 32); // > 256 range
    assert_eq!(params.ef_construction, 400);
}

#[test]
fn test_hnsw_params_fast() {
    let params = HnswParams::fast();
    assert_eq!(params.max_connections, 16);
    assert_eq!(params.ef_construction, 150);
    assert_eq!(params.max_elements, 100_000);
}

#[test]
fn test_hnsw_params_high_recall() {
    let params = HnswParams::high_recall(768);
    assert_eq!(params.max_connections, 40); // 32 + 8
    assert_eq!(params.ef_construction, 600); // 400 + 200
}

#[test]
fn test_hnsw_params_large_dataset() {
    // Updated: large_dataset now uses M=128, ef=2000 for better recall at 500K
    let params = HnswParams::large_dataset(768);
    assert_eq!(params.max_connections, 128);
    assert_eq!(params.ef_construction, 2000);
    assert_eq!(params.max_elements, 750_000);
}

#[test]
fn test_hnsw_params_for_dataset_size_small() {
    let params = HnswParams::for_dataset_size(768, 5_000);
    assert_eq!(params.max_connections, 32);
    assert_eq!(params.ef_construction, 400);
    assert_eq!(params.max_elements, 20_000);
}

#[test]
fn test_hnsw_params_for_dataset_size_medium() {
    // Updated: 50K at 768D now uses M=128, ef=1600 for better recall
    let params = HnswParams::for_dataset_size(768, 50_000);
    assert_eq!(params.max_connections, 128);
    assert_eq!(params.ef_construction, 1600);
    assert_eq!(params.max_elements, 150_000);
}

#[test]
fn test_hnsw_params_for_dataset_size_large() {
    // Updated: 300K at 768D now uses M=128, ef=2000 for better recall
    let params = HnswParams::for_dataset_size(768, 300_000);
    assert_eq!(params.max_connections, 128);
    assert_eq!(params.ef_construction, 2000);
    assert_eq!(params.max_elements, 750_000);
}

#[test]
fn test_hnsw_params_million_scale() {
    // 1M vectors at 768D should use M=128, ef=1600 for ≥95% recall
    let params = HnswParams::million_scale(768);
    assert_eq!(params.max_connections, 128);
    assert_eq!(params.ef_construction, 1600);
    assert_eq!(params.max_elements, 1_500_000);
}

#[test]
fn test_hnsw_params_max_recall_small() {
    let params = HnswParams::max_recall(128);
    assert_eq!(params.max_connections, 32);
    assert_eq!(params.ef_construction, 500);
}

#[test]
fn test_hnsw_params_max_recall_medium() {
    let params = HnswParams::max_recall(512);
    assert_eq!(params.max_connections, 48);
    assert_eq!(params.ef_construction, 800);
}

#[test]
fn test_hnsw_params_max_recall_large() {
    let params = HnswParams::max_recall(1024);
    assert_eq!(params.max_connections, 64);
    assert_eq!(params.ef_construction, 1000);
}

#[test]
fn test_hnsw_params_fast_indexing() {
    let params = HnswParams::fast_indexing(768);
    assert_eq!(params.max_connections, 16); // 32 / 2
    assert_eq!(params.ef_construction, 200); // 400 / 2
}

#[test]
fn test_hnsw_params_custom() {
    let params = HnswParams::custom(32, 400, 50_000);
    assert_eq!(params.max_connections, 32);
    assert_eq!(params.ef_construction, 400);
    assert_eq!(params.max_elements, 50_000);
    assert_eq!(params.storage_mode, StorageMode::Full);
}

#[test]
fn test_hnsw_params_with_sq8() {
    // Arrange & Act
    let params = HnswParams::with_sq8(768);

    // Assert - SQ8 mode enabled with auto-tuned params
    assert_eq!(params.storage_mode, StorageMode::SQ8);
    assert_eq!(params.max_connections, 32); // From auto(768)
    assert_eq!(params.ef_construction, 400);
}

#[test]
fn test_hnsw_params_with_binary() {
    // Arrange & Act
    let params = HnswParams::with_binary(768);

    // Assert - Binary mode for 32x compression
    assert_eq!(params.storage_mode, StorageMode::Binary);
    assert_eq!(params.max_connections, 32);
}

#[test]
fn test_hnsw_params_storage_mode_default() {
    // Arrange & Act
    let params = HnswParams::default();

    // Assert - Default is Full precision
    assert_eq!(params.storage_mode, StorageMode::Full);
}

#[test]
fn test_search_quality_ef_search() {
    assert_eq!(SearchQuality::Fast.ef_search(10), 64);
    assert_eq!(SearchQuality::Balanced.ef_search(10), 128);
    // Updated: Accurate now uses 512 base (was 256) for 100K+ scale
    assert_eq!(SearchQuality::Accurate.ef_search(10), 512);
    assert_eq!(SearchQuality::Custom(50).ef_search(10), 50);
}

#[test]
fn test_search_quality_perfect_ef_search() {
    // Perfect mode uses 4096 base (was 2048), scales with k * 100 for 100K+ scale
    assert_eq!(SearchQuality::Perfect.ef_search(10), 4096); // max(4096, 10*100=1000)
    assert_eq!(SearchQuality::Perfect.ef_search(50), 5000); // max(4096, 50*100=5000)
    assert_eq!(SearchQuality::Perfect.ef_search(100), 10000); // max(4096, 100*100=10000)
}

#[test]
fn test_search_quality_ef_search_high_k() {
    // Test that ef_search scales with k (updated for 100K+ scale)
    assert_eq!(SearchQuality::Fast.ef_search(100), 200); // 100 * 2
    assert_eq!(SearchQuality::Balanced.ef_search(50), 200); // 50 * 4
    assert_eq!(SearchQuality::Accurate.ef_search(40), 640); // 40 * 16 (was 40 * 8)
    assert_eq!(SearchQuality::Perfect.ef_search(50), 5000); // max(4096, 50*100=5000)
}

#[test]
fn test_search_quality_perfect_serialize_deserialize() {
    // Arrange
    let quality = SearchQuality::Perfect;

    // Act
    let json = serde_json::to_string(&quality).unwrap();
    let deserialized: SearchQuality = serde_json::from_str(&json).unwrap();

    // Assert
    assert_eq!(quality, deserialized);
}

#[test]
fn test_search_quality_default() {
    let quality = SearchQuality::default();
    assert_eq!(quality, SearchQuality::Balanced);
}

#[test]
fn test_hnsw_params_turbo() {
    // TDD: Turbo mode for maximum insert throughput
    // Target: 5k+ vec/s (vs ~2k/s with auto params)
    // Trade-off: Lower recall (~85%) but acceptable for bulk loading
    let params = HnswParams::turbo();

    // Aggressive params: M=12, ef=100 for fastest graph construction
    assert_eq!(params.max_connections, 12);
    assert_eq!(params.ef_construction, 100);
    assert_eq!(params.max_elements, 100_000);
    assert_eq!(params.storage_mode, StorageMode::Full);
}

#[test]
fn test_hnsw_params_serialize_deserialize() {
    let params = HnswParams::custom(32, 400, 50_000);
    let json = serde_json::to_string(&params).unwrap();
    let deserialized: HnswParams = serde_json::from_str(&json).unwrap();
    assert_eq!(params, deserialized);
}

#[test]
fn test_search_quality_serialize_deserialize() {
    let quality = SearchQuality::Custom(100);
    let json = serde_json::to_string(&quality).unwrap();
    let deserialized: SearchQuality = serde_json::from_str(&json).unwrap();
    assert_eq!(quality, deserialized);
}

// =============================================================================
// Phase 1: Large-scale optimization tests
// =============================================================================

#[test]
fn test_hnsw_params_for_dataset_size_100k_768d() {
    // 100K vectors at 768D should use M=128, ef=1600 for ≥95% recall
    let params = HnswParams::for_dataset_size(768, 100_000);
    assert_eq!(params.max_connections, 128);
    assert_eq!(params.ef_construction, 1600);
}

#[test]
fn test_hnsw_params_for_dataset_size_500k_768d() {
    // 500K vectors at 768D should use M=128, ef=2000 for ≥95% recall
    let params = HnswParams::for_dataset_size(768, 500_000);
    assert_eq!(params.max_connections, 128);
    assert_eq!(params.ef_construction, 2000);
}
