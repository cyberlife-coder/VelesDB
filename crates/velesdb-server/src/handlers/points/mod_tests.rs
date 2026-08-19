use super::*;

#[test]
fn upsert_batch_constant_matches_expected_value() {
    assert_eq!(MAX_UPSERT_BATCH_SIZE, 100_000);
}

#[test]
fn scroll_batch_constant_matches_expected_value() {
    assert_eq!(MAX_SCROLL_BATCH_SIZE, 10_000);
}

#[test]
fn bulk_delete_batch_constant_matches_expected_value() {
    assert_eq!(MAX_BULK_DELETE_SIZE, 10_000);
}

#[test]
fn upsert_batch_limit_is_larger_than_delete_limit() {
    // Upsert is intentionally higher: ingestion workloads need larger batches.
    assert!(MAX_UPSERT_BATCH_SIZE > MAX_BULK_DELETE_SIZE);
}
