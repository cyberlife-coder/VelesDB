//! Feature parity tests: migrate enum variants must match velesdb-core.
//!
//! These tests fail at compile time or test time when a new variant is
//! added to a core enum but not propagated to the migrate mirror.

use velesdb_migrate::config::{DistanceMetric, StorageMode};

/// All migrate `DistanceMetric` variants, in declaration order.
const MIGRATE_DISTANCE_METRICS: &[DistanceMetric] = &[
    DistanceMetric::Cosine,
    DistanceMetric::Euclidean,
    DistanceMetric::Dot,
    DistanceMetric::Hamming,
    DistanceMetric::Jaccard,
];

/// All core `DistanceMetric` variants, in declaration order.
const CORE_DISTANCE_METRICS: &[velesdb_core::DistanceMetric] = &[
    velesdb_core::DistanceMetric::Cosine,
    velesdb_core::DistanceMetric::Euclidean,
    velesdb_core::DistanceMetric::DotProduct,
    velesdb_core::DistanceMetric::Hamming,
    velesdb_core::DistanceMetric::Jaccard,
];

/// All migrate `StorageMode` variants, in declaration order.
const MIGRATE_STORAGE_MODES: &[StorageMode] = &[
    StorageMode::Full,
    StorageMode::SQ8,
    StorageMode::Binary,
    StorageMode::Pq,
    StorageMode::RaBitQ,
];

/// All core `StorageMode` variants, in declaration order.
const CORE_STORAGE_MODES: &[velesdb_core::StorageMode] = &[
    velesdb_core::StorageMode::Full,
    velesdb_core::StorageMode::SQ8,
    velesdb_core::StorageMode::Binary,
    velesdb_core::StorageMode::ProductQuantization,
    velesdb_core::StorageMode::RaBitQ,
];

#[test]
fn migrate_distance_metric_variant_count_matches_core() {
    assert_eq!(
        MIGRATE_DISTANCE_METRICS.len(),
        CORE_DISTANCE_METRICS.len(),
        "velesdb-migrate DistanceMetric has {} variants but velesdb-core has {}. \
         Add the missing variant to crates/velesdb-migrate/src/config.rs.",
        MIGRATE_DISTANCE_METRICS.len(),
        CORE_DISTANCE_METRICS.len(),
    );
}

#[test]
fn migrate_storage_mode_variant_count_matches_core() {
    assert_eq!(
        MIGRATE_STORAGE_MODES.len(),
        CORE_STORAGE_MODES.len(),
        "velesdb-migrate StorageMode has {} variants but velesdb-core has {}. \
         Add the missing variant to crates/velesdb-migrate/src/config.rs.",
        MIGRATE_STORAGE_MODES.len(),
        CORE_STORAGE_MODES.len(),
    );
}
