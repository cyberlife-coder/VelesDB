use super::*;

#[test]
fn test_parse_metric_valid() {
    assert!(matches!(
        parse_metric_inner("cosine"),
        Ok(DistanceMetric::Cosine)
    ));
    assert!(matches!(
        parse_metric_inner("EUCLIDEAN"),
        Ok(DistanceMetric::Euclidean)
    ));
    assert!(matches!(
        parse_metric_inner("l2"),
        Ok(DistanceMetric::Euclidean)
    ));
    assert!(matches!(
        parse_metric_inner("dot"),
        Ok(DistanceMetric::DotProduct)
    ));
    assert!(matches!(
        parse_metric_inner("dotproduct"),
        Ok(DistanceMetric::DotProduct)
    ));
    assert!(matches!(
        parse_metric_inner("hamming"),
        Ok(DistanceMetric::Hamming)
    ));
    assert!(matches!(
        parse_metric_inner("jaccard"),
        Ok(DistanceMetric::Jaccard)
    ));
}

#[test]
fn test_parse_metric_invalid() {
    assert!(parse_metric_inner("unknown").is_err());
}

#[test]
fn test_metric_parsing_is_delegated_to_core_source_of_truth() {
    use std::str::FromStr;

    for alias in ["cosine", "l2", "dot", "inner", "hamming", "jaccard"] {
        let parsed = parse_metric_inner(alias).unwrap();
        let from_core = DistanceMetric::from_str(alias).unwrap();
        assert_eq!(parsed, from_core);
    }
}

#[test]
fn test_parse_storage_mode_valid() {
    assert!(matches!(
        parse_storage_mode_inner("full"),
        Ok(StorageMode::Full)
    ));
    assert!(matches!(
        parse_storage_mode_inner("SQ8"),
        Ok(StorageMode::SQ8)
    ));
    assert!(matches!(
        parse_storage_mode_inner("binary"),
        Ok(StorageMode::Binary)
    ));
    assert!(matches!(
        parse_storage_mode_inner("pq"),
        Ok(StorageMode::ProductQuantization)
    ));
}

#[test]
fn test_parse_storage_mode_invalid() {
    assert!(parse_storage_mode_inner("unknown").is_err());
}

// BDD (audit-2026q2 H4): pins the round-trip contract between
// velesdb_core::StorageMode and the local WASM StorageMode.
//
// GIVEN every known variant of velesdb_core::StorageMode,
// WHEN core_to_wasm_storage_mode is applied,
// THEN the result is the matching WASM variant (Full → Full, SQ8 → SQ8, ...).
//
// If a new variant is added to core::StorageMode without being added here,
// the production fallback to Full would silently mask the gap. The
// debug_assert!(false) in the catch-all arm now panics in debug builds —
// this test ensures the explicit mapping for every known variant remains
// correct over time. To add a variant: extend the match in
// `core_to_wasm_storage_mode` AND add a row in `CASES` below.
#[test]
fn core_to_wasm_storage_mode_round_trips_every_known_variant() {
    use velesdb_core::StorageMode as Core;

    const CASES: &[(Core, StorageMode)] = &[
        (Core::Full, StorageMode::Full),
        (Core::SQ8, StorageMode::SQ8),
        (Core::Binary, StorageMode::Binary),
        (Core::ProductQuantization, StorageMode::ProductQuantization),
        (Core::RaBitQ, StorageMode::RaBitQ),
    ];

    for (core, expected) in CASES {
        let mapped = core_to_wasm_storage_mode(*core);
        assert_eq!(
            mapped,
            *expected,
            "core variant `{}` must map to expected WASM variant",
            core.canonical_name()
        );
    }
}

// =========================================================================
// SearchQuality parsing tests
// =========================================================================

#[test]
fn test_parse_search_quality_named_modes() {
    assert!(parse_search_quality_inner("fast").is_ok());
    assert!(parse_search_quality_inner("balanced").is_ok());
    assert!(parse_search_quality_inner("accurate").is_ok());
    assert!(parse_search_quality_inner("perfect").is_ok());
    assert!(parse_search_quality_inner("autotune").is_ok());
    assert!(parse_search_quality_inner("auto").is_ok());
}

#[test]
fn test_parse_search_quality_case_insensitive() {
    assert!(parse_search_quality_inner("FAST").is_ok());
    assert!(parse_search_quality_inner("Balanced").is_ok());
    assert!(parse_search_quality_inner("AUTOTUNE").is_ok());
}

#[test]
fn test_parse_search_quality_custom() {
    assert!(parse_search_quality_inner("custom:256").is_ok());
}

#[test]
fn test_parse_search_quality_custom_case_insensitive() {
    assert!(parse_search_quality_inner("Custom:128").is_ok());
}

#[test]
fn test_parse_search_quality_custom_invalid() {
    let err = parse_search_quality_inner("custom:abc");
    assert!(err.is_err());
    assert!(err
        .unwrap_err()
        .contains("Unknown search mode 'custom:abc'"));
}

#[test]
fn test_parse_search_quality_adaptive() {
    assert!(parse_search_quality_inner("adaptive:32:512").is_ok());
}

#[test]
fn test_parse_search_quality_adaptive_equal_bounds() {
    assert!(parse_search_quality_inner("adaptive:100:100").is_ok());
}

#[test]
fn test_parse_search_quality_adaptive_inverted_range() {
    let err = parse_search_quality_inner("adaptive:512:32");
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("(min_ef <= max_ef)"));
}

#[test]
fn test_parse_search_quality_adaptive_missing_max() {
    let err = parse_search_quality_inner("adaptive:32");
    assert!(err.is_err());
    assert!(err
        .unwrap_err()
        .contains("Unknown search mode 'adaptive:32'"));
}

#[test]
fn test_parse_search_quality_unknown() {
    let err = parse_search_quality_inner("nonexistent");
    assert!(err.is_err());
    let err = err.unwrap_err();
    assert!(err.contains("Unknown search mode 'nonexistent'"), "{err}");
    // The accepted forms are named in full, aliases included: the local
    // grammar this replaced accepted `auto_tune` and `auto` but never said so.
    assert!(err.contains("'auto_tune', 'auto'"), "{err}");
}

/// WASM reads the mode through core's parser, not a copy of its grammar: one
/// table of accepted and refused forms gives the same decision and the same
/// message on both, so a form core gains or drops reaches WASM with it.
#[test]
fn test_parse_search_quality_is_cores_grammar() {
    for mode in [
        "fast",
        "AUTO_TUNE",
        "custom:256",
        "+256",
        "custom:+256",
        "adaptive:32:512",
        "adaptive:512:32",
        "adaptive:32",
        "custom:-1",
        "custom:99999999999999999999999",
        " fast",
        "",
    ] {
        assert_eq!(
            parse_search_quality_inner(mode),
            velesdb_core::api_types::parse_search_mode(mode).map(drop),
            "{mode}"
        );
    }
}

/// Each ef is bounded as on every other surface (#2275): WASM applies no ef,
/// but it refuses the modes the server refuses.
#[test]
fn test_parse_advanced_quality_bounds_every_ef() {
    use velesdb_core::api_types::{MAX_EF_SEARCH, MIN_EF_SEARCH};
    for mode in [
        format!("custom:{}", MAX_EF_SEARCH + 1),
        format!("custom:{}", MIN_EF_SEARCH - 1),
        format!("adaptive:0:{MAX_EF_SEARCH}"),
        format!("adaptive:{MIN_EF_SEARCH}:{}", MAX_EF_SEARCH + 1),
    ] {
        let err = parse_search_quality_inner(&mode).expect_err(&mode);
        assert!(
            err.contains("ef_search must be an integer between"),
            "{mode}: {err}"
        );
    }
    assert!(parse_search_quality_inner(&format!("custom:{MAX_EF_SEARCH}")).is_ok());
    assert!(
        parse_search_quality_inner(&format!("adaptive:{MIN_EF_SEARCH}:{MAX_EF_SEARCH}")).is_ok()
    );
}
