//! The crate denies `unwrap`/`expect` in tests too: results are read
//! through `ok()` / `err()`.
use super::count;

#[test]
fn a_count_is_a_whole_number_in_u32_range() {
    assert_eq!(count("k", None).ok(), Some(None));
    assert_eq!(count("k", Some(0.0)).ok(), Some(Some(0)));
    assert_eq!(
        count("k", Some(f64::from(u32::MAX))).ok(),
        Some(Some(u32::MAX))
    );
    for bad in [
        f64::INFINITY,
        f64::NAN,
        -1.0,
        f64::from(u32::MAX) + 1.0,
        1.5,
    ] {
        let reason = count("k", Some(bad)).err().map(|e| e.reason);
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.contains("INVALID_INPUT")),
            "{bad}: {reason:?}"
        );
    }
}
