//! The metric and storage names a `CREATE COLLECTION` accepts.

use super::*;

#[test]
fn a_known_metric_or_storage_mode_resolves_and_none_is_full() {
    assert_eq!(
        resolve_metric("cosine").ok(),
        Some(crate::DistanceMetric::Cosine)
    );
    assert_eq!(
        resolve_storage_mode(None).ok(),
        Some(crate::StorageMode::default())
    );
    assert_eq!(
        resolve_storage_mode(Some("sq8")).ok(),
        Some(crate::StorageMode::SQ8)
    );
}

#[test]
fn an_unknown_metric_or_storage_mode_is_a_query_error_naming_it() {
    let metric = resolve_metric("bogus").expect_err("test: unknown metric");
    assert_eq!(metric.code(), "VELES-010");
    assert!(
        metric.to_string().contains("Unknown metric 'bogus'"),
        "{metric}"
    );
    let mode = resolve_storage_mode(Some("bogus")).expect_err("test: unknown mode");
    assert_eq!(
        mode.to_string(),
        "[VELES-010] Query error: Unknown storage mode 'bogus'. Use: full, sq8, binary, pq, rabitq"
    );
}
