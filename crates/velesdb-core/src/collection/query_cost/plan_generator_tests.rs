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

    let scan = plans
        .iter()
        .find(|p| matches!(p.plan, PhysicalPlan::SeqScan { .. }))
        .expect("scan plan must always be generated");
    // estimate_scan reports live rows (100_000 - 0 deleted) and the SeqScan
    // variant copies that into estimated_rows.
    assert_eq!(scan.cost.rows, 100_000);
    if let PhysicalPlan::SeqScan { estimated_rows, .. } = scan.plan {
        assert_eq!(estimated_rows, 100_000);
    }
    // io_cost + cpu_cost with positive default factors is strictly positive.
    assert!(scan.cost.total > 0.0);
    assert!(scan.cost.startup.abs() < f64::EPSILON);
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

    // Use input values that differ from the production defaults
    // (unwrap_or(10)/unwrap_or(100)) so the assertion proves real pass-through,
    // not a coincidence with the defaults.
    let query = QueryCharacteristics {
        collection: "test".to_string(),
        has_similarity: true,
        top_k: Some(7),
        ef_search: Some(64),
        ..Default::default()
    };

    let plans = generator.generate_plans(&query, &stats);

    let vp = plans
        .iter()
        .find(|p| matches!(p.plan, PhysicalPlan::VectorSearch { .. }))
        .expect("vector plan present");
    if let PhysicalPlan::VectorSearch {
        k,
        ef_search,
        collection,
    } = &vp.plan
    {
        assert_eq!(*k, 7);
        assert_eq!(*ef_search, 64);
        assert_eq!(collection, "test");
    } else {
        unreachable!();
    }
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

    let best = generator
        .optimize(&query, &stats)
        .expect("optimize always returns a plan (scan baseline)");
    // With this fixture the low-cardinality prop_category index (50 entries, depth 3)
    // wins: cost = depth*random_page_cost = 12, beating VectorSearch (~166) and
    // SeqScan+filter (~5125). Pin the deterministic winner so a regression in the
    // cost model or plan selection is caught.
    assert!(
        matches!(best.plan, PhysicalPlan::IndexScan { .. }),
        "IndexScan should win for selective-filter + similarity, got {:?}",
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
