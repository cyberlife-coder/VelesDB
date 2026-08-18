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
    let fused = apply(&rrf(60), vec![b1, b2]);
    assert!(!fused.is_empty());
    // Id 2 and 3 appear in both branches and should rank above 1/4.
    let top_two: Vec<u64> = fused.iter().take(2).map(|&(id, _)| id).collect();
    assert!(top_two.contains(&2) || top_two.contains(&3));
}

#[test]
fn test_apply_empty_branches_returns_empty() {
    let fused = apply(&rrf(60), vec![]);
    assert!(fused.is_empty());
}

#[test]
fn test_apply_unknown_strategy_falls_back_to_rrf() {
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
    let fused = apply(&bad, vec![vec![(1, 0.5)], vec![(2, 0.7)]]);
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
    let fused = apply(&clause, vec![b1, b2]);
    let id1 = fused
        .iter()
        .find(|(id, _)| *id == 1)
        .expect("test: id 1 present");
    // Maximum should pick the 0.9 from branch 2.
    assert!(id1.1 >= 0.85);
}
