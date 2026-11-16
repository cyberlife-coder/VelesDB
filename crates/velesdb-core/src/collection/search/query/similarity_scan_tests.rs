use super::*;

/// #901: a `similarity()` scan within the server ceiling is allowed.
#[test]
fn the_scan_guard_allows_within_the_ceiling() {
    assert!(Collection::guard_similarity_scan(Collection::SIMILARITY_SCAN_MAX).is_ok());
    assert!(Collection::guard_similarity_scan(10_000).is_ok());
}

/// #901: a `similarity()` scan over the server ceiling is REJECTED
/// (not merely warned) to block the unbounded-scan DoS vector.
#[test]
fn the_scan_guard_rejects_above_the_ceiling() {
    let err = Collection::guard_similarity_scan(Collection::SIMILARITY_SCAN_MAX + 1)
        .expect_err("scan above ceiling must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("scan limit") || msg.contains("exceeding"),
        "error should explain the scan-limit rejection, got: {msg}"
    );
}
