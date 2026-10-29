use super::*;
use velesdb_core::DistanceMetric;

/// Error construction and formatting go through the Python C-API.
fn init_python() {
    pyo3::Python::initialize();
}

#[test]
fn hash_id_is_core_hash_id() {
    // The empty string hashes to the FNV-1a offset basis, core's own
    // conformance vector (`conformance::hash`).
    assert_eq!(hash_id(""), 0xcbf2_9ce4_8422_2325);
    for value in ["a", "tenant:acme", "café", "日本語", "emoji:🚀"] {
        assert_eq!(hash_id(value), velesdb_core::hash_id(value), "{value}");
    }
}

#[test]
fn metric_direction_is_core_direction_for_every_metric() {
    for name in velesdb_core::DISTANCE_METRIC_NAMES {
        let metric = DistanceMetric::parse_alias(name).expect("canonical name parses");
        assert_eq!(
            metric_higher_is_better(name).expect("canonical name accepted"),
            metric.higher_is_better(),
            "{name}"
        );
    }
}

#[test]
fn jaccard_is_a_similarity_and_euclidean_a_distance() {
    assert!(metric_higher_is_better("jaccard").expect("jaccard"));
    assert!(!metric_higher_is_better("euclidean").expect("euclidean"));
    assert!(!metric_higher_is_better("l2").expect("alias of euclidean"));
}

#[test]
fn metric_direction_refuses_an_unknown_metric() {
    init_python();
    let err = metric_higher_is_better("manhattan").expect_err("unknown metric");
    assert!(err.to_string().contains("Unknown metric"), "got: {err}");
}

#[test]
fn search_quality_accepts_what_core_accepts() {
    // `custom:+256` is core's grammar (an integer with a leading `+`), which a
    // hand-written `custom:\d+` copy refused.
    for mode in [
        "fast",
        "AUTO",
        "custom:16",
        "custom:+256",
        "adaptive:32:512",
    ] {
        assert!(validate_search_quality(mode).is_ok(), "{mode}");
    }
}

#[test]
fn search_quality_refuses_with_core_message() {
    init_python();
    let err = validate_search_quality("warp").expect_err("unknown mode");
    assert!(
        err.to_string().contains("Unknown search mode 'warp'"),
        "got: {err}"
    );
    let err = validate_search_quality("custom:8").expect_err("ef below range");
    assert!(
        err.to_string().contains("ef_search must be an integer"),
        "got: {err}"
    );
}
