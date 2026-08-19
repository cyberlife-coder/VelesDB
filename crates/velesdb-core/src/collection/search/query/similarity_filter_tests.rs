use super::*;

/// #901: a `NOT similarity()` scan within the server ceiling is allowed.
#[test]
fn test_not_similarity_guard_allows_within_ceiling() {
    assert!(Collection::guard_not_similarity_scan(Collection::NOT_SIMILARITY_MAX_SCAN).is_ok());
    assert!(Collection::guard_not_similarity_scan(10_000).is_ok());
}

/// #901: a `NOT similarity()` scan over the server ceiling is REJECTED
/// (not merely warned) to block the unbounded-scan DoS vector.
#[test]
fn test_not_similarity_guard_rejects_above_ceiling() {
    let err = Collection::guard_not_similarity_scan(Collection::NOT_SIMILARITY_MAX_SCAN + 1)
        .expect_err("scan above ceiling must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("scan limit") || msg.contains("exceeding"),
        "error should explain the scan-limit rejection, got: {msg}"
    );
}
