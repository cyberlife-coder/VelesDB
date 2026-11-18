use super::Collection;
use crate::distance::DistanceMetric;
use crate::fusion::FusionStrategy;
use crate::point::Point;
use crate::velesql::FusionConfig;
use std::collections::HashMap;
use tempfile::TempDir;

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
    let strat = config("average", None).near_fused_strategy();
    assert_eq!(strat, FusionStrategy::Average);
}

#[test]
fn maps_maximum_strategy() {
    let strat = config("maximum", None).near_fused_strategy();
    assert_eq!(strat, FusionStrategy::Maximum);
}

#[test]
fn maps_rrf_strategy_with_default_k() {
    // No `k` param => the documented default k=60.
    let strat = config("rrf", None).near_fused_strategy();
    assert_eq!(strat, FusionStrategy::RRF { k: 60 });
}

#[test]
fn rrf_honors_explicit_k() {
    let strat = config("rrf", Some(42.0)).near_fused_strategy();
    assert_eq!(strat, FusionStrategy::RRF { k: 42 });
}

#[test]
fn unknown_strategy_falls_back_to_rrf() {
    // weighted/rsf/garbage are ill-defined for N homogeneous query vectors
    // and must fall back to RRF (parser default), not error or silently drop.
    for name in ["weighted", "rsf", "nonsense"] {
        let strat = config(name, Some(7.0)).near_fused_strategy();
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
        config("AVERAGE", None).near_fused_strategy(),
        FusionStrategy::Average
    );
    assert_eq!(
        config("Maximum", None).near_fused_strategy(),
        FusionStrategy::Maximum
    );
}

/// A `NEAR_FUSED` query reaches the fused search through the extraction of its
/// WHERE clause (#2403): the two query vectors rank the points differently
/// (`$a` alone gives `[10, 30, 20, 40]`, `$b` alone `[30, 20, 10, 40]`), and
/// only the fusion of both gives `[30, 10, 20, 40]`. A query whose vectors were
/// never extracted, or only one of them, would not return this order.
#[test]
fn a_near_fused_query_returns_the_fused_hits_best_first() {
    let dir = TempDir::new().expect("test: temp dir");
    let col = Collection::create(dir.path().join("fused"), 2, DistanceMetric::Cosine)
        .expect("test: create collection");
    let point = |id: u64, vector: [f32; 2]| Point {
        id,
        vector: vector.to_vec(),
        payload: Some(serde_json::json!({ "id": id })),
        sparse_vectors: None,
    };
    col.upsert(vec![
        point(10, [1.0, 0.0]),
        point(20, [0.0, 1.0]),
        point(30, [1.0, 1.0]),
        point(40, [-1.0, 0.0]),
    ])
    .expect("test: upsert");
    let params = HashMap::from([
        ("a".to_string(), serde_json::json!([1.0, 0.0])),
        ("b".to_string(), serde_json::json!([0.5, 1.0])),
    ]);

    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR_FUSED [$a, $b] LIMIT 4",
            &params,
        )
        .expect("test: the fused query runs");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, [30, 10, 20, 40]);
}

/// A scalar `ORDER BY` on a `NEAR_FUSED` query ranks every fused hit, not the
/// `LIMIT` best: fusion ranks id 40 last, yet its `rank` is the lowest, so
/// `ORDER BY rank LIMIT 1` must return it rather than the fused top hit (30).
#[test]
fn a_near_fused_query_with_a_scalar_order_by_ranks_every_hit() {
    let dir = TempDir::new().expect("test: temp dir");
    let col = Collection::create(dir.path().join("fused"), 2, DistanceMetric::Cosine)
        .expect("test: create collection");
    let point = |id: u64, vector: [f32; 2], rank: u64| Point {
        id,
        vector: vector.to_vec(),
        payload: Some(serde_json::json!({ "rank": rank })),
        sparse_vectors: None,
    };
    col.upsert(vec![
        point(10, [1.0, 0.0], 3),
        point(20, [0.0, 1.0], 2),
        point(30, [1.0, 1.0], 4),
        point(40, [-1.0, 0.0], 1),
    ])
    .expect("test: upsert");
    let params = HashMap::from([
        ("a".to_string(), serde_json::json!([1.0, 0.0])),
        ("b".to_string(), serde_json::json!([0.5, 1.0])),
    ]);

    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR_FUSED [$a, $b] ORDER BY rank ASC LIMIT 1",
            &params,
        )
        .expect("test: the fused query runs");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, [40]);
}
