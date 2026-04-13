//! Feature parity tests: mobile enum variants must match velesdb-core.
//!
//! These tests fail at compile time or test time when a new variant is
//! added to a core enum but not propagated to the mobile mirror.

/// All core `DistanceMetric` variants, in declaration order.
/// Update this list when adding a new variant to `velesdb_core::DistanceMetric`.
const CORE_DISTANCE_METRICS: &[velesdb_core::DistanceMetric] = &[
    velesdb_core::DistanceMetric::Cosine,
    velesdb_core::DistanceMetric::Euclidean,
    velesdb_core::DistanceMetric::DotProduct,
    velesdb_core::DistanceMetric::Hamming,
    velesdb_core::DistanceMetric::Jaccard,
];

/// All mobile `DistanceMetric` variants, in declaration order.
const MOBILE_DISTANCE_METRICS: &[velesdb_mobile::DistanceMetric] = &[
    velesdb_mobile::DistanceMetric::Cosine,
    velesdb_mobile::DistanceMetric::Euclidean,
    velesdb_mobile::DistanceMetric::DotProduct,
    velesdb_mobile::DistanceMetric::Hamming,
    velesdb_mobile::DistanceMetric::Jaccard,
];

/// All core `StorageMode` variants, in declaration order.
const CORE_STORAGE_MODES: &[velesdb_core::StorageMode] = &[
    velesdb_core::StorageMode::Full,
    velesdb_core::StorageMode::SQ8,
    velesdb_core::StorageMode::Binary,
    velesdb_core::StorageMode::ProductQuantization,
    velesdb_core::StorageMode::RaBitQ,
];

/// All mobile `StorageMode` variants, in declaration order.
const MOBILE_STORAGE_MODES: &[velesdb_mobile::StorageMode] = &[
    velesdb_mobile::StorageMode::Full,
    velesdb_mobile::StorageMode::Sq8,
    velesdb_mobile::StorageMode::Binary,
    velesdb_mobile::StorageMode::ProductQuantization,
    velesdb_mobile::StorageMode::Rabitq,
];

#[test]
fn mobile_distance_metric_variant_count_matches_core() {
    assert_eq!(
        MOBILE_DISTANCE_METRICS.len(),
        CORE_DISTANCE_METRICS.len(),
        "velesdb-mobile DistanceMetric has {} variants but velesdb-core has {}. \
         Add the missing variant to crates/velesdb-mobile/src/types.rs.",
        MOBILE_DISTANCE_METRICS.len(),
        CORE_DISTANCE_METRICS.len(),
    );
}

#[test]
fn mobile_storage_mode_variant_count_matches_core() {
    assert_eq!(
        MOBILE_STORAGE_MODES.len(),
        CORE_STORAGE_MODES.len(),
        "velesdb-mobile StorageMode has {} variants but velesdb-core has {}. \
         Add the missing variant to crates/velesdb-mobile/src/types.rs.",
        MOBILE_STORAGE_MODES.len(),
        CORE_STORAGE_MODES.len(),
    );
}

#[test]
fn mobile_distance_metric_conversions_are_exhaustive() {
    // Verify every mobile variant converts to the corresponding core variant.
    for (mobile, expected_core) in MOBILE_DISTANCE_METRICS
        .iter()
        .zip(CORE_DISTANCE_METRICS.iter())
    {
        let core: velesdb_core::DistanceMetric = (*mobile).into();
        assert_eq!(
            core, *expected_core,
            "mobile DistanceMetric conversion mismatch"
        );
    }
}

#[test]
fn mobile_storage_mode_conversions_are_exhaustive() {
    // Verify every mobile variant converts to the corresponding core variant.
    for (mobile, expected_core) in MOBILE_STORAGE_MODES.iter().zip(CORE_STORAGE_MODES.iter()) {
        let core: velesdb_core::StorageMode = (*mobile).into();
        assert_eq!(
            core, *expected_core,
            "mobile StorageMode conversion mismatch"
        );
    }
}
