use super::*;
use velesdb_core::DistanceMetric;

fn every_strategy() -> Vec<FusionStrategy> {
    vec![
        FusionStrategy::average(),
        FusionStrategy::maximum(),
        FusionStrategy::rrf(60),
        FusionStrategy {
            inner: CoreFusionStrategy::weighted(0.6, 0.3, 0.1).expect("weights"),
        },
        FusionStrategy::relative_score(0.5, 0.5).expect("weights"),
    ]
}

#[test]
fn fused_direction_is_core_fused_direction_for_every_strategy_and_metric() {
    for strategy in every_strategy() {
        for name in velesdb_core::DISTANCE_METRIC_NAMES {
            let metric = DistanceMetric::parse_alias(name).expect("canonical name parses");
            let core = strategy.inner.fused_direction(&[metric.score_direction()]);
            assert_eq!(
                strategy
                    .fused_higher_is_better(name)
                    .expect("canonical name accepted"),
                core == ScoreDirection::HigherIsBetter,
                "{:?} under {name}",
                strategy.inner
            );
        }
    }
}

#[test]
fn fused_direction_keeps_distance_units_for_average_only_under_a_distance() {
    assert!(!FusionStrategy::average()
        .fused_higher_is_better("euclidean")
        .expect("euclidean"));
    assert!(FusionStrategy::average()
        .fused_higher_is_better("cosine")
        .expect("cosine"));
    assert!(FusionStrategy::rrf(60)
        .fused_higher_is_better("l2")
        .expect("alias of euclidean"));
}

#[test]
fn fused_direction_refuses_an_unknown_metric() {
    pyo3::Python::initialize();
    let err = FusionStrategy::rrf(60)
        .fused_higher_is_better("manhattan")
        .expect_err("unknown metric");
    assert!(err.to_string().contains("Unknown metric"), "{err}");
}
