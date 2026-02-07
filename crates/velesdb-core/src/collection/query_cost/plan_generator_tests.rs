//! Tests for plan generator (query optimization).
//!
//! Extracted from `plan_generator.rs` for maintainability (04-06 module splitting).

use super::plan_generator::*;
use crate::collection::stats::{CollectionStats, IndexStats};

fn test_stats() -> CollectionStats {
    let mut stats = CollectionStats::with_counts(100_000, 0);
    stats.total_size_bytes = 100_000 * 256;
    stats.index_stats.insert(
        "hnsw_primary".to_string(),
        IndexStats::new("hnsw_primary", "HNSW").with_entry_count(100_000),
    );
    stats.index_stats.insert(
        "prop_category".to_string(),
        IndexStats::new("prop_category", "PropertyIndex")
            .with_entry_count(50)
            .with_depth(3),
    );
    stats
}

#[test]
fn test_generate_scan_plan() {
    let generator = PlanGenerator::default();
    let stats = test_stats();

    let query = QueryCharacteristics {
        collection: "test".to_string(),
        ..Default::default()
    };

    let plans = generator.generate_plans(&query, &stats);

    assert!(!plans.is_empty());
    assert!(plans
        .iter()
        .any(|p| matches!(p.plan, PhysicalPlan::SeqScan { .. })));
}

#[test]
fn test_generate_index_plan() {
    let generator = PlanGenerator::default();
    let stats = test_stats();

    let query = QueryCharacteristics {
        collection: "test".to_string(),
        has_filter: true,
        filter_selectivity: Some(0.01),
        ..Default::default()
    };

    let plans = generator.generate_plans(&query, &stats);

    assert!(plans
        .iter()
        .any(|p| matches!(p.plan, PhysicalPlan::IndexScan { .. })));
}

#[test]
fn test_generate_vector_plan() {
    let generator = PlanGenerator::default();
    let stats = test_stats();

    let query = QueryCharacteristics {
        collection: "test".to_string(),
        has_similarity: true,
        top_k: Some(10),
        ef_search: Some(100),
        ..Default::default()
    };

    let plans = generator.generate_plans(&query, &stats);

    assert!(plans
        .iter()
        .any(|p| matches!(p.plan, PhysicalPlan::VectorSearch { .. })));
}

#[test]
fn test_generate_hybrid_plans() {
    let generator = PlanGenerator::default();
    let stats = test_stats();

    let query = QueryCharacteristics {
        collection: "test".to_string(),
        has_similarity: true,
        has_match: true,
        top_k: Some(10),
        max_depth: Some(2),
        ..Default::default()
    };

    let plans = generator.generate_plans(&query, &stats);

    // Should have scan + vector + graph + 2 hybrid strategies
    assert!(plans.len() >= 4);
}

#[test]
fn test_select_best_plan() {
    let generator = PlanGenerator::default();
    let stats = test_stats();

    let query = QueryCharacteristics {
        collection: "test".to_string(),
        has_similarity: true,
        has_filter: true,
        filter_selectivity: Some(0.01),
        top_k: Some(10),
        ..Default::default()
    };

    let best = generator.optimize(&query, &stats);

    assert!(best.is_some());
    let best = best.unwrap();
    // Vector search should typically win for similarity queries
    assert!(
        matches!(
            best.plan,
            PhysicalPlan::VectorSearch { .. } | PhysicalPlan::IndexScan { .. }
        ),
        "Expected VectorSearch or IndexScan, got {:?}",
        best.plan.plan_type()
    );
}

#[test]
fn test_cost_ordering() {
    let generator = PlanGenerator::default();
    let stats = test_stats();

    let query = QueryCharacteristics {
        collection: "test".to_string(),
        has_filter: true,
        filter_selectivity: Some(0.001), // Very selective
        ..Default::default()
    };

    let plans = generator.generate_plans(&query, &stats);

    // Find scan and index plans
    let scan = plans
        .iter()
        .find(|p| matches!(p.plan, PhysicalPlan::SeqScan { .. }));
    let index = plans
        .iter()
        .find(|p| matches!(p.plan, PhysicalPlan::IndexScan { .. }));

    if let (Some(scan), Some(index)) = (scan, index) {
        assert!(
            index.cost.total < scan.cost.total,
            "Index should be cheaper for selective query"
        );
    }
}
