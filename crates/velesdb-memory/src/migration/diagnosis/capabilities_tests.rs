use super::*;

#[test]
fn a_missing_canonical_capability_is_a_validation_error() {
    let result = require_canonical_capability(
        &BTreeMap::new(),
        "disk_headroom",
        &missing_capability(NO_HEADROOM),
    );

    assert_eq!(
        result,
        Err("diagnosis capability `disk_headroom` is missing".to_owned())
    );
}
