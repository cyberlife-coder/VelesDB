use super::*;
use crate::distance::DistanceMetric;
use crate::quantization::StorageMode;

fn make_config(oversampling: Option<u32>) -> CollectionConfig {
    CollectionConfig {
        name: "test".to_string(),
        dimension: 128,
        metric: DistanceMetric::Euclidean,
        point_count: 0,
        schema_version: CURRENT_SCHEMA_VERSION,
        storage_mode: StorageMode::ProductQuantization,
        metadata_only: false,
        graph_schema: None,
        embedding_dimension: None,
        pq_rescore_oversampling: oversampling,
        hnsw_params: None,
        #[cfg(feature = "persistence")]
        deferred_indexing: None,
        async_index_builder: None,
        auto_reindex_config: None,
        #[cfg(feature = "persistence")]
        streaming_config: None,
        indexed_fields: BTreeSet::new(),
    }
}

#[test]
fn rescore_default_oversampling_is_4() {
    let config = make_config(default_pq_rescore_oversampling());
    assert_eq!(config.pq_rescore_oversampling, Some(4));
}

#[test]
fn rescore_candidates_k_formula_default() {
    // Default factor = 4, k = 10
    // candidates_k = max(10 * 4, 10 + 32) = max(40, 42) = 42
    let factor = 4_usize;
    let k = 10_usize;
    let candidates_k = k.saturating_mul(factor).max(k + 32);
    assert_eq!(candidates_k, 42);
}

#[test]
fn rescore_candidates_k_formula_custom_factor_6() {
    // factor = 6, k = 10
    // candidates_k = max(10 * 6, 10 + 32) = max(60, 42) = 60
    let factor = 6_usize;
    let k = 10_usize;
    let candidates_k = k.saturating_mul(factor).max(k + 32);
    assert_eq!(candidates_k, 60);
}

#[test]
fn rescore_none_disables_oversampling() {
    let config = make_config(None);
    let oversampling = config.pq_rescore_oversampling.unwrap_or(0);
    assert_eq!(oversampling, 0, "None should map to 0 (disabled)");
}

#[test]
fn rescore_active_by_default_for_pq() {
    let config = make_config(default_pq_rescore_oversampling());
    assert!(
        config.pq_rescore_oversampling.is_some(),
        "Rescore must be active by default for PQ"
    );
    assert!(
        config.pq_rescore_oversampling.unwrap() > 0,
        "Default oversampling must be > 0"
    );
}

#[test]
fn rescore_serde_default_backward_compat() {
    // Simulate deserializing a config without pq_rescore_oversampling field.
    // The serde default should kick in and set Some(4).
    let json = r#"{
        "name": "old_collection",
        "dimension": 128,
        "metric": "Euclidean",
        "point_count": 100,
        "storage_mode": "productquantization"
    }"#;
    let config: CollectionConfig = serde_json::from_str(json).unwrap();
    assert_eq!(
        config.pq_rescore_oversampling,
        Some(4),
        "Missing field must deserialize to Some(4) for backward compat"
    );
}

#[test]
fn rescore_minimum_floor_preserved() {
    // Even with small k, the floor k + 32 must dominate
    let factor = 4_usize;
    let k = 5_usize;
    let candidates_k = k.saturating_mul(factor).max(k + 32);
    // max(20, 37) = 37
    assert_eq!(candidates_k, 37);
}
