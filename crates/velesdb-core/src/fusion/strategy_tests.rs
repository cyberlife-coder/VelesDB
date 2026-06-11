//! Tests for `FusionStrategy` implementations.

use super::strategy::{FusionError, FusionStrategy};

// =============================================================================
// Test helpers
// =============================================================================

/// Creates sample search results for testing.
/// Each inner Vec represents results from one query: `Vec<(doc_id, score)>`
fn sample_results() -> Vec<Vec<(u64, f32)>> {
    vec![
        // Query 1 results
        vec![(1, 0.95), (2, 0.85), (3, 0.75), (4, 0.65)],
        // Query 2 results
        vec![(2, 0.90), (1, 0.80), (5, 0.70), (3, 0.60)],
        // Query 3 results
        vec![(1, 0.92), (3, 0.82), (2, 0.72), (6, 0.62)],
    ]
}

/// Results where documents appear in different subsets of queries.
fn partial_overlap_results() -> Vec<Vec<(u64, f32)>> {
    vec![
        vec![(1, 0.9), (2, 0.8)],   // Doc 1, 2
        vec![(2, 0.85), (3, 0.75)], // Doc 2, 3
        vec![(3, 0.8), (4, 0.7)],   // Doc 3, 4
    ]
}

/// Single query results (edge case).
fn single_query_results() -> Vec<Vec<(u64, f32)>> {
    vec![vec![(1, 0.95), (2, 0.85), (3, 0.75)]]
}

/// Empty results (edge case).
fn empty_results() -> Vec<Vec<(u64, f32)>> {
    vec![]
}

/// Results with empty queries.
fn results_with_empty_query() -> Vec<Vec<(u64, f32)>> {
    vec![
        vec![(1, 0.9), (2, 0.8)],
        vec![], // Empty query
        vec![(1, 0.85), (3, 0.75)],
    ]
}

// =============================================================================
// AC1: FusionStrategy::Average tests
// =============================================================================

#[test]
fn test_average_basic() {
    let strategy = FusionStrategy::Average;
    let results = sample_results();

    let fused = strategy.fuse(results).unwrap();

    // Doc 1 appears in all 3 queries: (0.95 + 0.80 + 0.92) / 3 = 0.89
    // Doc 2 appears in all 3 queries: (0.85 + 0.90 + 0.72) / 3 = 0.823...
    // Doc 3 appears in all 3 queries: (0.75 + 0.60 + 0.82) / 3 = 0.723...

    assert!(!fused.is_empty());

    // Check doc 1 is present and has correct average
    let doc1 = fused.iter().find(|(id, _)| *id == 1).unwrap();
    assert!(
        (doc1.1 - 0.89).abs() < 0.01,
        "Doc 1 avg should be ~0.89, got {}",
        doc1.1
    );

    // Results should be sorted by score descending
    for i in 1..fused.len() {
        assert!(
            fused[i - 1].1 >= fused[i].1,
            "Results should be sorted descending"
        );
    }
}

#[test]
fn test_average_partial_overlap() {
    let strategy = FusionStrategy::Average;
    let results = partial_overlap_results();

    let fused = strategy.fuse(results).unwrap();

    // Doc 1: only in query 1 → 0.9 / 1 = 0.9
    // Doc 2: in query 1 and 2 → (0.8 + 0.85) / 2 = 0.825
    // Doc 3: in query 2 and 3 → (0.75 + 0.8) / 2 = 0.775
    // Doc 4: only in query 3 → 0.7 / 1 = 0.7

    let doc1 = fused.iter().find(|(id, _)| *id == 1).unwrap();
    assert!((doc1.1 - 0.9).abs() < 0.01);

    let doc2 = fused.iter().find(|(id, _)| *id == 2).unwrap();
    assert!((doc2.1 - 0.825).abs() < 0.01);
}

#[test]
fn test_average_single_query() {
    let strategy = FusionStrategy::Average;
    let results = single_query_results();

    let fused = strategy.fuse(results).unwrap();

    // Single query: average = original scores
    assert_eq!(fused.len(), 3);
    assert!((fused[0].1 - 0.95).abs() < 0.001);
}

#[test]
fn test_average_empty_input() {
    let strategy = FusionStrategy::Average;
    let results = empty_results();

    let fused = strategy.fuse(results).unwrap();
    assert!(fused.is_empty());
}

#[test]
fn test_average_with_empty_query() {
    let strategy = FusionStrategy::Average;
    let results = results_with_empty_query();

    let fused = strategy.fuse(results).unwrap();

    // Should handle empty queries gracefully
    assert!(!fused.is_empty());
}

// =============================================================================
// AC2: FusionStrategy::Maximum tests
// =============================================================================

#[test]
fn test_maximum_basic() {
    let strategy = FusionStrategy::Maximum;
    let results = sample_results();

    let fused = strategy.fuse(results).unwrap();

    // Doc 1: max(0.95, 0.80, 0.92) = 0.95
    // Doc 2: max(0.85, 0.90, 0.72) = 0.90
    // Doc 3: max(0.75, 0.60, 0.82) = 0.82

    let doc1 = fused.iter().find(|(id, _)| *id == 1).unwrap();
    assert!(
        (doc1.1 - 0.95).abs() < 0.001,
        "Doc 1 max should be 0.95, got {}",
        doc1.1
    );

    let doc2 = fused.iter().find(|(id, _)| *id == 2).unwrap();
    assert!(
        (doc2.1 - 0.90).abs() < 0.001,
        "Doc 2 max should be 0.90, got {}",
        doc2.1
    );

    // Sorted descending
    for i in 1..fused.len() {
        assert!(fused[i - 1].1 >= fused[i].1);
    }
}

#[test]
fn test_maximum_partial_overlap() {
    let strategy = FusionStrategy::Maximum;
    let results = partial_overlap_results();

    let fused = strategy.fuse(results).unwrap();

    // Doc 2: max(0.8, 0.85) = 0.85
    let doc2 = fused.iter().find(|(id, _)| *id == 2).unwrap();
    assert!((doc2.1 - 0.85).abs() < 0.001);
}

#[test]
fn test_maximum_single_query() {
    let strategy = FusionStrategy::Maximum;
    let results = single_query_results();

    let fused = strategy.fuse(results).unwrap();

    // Single query: max = original scores
    assert_eq!(fused.len(), 3);
    assert!((fused[0].1 - 0.95).abs() < 0.001);
}

// =============================================================================
// AC3: FusionStrategy::RRF tests
// =============================================================================

#[test]
fn test_rrf_basic() {
    let strategy = FusionStrategy::RRF { k: 60 };
    let results = sample_results();

    let fused = strategy.fuse(results).unwrap();

    // RRF formula: score = Σ 1/(k + rank)
    // Doc 1: rank 1 in Q1, rank 2 in Q2, rank 1 in Q3
    //        1/(60+1) + 1/(60+2) + 1/(60+1) = 0.01639 + 0.01613 + 0.01639 = 0.04891

    assert!(!fused.is_empty());

    // Doc 1 should have high score (appears high in all queries)
    let doc1 = fused.iter().find(|(id, _)| *id == 1).unwrap();
    assert!(
        doc1.1 > 0.04,
        "Doc 1 RRF score should be > 0.04, got {}",
        doc1.1
    );

    // Sorted descending
    for i in 1..fused.len() {
        assert!(fused[i - 1].1 >= fused[i].1);
    }
}

#[test]
fn test_rrf_k_parameter() {
    let results = sample_results();

    // Lower k = more weight to top ranks
    let strategy_low_k = FusionStrategy::RRF { k: 1 };
    let strategy_high_k = FusionStrategy::RRF { k: 100 };

    let fused_low = strategy_low_k.fuse(results.clone()).unwrap();
    let fused_high = strategy_high_k.fuse(results).unwrap();

    // With lower k, scores should be higher overall
    let doc1_low = fused_low.iter().find(|(id, _)| *id == 1).unwrap();
    let doc1_high = fused_high.iter().find(|(id, _)| *id == 1).unwrap();

    assert!(
        doc1_low.1 > doc1_high.1,
        "Lower k should yield higher scores"
    );
}

#[test]
fn test_rrf_default_k() {
    // Default k=60 is standard in literature
    let strategy = FusionStrategy::rrf_default();

    match strategy {
        FusionStrategy::RRF { k } => assert_eq!(k, 60),
        _ => panic!("Expected RRF variant"),
    }
}

#[test]
fn test_rrf_single_query() {
    let strategy = FusionStrategy::RRF { k: 60 };
    let results = single_query_results();

    let fused = strategy.fuse(results).unwrap();

    // Single query: RRF based on single ranking
    assert_eq!(fused.len(), 3);

    // First doc should have highest RRF score
    assert!(fused[0].1 > fused[1].1);
}

// =============================================================================
// AC4: FusionStrategy::Weighted tests
// =============================================================================

#[test]
fn test_weighted_basic() {
    let strategy = FusionStrategy::Weighted {
        avg_weight: 0.6,
        max_weight: 0.3,
        hit_weight: 0.1,
    };
    let results = sample_results();

    let fused = strategy.fuse(results).unwrap();

    // Doc 1: avg=0.89, max=0.95, hits=3/3=1.0
    // score = 0.6*0.89 + 0.3*0.95 + 0.1*1.0 = 0.534 + 0.285 + 0.1 = 0.919

    let doc1 = fused.iter().find(|(id, _)| *id == 1).unwrap();
    assert!(
        (doc1.1 - 0.919).abs() < 0.02,
        "Doc 1 weighted should be ~0.919, got {}",
        doc1.1
    );

    // Sorted descending
    for i in 1..fused.len() {
        assert!(fused[i - 1].1 >= fused[i].1);
    }
}

#[test]
fn test_weighted_partial_overlap() {
    let strategy = FusionStrategy::Weighted {
        avg_weight: 0.6,
        max_weight: 0.3,
        hit_weight: 0.1,
    };
    let results = partial_overlap_results(); // 3 queries

    let fused = strategy.fuse(results).unwrap();

    // Doc 1: avg=0.9, max=0.9, hits=1/3=0.333
    // score = 0.6*0.9 + 0.3*0.9 + 0.1*0.333 = 0.54 + 0.27 + 0.0333 = 0.8433

    // Doc 2: avg=0.825, max=0.85, hits=2/3=0.667
    // score = 0.6*0.825 + 0.3*0.85 + 0.1*0.667 = 0.495 + 0.255 + 0.0667 = 0.8167

    let doc1 = fused.iter().find(|(id, _)| *id == 1).unwrap();
    let doc2 = fused.iter().find(|(id, _)| *id == 2).unwrap();

    // Doc 1 should rank higher due to higher avg/max despite lower hit count
    assert!(doc1.1 > doc2.1);
}

#[test]
fn test_weighted_validation_sum_to_one() {
    // Weights must sum to 1.0
    let result = FusionStrategy::weighted(0.5, 0.3, 0.1);
    assert!(result.is_err(), "Weights summing to 0.9 should fail");

    let result = FusionStrategy::weighted(0.6, 0.3, 0.1);
    assert!(result.is_ok(), "Weights summing to 1.0 should succeed");

    let result = FusionStrategy::weighted(0.5, 0.5, 0.1);
    assert!(result.is_err(), "Weights summing to 1.1 should fail");
}

#[test]
fn test_weighted_validation_non_negative() {
    let result = FusionStrategy::weighted(-0.1, 0.6, 0.5);
    assert!(result.is_err(), "Negative weights should fail");
}

#[test]
fn test_weighted_zero_hit_weight() {
    // hit_weight = 0 is valid (only use avg and max)
    let strategy = FusionStrategy::Weighted {
        avg_weight: 0.7,
        max_weight: 0.3,
        hit_weight: 0.0,
    };
    let results = sample_results();

    let fused = strategy.fuse(results).unwrap();
    assert!(!fused.is_empty());
}

// =============================================================================
// Edge cases and error handling
// =============================================================================

#[test]
fn test_fuse_preserves_all_documents() {
    let strategy = FusionStrategy::Average;
    let results = sample_results();

    let fused = strategy.fuse(results).unwrap();

    // All unique doc IDs should be present: 1, 2, 3, 4, 5, 6
    let ids: std::collections::HashSet<u64> = fused.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
    assert!(ids.contains(&3));
    assert!(ids.contains(&4));
    assert!(ids.contains(&5));
    assert!(ids.contains(&6));
}

#[test]
fn test_fuse_handles_duplicate_ids_in_same_query() {
    let strategy = FusionStrategy::Average;
    let results = vec![
        vec![(1, 0.9), (1, 0.8), (2, 0.7)], // Doc 1 appears twice (should take best)
    ];

    let fused = strategy.fuse(results).unwrap();

    // Doc 1 should appear once with the best score
    let doc1_count = fused.iter().filter(|(id, _)| *id == 1).count();
    assert_eq!(doc1_count, 1, "Doc 1 should appear exactly once");
}

#[test]
fn test_fuse_score_bounds() {
    let strategy = FusionStrategy::Average;
    let results = sample_results();

    let fused = strategy.fuse(results).unwrap();

    // All scores should be in [0, 1] range for Average
    for (_, score) in &fused {
        assert!(
            *score >= 0.0 && *score <= 1.0,
            "Score {score} out of bounds"
        );
    }
}

#[test]
fn test_rrf_scores_are_positive() {
    let strategy = FusionStrategy::RRF { k: 60 };
    let results = sample_results();

    let fused = strategy.fuse(results).unwrap();

    for (_, score) in &fused {
        assert!(*score > 0.0, "RRF score should be positive");
    }
}

// =============================================================================
// AC5: FusionStrategy::RelativeScore tests
// =============================================================================

#[test]
fn test_rsf_normalization() {
    // Test min-max normalization on known inputs
    let strategy = FusionStrategy::relative_score(0.5, 0.5).unwrap();

    // Dense: scores 1.0, 2.0, 3.0 -> normalized 0.0, 0.5, 1.0
    // Sparse: scores 10.0, 20.0 -> normalized 0.0, 1.0
    let results = vec![
        vec![(1, 1.0_f32), (2, 2.0), (3, 3.0)], // dense
        vec![(2, 10.0_f32), (4, 20.0)],         // sparse
    ];

    let fused = strategy.fuse(results).unwrap();

    // doc 3: dense_norm=1.0, sparse=missing(0) -> 0.5*1.0 + 0.5*0.0 = 0.5
    // doc 4: dense=missing(0), sparse_norm=1.0 -> 0.5*0.0 + 0.5*1.0 = 0.5
    // doc 2: dense_norm=0.5, sparse_norm=0.0 -> 0.5*0.5 + 0.5*0.0 = 0.25
    // doc 1: dense_norm=0.0, sparse=missing(0) -> 0.0
    let find = |id: u64| fused.iter().find(|(i, _)| *i == id).unwrap().1;
    assert!((find(3) - 0.5).abs() < 1e-5);
    assert!((find(4) - 0.5).abs() < 1e-5);
    assert!((find(2) - 0.25).abs() < 1e-5);
    assert!((find(1) - 0.0).abs() < 1e-5);
}

#[test]
fn test_rsf_normalization_equal_scores() {
    // Edge case: all scores equal -> all get 0.5
    let strategy = FusionStrategy::relative_score(0.5, 0.5).unwrap();
    let results = vec![
        vec![(1, 5.0_f32), (2, 5.0), (3, 5.0)],
        vec![(1, 3.0_f32), (4, 3.0)],
    ];

    let fused = strategy.fuse(results).unwrap();
    let find = |id: u64| fused.iter().find(|(i, _)| *i == id).unwrap().1;

    // Dense all 0.5, sparse all 0.5
    // doc 1: 0.5*0.5 + 0.5*0.5 = 0.5
    // doc 2: 0.5*0.5 + 0.5*0.0 = 0.25
    // doc 4: 0.5*0.0 + 0.5*0.5 = 0.25
    assert!((find(1) - 0.5).abs() < 1e-5);
    assert!((find(2) - 0.25).abs() < 1e-5);
    assert!((find(4) - 0.25).abs() < 1e-5);
}

#[test]
fn test_rsf_fuse_two_branches() {
    // Weighted combination: dense=0.7, sparse=0.3
    let strategy = FusionStrategy::relative_score(0.7, 0.3).unwrap();
    let results = vec![
        vec![(1, 10.0_f32), (2, 8.0), (3, 6.0)], // dense
        vec![(3, 5.0_f32), (4, 3.0), (1, 1.0)],  // sparse
    ];

    let fused = strategy.fuse(results).unwrap();

    // Dense norm: 1->1.0, 2->0.5, 3->0.0
    // Sparse norm: 3->1.0, 4->0.5, 1->0.0
    // doc 1: 0.7*1.0 + 0.3*0.0 = 0.7
    // doc 3: 0.7*0.0 + 0.3*1.0 = 0.3
    // doc 2: 0.7*0.5 + 0.3*0.0 = 0.35
    // doc 4: 0.7*0.0 + 0.3*0.5 = 0.15
    assert_eq!(fused[0].0, 1); // 0.7
    assert_eq!(fused[1].0, 2); // 0.35
    assert_eq!(fused[2].0, 3); // 0.3
    assert_eq!(fused[3].0, 4); // 0.15
}

#[test]
fn test_rsf_single_branch_empty() {
    // Dense empty -> only sparse results pass through
    let strategy = FusionStrategy::relative_score(0.5, 0.5).unwrap();
    let results = vec![
        vec![],                                 // empty dense
        vec![(1, 5.0_f32), (2, 3.0), (3, 1.0)], // sparse
    ];

    let fused = strategy.fuse(results).unwrap();
    assert_eq!(fused.len(), 3);
    // All sparse-only: dense is 0.0
    // Sparse norm: 1->1.0, 2->0.5, 3->0.0
    // doc 1: 0.5*0.0 + 0.5*1.0 = 0.5
    // doc 2: 0.5*0.0 + 0.5*0.5 = 0.25
    // doc 3: 0.5*0.0 + 0.5*0.0 = 0.0
    assert_eq!(fused[0].0, 1);
    assert!((fused[0].1 - 0.5).abs() < 1e-5);
}

#[test]
fn test_rsf_validation_negative_weight() {
    let result = FusionStrategy::relative_score(-0.1, 1.1);
    assert!(result.is_err());
}

#[test]
fn test_rsf_validation_sum_not_one() {
    let result = FusionStrategy::relative_score(0.3, 0.3);
    assert!(result.is_err());
}

#[test]
fn test_weighted_fuse_rejects_invalid_weight_sum_from_literal() {
    // Direct enum-literal construction bypasses `FusionStrategy::weighted()`,
    // so the validating constructor cannot guard against unnormalized weights.
    // The fuse path must reject a weight sum that is not 1.0.
    let strategy = FusionStrategy::Weighted {
        avg_weight: 0.9,
        max_weight: 0.9,
        hit_weight: 0.9,
    };
    let results = vec![vec![(1_u64, 1.0_f32)]];
    let result = strategy.fuse(results);
    assert!(
        matches!(result, Err(FusionError::InvalidWeightSum { .. })),
        "Weighted fuse must reject weights that do not sum to 1.0, got {result:?}"
    );
}

#[test]
fn test_weighted_fuse_rejects_negative_weight_from_literal() {
    let strategy = FusionStrategy::Weighted {
        avg_weight: -0.5,
        max_weight: 0.75,
        hit_weight: 0.75,
    };
    let results = vec![vec![(1_u64, 1.0_f32)]];
    let result = strategy.fuse(results);
    assert!(
        matches!(result, Err(FusionError::NegativeWeight { .. })),
        "Weighted fuse must reject negative weights, got {result:?}"
    );
}

#[test]
fn test_rsf_fuse_rejects_invalid_weight_sum_from_literal() {
    let strategy = FusionStrategy::RelativeScore {
        dense_weight: 0.3,
        sparse_weight: 0.3,
    };
    let results = vec![vec![(1_u64, 1.0_f32)], vec![(2_u64, 1.0_f32)]];
    let result = strategy.fuse(results);
    assert!(
        matches!(result, Err(FusionError::InvalidWeightSum { .. })),
        "RelativeScore fuse must reject weights that do not sum to 1.0, got {result:?}"
    );
}

#[test]
fn test_rsf_fuse_rejects_negative_weight_from_literal() {
    let strategy = FusionStrategy::RelativeScore {
        dense_weight: -0.1,
        sparse_weight: 1.1,
    };
    let results = vec![vec![(1_u64, 1.0_f32)], vec![(2_u64, 1.0_f32)]];
    let result = strategy.fuse(results);
    assert!(
        matches!(result, Err(FusionError::NegativeWeight { .. })),
        "RelativeScore fuse must reject negative weights, got {result:?}"
    );
}

#[test]
fn test_rsf_ignores_extra_branches_beyond_two() {
    // M-7: fuse_relative_score silently drops branches beyond index 1.
    // The result must be identical to passing only the first two branches.
    let strategy = FusionStrategy::relative_score(0.6, 0.4).unwrap();

    let two_branches = vec![
        vec![(1_u64, 10.0_f32), (2, 8.0)], // dense
        vec![(2_u64, 5.0_f32), (3, 3.0)],  // sparse
    ];
    let three_branches = vec![
        vec![(1_u64, 10.0_f32), (2, 8.0)], // dense
        vec![(2_u64, 5.0_f32), (3, 3.0)],  // sparse
        vec![(4_u64, 99.0_f32)],           // extra (must be ignored)
    ];

    let fused_two = strategy.fuse(two_branches).unwrap();
    let fused_three = strategy.fuse(three_branches).unwrap();

    // Extra branch doc 4 must NOT appear in the results.
    assert!(
        !fused_three.iter().any(|(id, _)| *id == 4),
        "doc 4 from the extra branch must be absent from RSF output"
    );

    // Scores for the shared documents must be identical.
    for (id, score) in &fused_two {
        let matching = fused_three.iter().find(|(i, _)| i == id);
        assert!(
            matching.is_some(),
            "doc {id} must appear in three-branch result"
        );
        let (_, score_three) = matching.unwrap();
        assert!(
            (score - score_three).abs() < 1e-5,
            "score for doc {id} must not change when an extra branch is present"
        );
    }
}

// =============================================================================
// WeightedRRF tests (EPIC-040)
// =============================================================================

#[test]
fn test_weighted_rrf_basic_two_branches() {
    let strategy = FusionStrategy::weighted_rrf(vec![0.7, 0.3], 60.0).unwrap();
    // Branch 0: doc 1 rank 0, doc 2 rank 1
    // Branch 1: doc 2 rank 0, doc 3 rank 1
    let results = vec![
        vec![(1u64, 0.9), (2u64, 0.8)],
        vec![(2u64, 0.85), (3u64, 0.7)],
    ];
    let fused = strategy.fuse(results).unwrap();

    // doc 1: 0.7/(0+60) = 0.01167
    // doc 2: 0.7/(1+60) + 0.3/(0+60) = 0.01148 + 0.005 = 0.01648
    // doc 3: 0.3/(1+60) = 0.00492
    // Expected order: doc2 > doc1 > doc3
    assert_eq!(fused[0].0, 2, "doc2 highest (appears in both branches)");
    assert_eq!(fused[1].0, 1, "doc1 second");
    assert_eq!(fused[2].0, 3, "doc3 last");
}

#[test]
fn test_weighted_rrf_scores_use_zero_based_ranks() {
    let strategy = FusionStrategy::weighted_rrf(vec![1.0], 60.0).unwrap();
    let results = vec![vec![(10u64, 0.0), (20u64, 0.0), (30u64, 0.0)]];
    let fused = strategy.fuse(results).unwrap();

    // rank 0: 1.0/60  rank 1: 1.0/61  rank 2: 1.0/62
    let score_10 = fused.iter().find(|(id, _)| *id == 10).unwrap().1;
    let score_20 = fused.iter().find(|(id, _)| *id == 20).unwrap().1;
    let score_30 = fused.iter().find(|(id, _)| *id == 30).unwrap().1;

    assert!((score_10 - 1.0 / 60.0).abs() < 1e-6, "rank-0 score");
    assert!((score_20 - 1.0 / 61.0).abs() < 1e-6, "rank-1 score");
    assert!((score_30 - 1.0 / 62.0).abs() < 1e-6, "rank-2 score");
}

#[test]
fn test_weighted_rrf_weight_count_mismatch_is_error() {
    let strategy = FusionStrategy::weighted_rrf(vec![0.5, 0.5], 60.0).unwrap();
    let results = vec![vec![(1u64, 0.9)]]; // 1 branch but 2 weights
    let err = strategy.fuse(results).unwrap_err();
    assert!(
        matches!(
            err,
            FusionError::WeightCountMismatch {
                weights: 2,
                branches: 1
            }
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn test_weighted_rrf_negative_weight_rejected_at_construction() {
    let err = FusionStrategy::weighted_rrf(vec![0.7, -0.3], 60.0).unwrap_err();
    assert!(matches!(err, FusionError::NegativeWeight { .. }));
}

#[test]
fn test_weighted_rrf_zero_k_rejected_at_construction() {
    let err = FusionStrategy::weighted_rrf(vec![0.5, 0.5], 0.0).unwrap_err();
    assert!(matches!(err, FusionError::NegativeWeight { .. }));
}

#[test]
fn test_weighted_rrf_empty_branches_returns_empty() {
    let strategy = FusionStrategy::weighted_rrf(vec![], 60.0).unwrap();
    let fused = strategy.fuse(vec![]).unwrap();
    assert!(fused.is_empty());
}

#[test]
fn test_weighted_rrf_duplicate_ids_within_branch_use_best_rank() {
    // Doc 99 appears twice in branch 0; only rank 0 should count.
    let strategy = FusionStrategy::weighted_rrf(vec![1.0], 60.0).unwrap();
    let results = vec![vec![(99u64, 0.9), (99u64, 0.1)]];
    let fused = strategy.fuse(results).unwrap();
    // Should appear once with score = 1.0/60 (rank 0)
    assert_eq!(fused.len(), 1);
    assert!((fused[0].1 - 1.0 / 60.0).abs() < 1e-6);
}

#[test]
fn test_weighted_rrf_doc_absent_from_branch_contributes_zero() {
    // Doc 5 only in branch 1, doc 7 only in branch 0.
    let strategy = FusionStrategy::weighted_rrf(vec![0.6, 0.4], 60.0).unwrap();
    let results = vec![vec![(7u64, 0.9)], vec![(5u64, 0.9)]];
    let fused = strategy.fuse(results).unwrap();

    let score_7 = fused.iter().find(|(id, _)| *id == 7).unwrap().1;
    let score_5 = fused.iter().find(|(id, _)| *id == 5).unwrap().1;
    // doc7: 0.6/60  doc5: 0.4/60  → doc7 wins
    assert!((score_7 - 0.6 / 60.0).abs() < 1e-6);
    assert!((score_5 - 0.4 / 60.0).abs() < 1e-6);
    assert!(score_7 > score_5);
}

#[test]
fn test_weighted_rrf_result_sorted_descending() {
    let strategy = FusionStrategy::weighted_rrf(vec![0.5, 0.5], 60.0).unwrap();
    let results = vec![
        vec![(1u64, 0.9), (2u64, 0.8), (3u64, 0.7)],
        vec![(3u64, 0.95), (1u64, 0.85), (2u64, 0.75)],
    ];
    let fused = strategy.fuse(results).unwrap();
    for w in fused.windows(2) {
        assert!(w[0].1 >= w[1].1, "results must be sorted descending");
    }
}

#[test]
fn test_weighted_rrf_unequal_weights_change_ranking() {
    // With equal weights doc1 (rank 0 in both) > doc2.
    // With heavy weight on branch 1 where doc2 is rank 0, ranking may change.
    let equal = FusionStrategy::weighted_rrf(vec![0.5, 0.5], 60.0).unwrap();
    let biased = FusionStrategy::weighted_rrf(vec![0.1, 0.9], 60.0).unwrap();
    let results = vec![
        vec![(1u64, 0.9), (2u64, 0.5)], // branch 0: doc1 best
        vec![(2u64, 0.9), (1u64, 0.5)], // branch 1: doc2 best
    ];
    let fused_equal = equal.fuse(results.clone()).unwrap();
    let fused_biased = biased.fuse(results).unwrap();

    // Equal weights: both docs appear rank-0 in one branch, so scores are symmetric.
    assert!(
        (fused_equal[0].1 - fused_equal[1].1).abs() < f32::EPSILON,
        "equal weights → equal scores"
    );
    // Biased toward branch 1 where doc2 is rank-0 → doc2 wins.
    assert_eq!(fused_biased[0].0, 2, "biased toward branch 1 → doc2 wins");
}
