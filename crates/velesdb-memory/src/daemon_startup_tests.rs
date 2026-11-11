//! `auto_link_threshold`: the env knob's parse contract — unset is the
//! documented default, a real number inside `[0, 1]` passes verbatim, and
//! everything else is a startup error carrying the offender.

use super::auto_link_threshold;

#[test]
fn unset_threshold_is_the_documented_default() {
    let threshold = auto_link_threshold(None).expect("unset parses");
    assert!(
        (threshold - velesdb_memory::service::AUTO_LINK_DEFAULT_THRESHOLD).abs() < f32::EPSILON,
        "unset must parse to the documented default, got {threshold}"
    );
}

#[test]
fn in_range_threshold_passes_verbatim() {
    let threshold = auto_link_threshold(Some("0.8")).expect("in range parses");
    assert!((threshold - 0.8).abs() < f32::EPSILON);
}

#[test]
fn malformed_threshold_is_an_error_naming_the_text() {
    for text in ["abc", "1.5", "-0.1", "NaN"] {
        let err = auto_link_threshold(Some(text)).expect_err("out of contract refused");
        assert!(
            err.to_string().contains(text),
            "error must name the offending value {text:?}: {err}"
        );
    }
}
