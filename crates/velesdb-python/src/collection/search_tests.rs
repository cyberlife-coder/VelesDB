use super::*;

/// Initialize the Python interpreter once (idempotent, required by PyO3
/// error constructors such as `PyValueError::new_err`).
fn init_python() {
    pyo3::Python::initialize();
}

// ---- Named modes ----

#[test]
fn test_parse_named_modes() {
    init_python();
    assert!(matches!(
        parse_search_quality("fast").unwrap(),
        velesdb_core::SearchQuality::Fast
    ));
    assert!(matches!(
        parse_search_quality("balanced").unwrap(),
        velesdb_core::SearchQuality::Balanced
    ));
    assert!(matches!(
        parse_search_quality("accurate").unwrap(),
        velesdb_core::SearchQuality::Accurate
    ));
    assert!(matches!(
        parse_search_quality("perfect").unwrap(),
        velesdb_core::SearchQuality::Perfect
    ));
    assert!(matches!(
        parse_search_quality("autotune").unwrap(),
        velesdb_core::SearchQuality::AutoTune
    ));
    assert!(matches!(
        parse_search_quality("auto").unwrap(),
        velesdb_core::SearchQuality::AutoTune
    ));
}

#[test]
fn test_parse_named_modes_case_insensitive() {
    init_python();
    assert!(matches!(
        parse_search_quality("FAST").unwrap(),
        velesdb_core::SearchQuality::Fast
    ));
    assert!(matches!(
        parse_search_quality("Balanced").unwrap(),
        velesdb_core::SearchQuality::Balanced
    ));
}

// ---- Custom mode ----

#[test]
fn test_parse_custom_valid() {
    init_python();
    let q = parse_search_quality("custom:256").unwrap();
    assert!(matches!(q, velesdb_core::SearchQuality::Custom(256)));
}

#[test]
fn test_parse_custom_case_insensitive() {
    init_python();
    let q = parse_search_quality("Custom:128").unwrap();
    assert!(matches!(q, velesdb_core::SearchQuality::Custom(128)));
}

#[test]
fn test_parse_custom_invalid_value() {
    init_python();
    let err = parse_search_quality("custom:abc").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid custom ef_search"), "got: {msg}");
}

#[test]
fn test_parse_custom_empty_value() {
    init_python();
    let err = parse_search_quality("custom:").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid custom ef_search"), "got: {msg}");
}

// ---- Adaptive mode ----

#[test]
fn test_parse_adaptive_valid() {
    init_python();
    let q = parse_search_quality("adaptive:32:512").unwrap();
    assert!(matches!(
        q,
        velesdb_core::SearchQuality::Adaptive {
            min_ef: 32,
            max_ef: 512
        }
    ));
}

#[test]
fn test_parse_adaptive_equal_bounds() {
    init_python();
    let q = parse_search_quality("adaptive:100:100").unwrap();
    assert!(matches!(
        q,
        velesdb_core::SearchQuality::Adaptive {
            min_ef: 100,
            max_ef: 100
        }
    ));
}

#[test]
fn test_parse_adaptive_case_insensitive() {
    init_python();
    let q = parse_search_quality("Adaptive:16:256").unwrap();
    assert!(matches!(
        q,
        velesdb_core::SearchQuality::Adaptive {
            min_ef: 16,
            max_ef: 256
        }
    ));
}

#[test]
fn test_parse_adaptive_inverted_range() {
    init_python();
    let err = parse_search_quality("adaptive:512:32").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("must be <= max_ef"), "got: {msg}");
}

#[test]
fn test_parse_adaptive_missing_max() {
    init_python();
    let err = parse_search_quality("adaptive:32").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid adaptive format"), "got: {msg}");
}

#[test]
fn test_parse_adaptive_non_numeric() {
    init_python();
    let err = parse_search_quality("adaptive:a:b").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid adaptive min_ef"), "got: {msg}");
}

// ---- ef range (#2275) ----

/// `custom:<ef>` and `adaptive:<min_ef>:<max_ef>` reach the range
/// `search_with_ef` enforces, and report it with the same message.
#[test]
fn test_parse_advanced_modes_reject_an_out_of_range_ef() {
    use velesdb_core::api_types::{ef_search_out_of_range, MAX_EF_SEARCH, MIN_EF_SEARCH};
    init_python();
    let too_big = MAX_EF_SEARCH + 1;
    for (mode, shown) in [
        (format!("custom:{too_big}"), too_big),
        (format!("custom:{}", MIN_EF_SEARCH - 1), MIN_EF_SEARCH - 1),
        (format!("adaptive:0:{MAX_EF_SEARCH}"), 0),
        (format!("adaptive:{MIN_EF_SEARCH}:{too_big}"), too_big),
    ] {
        let msg = parse_search_quality(&mode).unwrap_err().to_string();
        assert!(
            msg.contains(&ef_search_out_of_range(shown)),
            "{mode}: {msg}"
        );
    }
}

#[test]
fn test_parse_advanced_modes_accept_the_range_bounds() {
    use velesdb_core::api_types::{MAX_EF_SEARCH, MIN_EF_SEARCH};
    init_python();
    assert!(parse_search_quality(&format!("custom:{MIN_EF_SEARCH}")).is_ok());
    assert!(parse_search_quality(&format!("custom:{MAX_EF_SEARCH}")).is_ok());
    let adaptive = format!("adaptive:{MIN_EF_SEARCH}:{MAX_EF_SEARCH}");
    assert!(parse_search_quality(&adaptive).is_ok());
}

// ---- Unknown mode ----

#[test]
fn test_parse_unknown_mode() {
    init_python();
    let err = parse_search_quality("nonexistent").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Unknown search quality"), "got: {msg}");
    assert!(
        msg.contains("custom:<ef>"),
        "error should mention custom syntax: {msg}"
    );
    assert!(
        msg.contains("adaptive:<min_ef>:<max_ef>"),
        "error should mention adaptive syntax: {msg}"
    );
}
