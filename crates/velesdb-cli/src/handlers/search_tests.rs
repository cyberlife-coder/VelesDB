//! The CLI reads fusion strategy names through core's grammar.
//!
//! `multi-search --strategy weighted` and the REPL's `.hybrid-sparse` built
//! `weighted` from the pre-#1545 literals 0.5/0.3/0.2, so the same query
//! ranked differently from the CLI than from the server, Python or WASM; the
//! REPL also read names case-sensitively and refused `avg`.

use super::parse_fusion_strategy;
use velesdb_core::{FusionStrategy, DEFAULT_RRF_K};

#[test]
fn weighted_is_cores_canonical_default() {
    let strategy = parse_fusion_strategy("weighted", DEFAULT_RRF_K).expect("weighted parses");
    assert_eq!(strategy, FusionStrategy::weighted_default());
}

#[test]
fn rrf_takes_the_flag_k() {
    let strategy = parse_fusion_strategy("rrf", 30).expect("rrf parses");
    assert_eq!(strategy, FusionStrategy::RRF { k: 30 });
}

#[test]
fn aliases_and_case_follow_core() {
    for (name, expected) in [
        ("AVG", FusionStrategy::Average),
        ("Max", FusionStrategy::Maximum),
        (
            "RSF",
            FusionStrategy::relative_score(0.5, 0.5).expect("valid"),
        ),
    ] {
        let strategy = parse_fusion_strategy(name, DEFAULT_RRF_K)
            .unwrap_or_else(|e| panic!("'{name}' must parse: {e}"));
        assert_eq!(strategy, expected, "'{name}'");
    }
}

#[test]
fn an_unknown_name_reports_cores_message() {
    let error = parse_fusion_strategy("nope", DEFAULT_RRF_K).expect_err("must be refused");
    assert!(
        error.to_string().contains("Unknown fusion strategy 'nope'"),
        "{error}"
    );
}
