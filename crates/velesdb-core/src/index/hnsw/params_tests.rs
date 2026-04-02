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
    assert_eq!(SearchQuality::Fast.ef_search(10), 96);
    assert_eq!(SearchQuality::Balanced.ef_search(10), 160);
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
    assert_eq!(SearchQuality::Fast.ef_search(100), 300); // 100 * 3
    assert_eq!(SearchQuality::Balanced.ef_search(50), 250); // 50 * 5
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
// AutoTune variant tests
// =============================================================================

#[test]
fn test_search_quality_autotune_ef_search_fallback() {
    // AutoTune falls back to Balanced when ef_search() is called without
    // collection context (same as Balanced: 160 base, k*5 scaling).
    assert_eq!(SearchQuality::AutoTune.ef_search(10), 160);
    assert_eq!(SearchQuality::AutoTune.ef_search(50), 250); // 50 * 5
}

#[test]
fn test_search_quality_autotune_is_adaptive() {
    assert!(SearchQuality::AutoTune.is_adaptive());
}

#[test]
fn test_search_quality_autotune_adaptive_max_ef_is_none() {
    // AutoTune computes max_ef dynamically, so adaptive_max_ef returns None
    assert_eq!(SearchQuality::AutoTune.adaptive_max_ef(), None);
}

#[test]
fn test_search_quality_autotune_serialize_deserialize() {
    let quality = SearchQuality::AutoTune;
    let json = serde_json::to_string(&quality).unwrap();
    let deserialized: SearchQuality = serde_json::from_str(&json).unwrap();
    assert_eq!(quality, deserialized);
}

#[test]
fn test_search_quality_autotune_default_is_not_autotune() {
    // Default should remain Balanced, not AutoTune
    assert_eq!(SearchQuality::default(), SearchQuality::Balanced);
    assert_ne!(SearchQuality::default(), SearchQuality::AutoTune);
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

// =============================================================================
// ef_search_for_scale: auto-adaptive to dataset size
// =============================================================================

#[test]
fn test_ef_search_for_scale_no_scaling_at_10k() {
    // Datasets <= 10K use base ef_search unchanged
    assert_eq!(SearchQuality::Fast.ef_search_for_scale(10, 5_000), 96);
    assert_eq!(SearchQuality::Balanced.ef_search_for_scale(10, 10_000), 160);
}

#[test]
fn test_ef_search_for_scale_100k() {
    // At 100K: sqrt(10) ≈ 3.16 → capped at 2.0
    let ef = SearchQuality::Balanced.ef_search_for_scale(10, 100_000);
    // 160 * 2.0 = 320, capped at 160*2=320
    assert!(ef > 160, "ef should scale up at 100K, got {ef}");
    assert!(ef <= 320, "ef should be capped at 2x base, got {ef}");
}

#[test]
fn test_ef_search_for_scale_1m() {
    // At 1M: sqrt(100) = 10 → capped at 2x
    let ef = SearchQuality::Fast.ef_search_for_scale(10, 1_000_000);
    assert_eq!(ef, 96 * 2, "ef should be capped at 2x for 1M dataset");
}

#[test]
fn test_ef_search_for_scale_custom_passes_through() {
    // Custom ef should also scale
    let ef = SearchQuality::Custom(200).ef_search_for_scale(10, 100_000);
    assert!(ef > 200, "custom ef should scale at 100K, got {ef}");
    assert!(ef <= 400, "custom ef should be capped at 2x, got {ef}");
}

#[test]
fn test_hnsw_params_for_dataset_size_500k_768d() {
    // 500K vectors at 768D should use M=128, ef=2000 for ≥95% recall
    let params = HnswParams::for_dataset_size(768, 500_000);
    assert_eq!(params.max_connections, 128);
    assert_eq!(params.ef_construction, 2000);
}

// =============================================================================
// Alpha (VAMANA diversification) parameter tests
// =============================================================================

#[test]
fn test_hnsw_params_alpha_default() {
    // All constructors should default alpha to 1.2 (VAMANA recommendation)
    let params = HnswParams::auto(768);
    assert!(
        (params.alpha - 1.2).abs() < f32::EPSILON,
        "auto() alpha should be 1.2, got {}",
        params.alpha
    );
}

#[test]
fn test_hnsw_params_alpha_default_all_constructors() {
    // Every constructor must produce alpha = 1.2 by default
    let constructors: Vec<(&str, HnswParams)> = vec![
        ("default", HnswParams::default()),
        ("auto(128)", HnswParams::auto(128)),
        ("auto(768)", HnswParams::auto(768)),
        ("fast", HnswParams::fast()),
        ("turbo", HnswParams::turbo()),
        ("high_recall", HnswParams::high_recall(768)),
        ("max_recall(128)", HnswParams::max_recall(128)),
        ("max_recall(512)", HnswParams::max_recall(512)),
        ("max_recall(1024)", HnswParams::max_recall(1024)),
        ("fast_indexing", HnswParams::fast_indexing(768)),
        ("custom", HnswParams::custom(32, 400, 50_000)),
        ("with_sq8", HnswParams::with_sq8(768)),
        ("with_binary", HnswParams::with_binary(768)),
        (
            "for_dataset_size(5K)",
            HnswParams::for_dataset_size(768, 5_000),
        ),
        (
            "for_dataset_size(50K)",
            HnswParams::for_dataset_size(768, 50_000),
        ),
        ("large_dataset", HnswParams::large_dataset(768)),
        ("million_scale", HnswParams::million_scale(768)),
    ];
    for (name, params) in &constructors {
        assert!(
            (params.alpha - 1.2).abs() < f32::EPSILON,
            "{name}() alpha should be 1.2, got {}",
            params.alpha
        );
    }
}

#[test]
fn test_hnsw_params_with_alpha_custom() {
    let params = HnswParams::auto(768).with_alpha(1.0);
    assert!(
        (params.alpha - 1.0).abs() < f32::EPSILON,
        "with_alpha(1.0) should set alpha to 1.0, got {}",
        params.alpha
    );
}

#[test]
fn test_hnsw_params_with_alpha_preserves_other_fields() {
    let base = HnswParams::auto(768);
    let modified = base.with_alpha(1.5);
    assert_eq!(modified.max_connections, base.max_connections);
    assert_eq!(modified.ef_construction, base.ef_construction);
    assert_eq!(modified.max_elements, base.max_elements);
    assert_eq!(modified.storage_mode, base.storage_mode);
    assert!(
        (modified.alpha - 1.5).abs() < f32::EPSILON,
        "alpha should be 1.5, got {}",
        modified.alpha
    );
}

#[test]
fn test_hnsw_params_alpha_serde_roundtrip() {
    let params = HnswParams::auto(768).with_alpha(1.5);
    let json = serde_json::to_string(&params).expect("test: serialize");
    let deserialized: HnswParams = serde_json::from_str(&json).expect("test: deserialize");
    assert!(
        (deserialized.alpha - 1.5).abs() < f32::EPSILON,
        "roundtrip alpha should be 1.5, got {}",
        deserialized.alpha
    );
}

#[test]
fn test_hnsw_params_implements_eq() {
    // HnswParams has a manual Eq impl (alpha is f32, but always finite).
    // Verify Eq reflexivity and that equal values work as HashMap keys.
    let a = HnswParams::auto(768);
    let b = HnswParams::auto(768);
    // PartialEq + Eq: reflexive, symmetric
    assert_eq!(a, b);
    assert_eq!(b, a);
    assert_eq!(a, a); // reflexive

    // Verify Eq is usable in contexts that require it.
    // Storing in a slice and using contains() exercises PartialEq;
    // the manual Eq impl on HnswParams enables use as HashMap key
    // or in Eq-bounded generic contexts.
    let params_list = [a];
    assert!(params_list.contains(&b));
}

#[test]
fn test_hnsw_params_alpha_backward_compat_missing_field() {
    // Persisted configs from before alpha was added won't have the field.
    // Deserialization must default to 1.2.
    let json = r#"{"max_connections":32,"ef_construction":400,"max_elements":100000}"#;
    let params: HnswParams = serde_json::from_str(json).expect("test: backward compat deserialize");
    assert!(
        (params.alpha - 1.2).abs() < f32::EPSILON,
        "missing alpha field should default to 1.2, got {}",
        params.alpha
    );
}
