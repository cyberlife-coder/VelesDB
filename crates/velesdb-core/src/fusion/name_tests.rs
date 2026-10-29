//! The fusion strategy name grammar every surface shares.
//!
//! Seven surfaces each carried their own `match` over the strategy name, and
//! they drifted: the CLI kept the pre-#1545 weighted literals 0.5/0.3/0.2, the
//! REPL read names case-sensitively and refused `avg`, Tauri refused `avg`
//! and `max`. These tests pin the one grammar they now delegate to.

use super::{
    FusionError, FusionParams, FusionStrategy, DEFAULT_RELATIVE_SCORE_DENSE_WEIGHT,
    DEFAULT_RELATIVE_SCORE_SPARSE_WEIGHT, DEFAULT_RRF_K,
};

fn parse(name: &str) -> Result<FusionStrategy, FusionError> {
    FusionStrategy::from_name(name, FusionParams::default())
}

#[test]
fn every_name_and_alias_maps_to_its_strategy() {
    let cases = [
        ("rrf", FusionStrategy::rrf_default()),
        ("average", FusionStrategy::Average),
        ("avg", FusionStrategy::Average),
        ("maximum", FusionStrategy::Maximum),
        ("max", FusionStrategy::Maximum),
        ("weighted", FusionStrategy::weighted_default()),
        (
            "relative_score",
            FusionStrategy::RelativeScore {
                dense_weight: DEFAULT_RELATIVE_SCORE_DENSE_WEIGHT,
                sparse_weight: DEFAULT_RELATIVE_SCORE_SPARSE_WEIGHT,
            },
        ),
        (
            "rsf",
            FusionStrategy::RelativeScore {
                dense_weight: DEFAULT_RELATIVE_SCORE_DENSE_WEIGHT,
                sparse_weight: DEFAULT_RELATIVE_SCORE_SPARSE_WEIGHT,
            },
        ),
    ];
    for (name, expected) in cases {
        assert_eq!(parse(name), Ok(expected), "'{name}'");
    }
}

#[test]
fn names_are_read_case_insensitively() {
    for name in ["RRF", "Avg", "MAXIMUM", "Weighted", "RSF", "Relative_Score"] {
        let lower = parse(&name.to_lowercase()).expect("the lowercase form parses");
        assert_eq!(parse(name), Ok(lower), "'{name}'");
    }
}

/// The CLI built `weighted` from the pre-#1545 literals 0.5/0.3/0.2; the
/// default a name carries is core's, whoever parses it.
#[test]
fn weighted_without_weights_is_the_canonical_default() {
    assert_eq!(parse("weighted"), Ok(FusionStrategy::weighted_default()));
}

#[test]
fn rrf_without_k_uses_the_canonical_k() {
    assert_eq!(parse("rrf"), Ok(FusionStrategy::RRF { k: DEFAULT_RRF_K }));
    assert_eq!(FusionStrategy::default(), FusionStrategy::rrf_default());
}

#[test]
fn supplied_parameters_override_the_defaults() {
    let params = FusionParams::default()
        .with_rrf_k(Some(7))
        .with_avg_weight(Some(0.7))
        .with_max_weight(Some(0.2))
        .with_hit_weight(Some(0.1))
        .with_dense_weight(Some(0.8))
        .with_sparse_weight(Some(0.2));
    assert_eq!(
        FusionStrategy::from_name("rrf", params),
        Ok(FusionStrategy::RRF { k: 7 })
    );
    assert_eq!(
        FusionStrategy::from_name("weighted", params),
        FusionStrategy::weighted(0.7, 0.2, 0.1)
    );
    assert_eq!(
        FusionStrategy::from_name("rsf", params),
        FusionStrategy::relative_score(0.8, 0.2)
    );
}

/// One relative-score weight implies the other: the two must sum to 1.0, so
/// filling the missing one with its 0.5 default would refuse every weight
/// but 0.5 itself.
#[test]
fn one_relative_score_weight_implies_its_complement() {
    let dense_only = FusionParams::default().with_dense_weight(Some(0.7));
    let sparse_only = FusionParams::default().with_sparse_weight(Some(0.25));
    assert_eq!(
        FusionStrategy::from_name("rsf", dense_only),
        FusionStrategy::relative_score(0.7, 1.0 - 0.7)
    );
    assert_eq!(
        FusionStrategy::from_name("rsf", sparse_only),
        FusionStrategy::relative_score(1.0 - 0.25, 0.25)
    );
}

/// The weights go through the validating constructors, so a name never builds
/// a strategy the fusion kernel would refuse later.
#[test]
fn weights_the_constructors_refuse_are_refused() {
    let off_sum = FusionParams::default()
        .with_avg_weight(Some(0.9))
        .with_max_weight(Some(0.9))
        .with_hit_weight(Some(0.9));
    assert!(matches!(
        FusionStrategy::from_name("weighted", off_sum),
        Err(FusionError::InvalidWeightSum { .. })
    ));
    let negative = FusionParams::default()
        .with_dense_weight(Some(-0.5))
        .with_sparse_weight(Some(1.5));
    assert!(matches!(
        FusionStrategy::from_name("rsf", negative),
        Err(FusionError::NegativeWeight { .. })
    ));
    let nan = FusionParams::default().with_avg_weight(Some(f32::NAN));
    assert!(matches!(
        FusionStrategy::from_name("weighted", nan),
        Err(FusionError::NonFiniteWeight { .. })
    ));
}

/// Parameters a strategy does not read are ignored, not refused: a request
/// body carries every field with its default whatever strategy it names.
#[test]
fn parameters_of_another_strategy_are_ignored() {
    let params = FusionParams::default()
        .with_avg_weight(Some(9.0))
        .with_dense_weight(Some(9.0));
    assert_eq!(
        FusionStrategy::from_name("rrf", params),
        Ok(FusionStrategy::rrf_default())
    );
    assert_eq!(
        FusionStrategy::from_name("max", params),
        Ok(FusionStrategy::Maximum)
    );
}

#[test]
fn an_unknown_name_is_refused_with_the_accepted_names() {
    let error = parse("NoSuchStrategy").expect_err("an unknown name must be refused");
    assert_eq!(
        error,
        FusionError::UnknownStrategy {
            name: "NoSuchStrategy".to_string()
        }
    );
    let message = error.to_string();
    assert!(message.contains("'NoSuchStrategy'"), "{message}");
    for name in [
        "rrf",
        "rsf",
        "relative_score",
        "average",
        "avg",
        "maximum",
        "max",
        "weighted",
    ] {
        assert!(message.contains(&format!("'{name}'")), "{name}: {message}");
    }
}

/// The grammar is exact: no trimming, no prefix match, and the
/// `weighted_rrf` strategy, which needs one weight per branch, has no name.
#[test]
fn near_misses_are_refused() {
    for name in ["", " rrf", "rrf ", "r", "weighted_rrf", "averages"] {
        assert!(
            matches!(parse(name), Err(FusionError::UnknownStrategy { .. })),
            "'{name}' must be refused"
        );
    }
}
