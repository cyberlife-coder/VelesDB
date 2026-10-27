use super::*;

#[test]
fn a_missing_canonical_capability_is_a_validation_error() {
    let result = require_canonical_capability(
        &BTreeMap::new(),
        "disk_headroom",
        &missing_capability(NO_HEADROOM),
    );

    let error = result.expect_err("a missing capability must be refused");
    assert!(matches!(error, MigrationError::Integrity(_)), "{error}");
    assert_eq!(
        error.to_string(),
        "integrity violation: diagnosis capability `disk_headroom` is missing"
    );
}
