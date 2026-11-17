use super::*;

fn rrf(k: u32) -> FusionClause {
    FusionClause {
        strategy: FusionStrategyType::Rrf,
        k: Some(k),
        vector_weight: None,
        graph_weight: None,
        dense_weight: None,
        sparse_weight: None,
    }
}

#[test]
fn test_apply_rrf_returns_combined_ranking() {
    let b1 = vec![(1, 0.9), (2, 0.8), (3, 0.7)];
    let b2 = vec![(3, 0.95), (2, 0.85), (4, 0.75)];
    let fused = apply(&rrf(60), vec![b1, b2]).expect("rrf fuses");
    assert!(!fused.is_empty());
    // Id 2 and 3 appear in both branches and should rank above 1/4.
    let top_two: Vec<u64> = fused.iter().take(2).map(|&(id, _)| id).collect();
    assert!(top_two.contains(&2) || top_two.contains(&3));
}

#[test]
fn test_apply_empty_branches_returns_empty() {
    let fused = apply(&rrf(60), vec![]).expect("nothing to fuse");
    assert!(fused.is_empty());
}

#[test]
fn test_apply_weighted_with_weights_out_of_range_still_ranks() {
    // Weighted with invalid weights (both 2.0) fails validation and
    // must fall back to RRF without panicking.
    let bad = FusionClause {
        strategy: FusionStrategyType::Weighted,
        k: None,
        vector_weight: Some(2.0),
        graph_weight: Some(2.0),
        dense_weight: None,
        sparse_weight: None,
    };
    let fused = apply(&bad, vec![vec![(1, 0.5)], vec![(2, 0.7)]]).expect("fused");
    assert!(!fused.is_empty());
}

#[test]
fn test_apply_maximum_picks_top_score_per_id() {
    let clause = FusionClause {
        strategy: FusionStrategyType::Maximum,
        k: None,
        vector_weight: None,
        graph_weight: None,
        dense_weight: None,
        sparse_weight: None,
    };
    let b1 = vec![(1, 0.2), (2, 0.5)];
    let b2 = vec![(1, 0.9), (3, 0.1)];
    let fused = apply(&clause, vec![b1, b2]).expect("maximum fuses");
    let id1 = fused
        .iter()
        .find(|(id, _)| *id == 1)
        .expect("test: id 1 present");
    // Maximum should pick the 0.9 from branch 2.
    assert!(id1.1 >= 0.85);
}

/// A refusal of the strategy is an error, not an empty ranking that reads
/// as "nothing matched".
#[test]
fn a_fusion_the_strategy_refuses_is_an_error() {
    let weighted_rrf = FusionClause {
        strategy: FusionStrategyType::WeightedRrf,
        k: None,
        vector_weight: None,
        graph_weight: None,
        dense_weight: Some(0.5),
        sparse_weight: Some(0.5),
    };
    let three = vec![vec![(1, 0.5)], vec![(2, 0.7)], vec![(3, 0.1)]];
    let err = apply(&weighted_rrf, three).expect_err("two weights cannot weigh three branches");
    assert!(err.contains("FUSION"), "{err}");
}
