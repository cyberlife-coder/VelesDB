//! Tests for `FusionStrategy` implementations.

use super::strategy::FusionStrategy;

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
