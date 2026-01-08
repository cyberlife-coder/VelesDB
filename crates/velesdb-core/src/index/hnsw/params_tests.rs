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
    let params = HnswParams::large_dataset(768);
    assert_eq!(params.max_connections, 96); // for_dataset_size(768, 500K)
    assert_eq!(params.ef_construction, 1200);
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
    let params = HnswParams::for_dataset_size(768, 50_000);
    assert_eq!(params.max_connections, 64);
    assert_eq!(params.ef_construction, 800);
    assert_eq!(params.max_elements, 150_000);
}

#[test]
fn test_hnsw_params_for_dataset_size_large() {
    let params = HnswParams::for_dataset_size(768, 300_000);
    assert_eq!(params.max_connections, 96);
    assert_eq!(params.ef_construction, 1200);
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
    assert_eq!(SearchQuality::Accurate.ef_search(10), 256);
    assert_eq!(SearchQuality::Custom(50).ef_search(10), 50);
}

#[test]
fn test_search_quality_perfect_ef_search() {
    // Perfect mode should use very high ef_search for 100% recall
    // Base value 2048, scales with k * 50
    assert_eq!(SearchQuality::Perfect.ef_search(10), 2048); // max(2048, 10*50=500)
    assert_eq!(SearchQuality::Perfect.ef_search(50), 2500); // max(2048, 50*50=2500)
    assert_eq!(SearchQuality::Perfect.ef_search(100), 5000); // max(2048, 100*50=5000)
}

#[test]
fn test_search_quality_ef_search_high_k() {
    // Test that ef_search scales with k
    assert_eq!(SearchQuality::Fast.ef_search(100), 200); // 100 * 2
    assert_eq!(SearchQuality::Balanced.ef_search(50), 200); // 50 * 4
    assert_eq!(SearchQuality::Accurate.ef_search(40), 320); // 40 * 8
                                                            // HighRecall now uses 1024 base (was 512) for better recall
    assert_eq!(SearchQuality::HighRecall.ef_search(10), 1024); // max(1024, 10*32=320)
    assert_eq!(SearchQuality::HighRecall.ef_search(50), 1600); // max(1024, 50*32=1600)
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
