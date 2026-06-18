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

/// Panics if `m` is a core `DistanceMetric` variant not yet mirrored in mobile.
/// Because core `DistanceMetric` is `#[non_exhaustive]` a compile-time exhaustive
/// match is not possible from this crate; a panic arm is the practical guard.
fn assert_known_distance_metric(m: velesdb_core::DistanceMetric) {
    match m {
        velesdb_core::DistanceMetric::Cosine
        | velesdb_core::DistanceMetric::Euclidean
        | velesdb_core::DistanceMetric::DotProduct
        | velesdb_core::DistanceMetric::Hamming
        | velesdb_core::DistanceMetric::Jaccard => {}
        _ => panic!(
            "new core DistanceMetric variant not yet mirrored in velesdb-mobile: {m:?}. \
             Add it to crates/velesdb-mobile/src/types.rs."
        ),
    }
}

#[test]
fn mobile_distance_metric_variant_count_matches_core() {
    // Anchor against velesdb_core::DISTANCE_METRIC_NAMES, which core's own
    // distance_metric_names_is_exhaustive_and_canonical test keeps in sync with
    // the core enum, so this fails whenever core grows a new variant even if
    // CORE_DISTANCE_METRICS below is not yet updated.
    assert_eq!(
        MOBILE_DISTANCE_METRICS.len(),
        velesdb_core::DISTANCE_METRIC_NAMES.len(),
        "velesdb-mobile DistanceMetric has {} variants but velesdb-core has {}. \
         Add the missing variant to crates/velesdb-mobile/src/types.rs.",
        MOBILE_DISTANCE_METRICS.len(),
        velesdb_core::DISTANCE_METRIC_NAMES.len(),
    );
    // Runtime guard: panic if any known core variant is not in our exhaustive
    // match above (catches the case where the array IS updated but the match
    // arm is forgotten).
    for &m in CORE_DISTANCE_METRICS {
        assert_known_distance_metric(m);
    }
}

#[test]
fn mobile_storage_mode_variant_count_matches_core() {
    // Anchor against velesdb_core::STORAGE_MODE_NAMES, which core's own
    // storage_mode_names_is_exhaustive_and_canonical test keeps in sync with
    // the core enum. This causes a mobile-side failure whenever core grows a
    // new StorageMode variant, even if CORE_STORAGE_MODES below is not yet
    // updated. True compile-time exhaustiveness is not achievable here because
    // core StorageMode is #[non_exhaustive].
    assert_eq!(
        MOBILE_STORAGE_MODES.len(),
        velesdb_core::STORAGE_MODE_NAMES.len(),
        "velesdb-mobile StorageMode has {} variants but velesdb-core has {}. \
         Add the missing variant to crates/velesdb-mobile/src/types.rs.",
        MOBILE_STORAGE_MODES.len(),
        velesdb_core::STORAGE_MODE_NAMES.len(),
    );
}

#[test]
fn mobile_distance_metric_conversions_are_exhaustive() {
    // Guard against length mismatch: .zip() silently truncates, so an
    // array-length divergence would cause the loop below to pass on a prefix.
    assert_eq!(
        MOBILE_DISTANCE_METRICS.len(),
        CORE_DISTANCE_METRICS.len(),
        "array length drift — update CORE_DISTANCE_METRICS to match the enum"
    );
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
    // Guard against length mismatch: .zip() silently truncates, so an
    // array-length divergence would cause the loop below to pass on a prefix.
    assert_eq!(
        MOBILE_STORAGE_MODES.len(),
        CORE_STORAGE_MODES.len(),
        "array length drift — update CORE_STORAGE_MODES to match the enum"
    );
    // Verify every mobile variant converts to the corresponding core variant.
    for (mobile, expected_core) in MOBILE_STORAGE_MODES.iter().zip(CORE_STORAGE_MODES.iter()) {
        let core: velesdb_core::StorageMode = (*mobile).into();
        assert_eq!(
            core, *expected_core,
            "mobile StorageMode conversion mismatch"
        );
    }
}
