use super::whole_count;
use crate::{ErrorCategory, MemoryError};

#[test]
fn a_count_is_a_whole_number_in_u32_range() {
    assert_eq!(whole_count("k", None).ok(), Some(None));
    assert_eq!(whole_count("k", Some(0.0)).ok(), Some(Some(0)));
    assert_eq!(whole_count("k", Some(60.0)).ok(), Some(Some(60)));
    assert_eq!(
        whole_count("k", Some(f64::from(u32::MAX))).ok(),
        Some(Some(u32::MAX))
    );
}

/// What a JS host's own `u32`/`usize`/`u64` argument conversion wraps instead
/// of refusing (`Infinity` → 0, `-1` → 4294967295, `2.9` → 2), so a binding
/// reads a count as `f64` and asks this validator.
#[test]
fn a_count_the_host_would_wrap_is_refused_as_invalid_input() {
    for bad in [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        -1.0,
        -0.5,
        2.9,
        f64::from(u32::MAX) + 1.0,
    ] {
        let err = whole_count("ttlSeconds", Some(bad)).expect_err("refused");
        assert!(
            matches!(
                err,
                MemoryError::InvalidCount {
                    name: "ttlSeconds",
                    ..
                }
            ),
            "{bad}: {err:?}"
        );
        assert_eq!(err.category(), ErrorCategory::InvalidInput, "{bad}");
        assert_eq!(
            err.to_string(),
            format!(
                "ttlSeconds must be a whole number in 0..={}, got {bad}",
                u32::MAX
            )
        );
    }
}
