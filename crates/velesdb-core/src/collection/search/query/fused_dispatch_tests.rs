use super::Collection;
use crate::fusion::FusionStrategy;
use crate::velesql::FusionConfig;

/// Builds a `FusionConfig` with the given strategy name and optional `k`.
fn config(strategy: &str, k: Option<f64>) -> FusionConfig {
    let mut params = std::collections::HashMap::new();
    if let Some(k) = k {
        params.insert("k".to_string(), k);
    }
    FusionConfig {
        strategy: strategy.to_string(),
        params,
    }
}

#[test]
fn maps_average_strategy() {
    let strat = Collection::fused_config_to_strategy(&config("average", None));
    assert_eq!(strat, FusionStrategy::Average);
}

#[test]
fn maps_maximum_strategy() {
    let strat = Collection::fused_config_to_strategy(&config("maximum", None));
    assert_eq!(strat, FusionStrategy::Maximum);
}

#[test]
fn maps_rrf_strategy_with_default_k() {
    // No `k` param => the documented default k=60.
    let strat = Collection::fused_config_to_strategy(&config("rrf", None));
    assert_eq!(strat, FusionStrategy::RRF { k: 60 });
}

#[test]
fn rrf_honors_explicit_k() {
    let strat = Collection::fused_config_to_strategy(&config("rrf", Some(42.0)));
    assert_eq!(strat, FusionStrategy::RRF { k: 42 });
}

#[test]
fn unknown_strategy_falls_back_to_rrf() {
    // weighted/rsf/garbage are ill-defined for N homogeneous query vectors
    // and must fall back to RRF (parser default), not error or silently drop.
    for name in ["weighted", "rsf", "nonsense"] {
        let strat = Collection::fused_config_to_strategy(&config(name, Some(7.0)));
        assert_eq!(
            strat,
            FusionStrategy::RRF { k: 7 },
            "strategy '{name}' must fall back to RRF (honoring k)"
        );
    }
}

#[test]
fn strategy_name_is_case_insensitive() {
    assert_eq!(
        Collection::fused_config_to_strategy(&config("AVERAGE", None)),
        FusionStrategy::Average
    );
    assert_eq!(
        Collection::fused_config_to_strategy(&config("Maximum", None)),
        FusionStrategy::Maximum
    );
}
