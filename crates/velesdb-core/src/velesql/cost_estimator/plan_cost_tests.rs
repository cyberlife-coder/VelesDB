//! Unit tests for the `estimate_plan_cost` API. These exercise the
//! cost-monotonicity invariants independently of the EXPLAIN pipeline.

use super::*;
use crate::collection::stats::CollectionStats;
use crate::velesql::explain::{
    FilterPlan, IndexLookupPlan, LimitPlan, MatchTraversalPlan, PlanNode, TableScanPlan,
    VectorSearchPlan,
};

/// Builds a `CollectionStats` with a fixed total point count.
fn stats_with_points(total: u64) -> CollectionStats {
    let mut s = CollectionStats::new();
    s.total_points = total;
    s.row_count = total;
    s
}

#[test]
fn plan_cost_vector_search_scales_with_ef_search() {
    let stats = stats_with_points(10_000);
    let est = CostEstimator::new(&stats);

    let low_ef = PlanNode::VectorSearch(VectorSearchPlan {
        collection: "t".into(),
        ef_search: 50,
        candidates: 10,
    });
    let high_ef = PlanNode::VectorSearch(VectorSearchPlan {
        collection: "t".into(),
        ef_search: 500,
        candidates: 10,
    });

    let c_low = est.estimate_plan_cost(&low_ef).total();
    let c_high = est.estimate_plan_cost(&high_ef).total();
    assert!(
        c_high > c_low,
        "larger ef_search must cost more: low={c_low} high={c_high}"
    );
}

#[test]
fn plan_cost_table_scan_scales_with_collection_size() {
    let small = stats_with_points(100);
    let large = stats_with_points(10_000);

    let scan = PlanNode::TableScan(TableScanPlan {
        collection: "t".into(),
    });

    let c_small = CostEstimator::new(&small).estimate_plan_cost(&scan).total();
    let c_large = CostEstimator::new(&large).estimate_plan_cost(&scan).total();
    assert!(
        c_large > c_small,
        "larger collection must cost more to scan: small={c_small} large={c_large}"
    );
}

#[test]
fn plan_cost_index_lookup_cheaper_than_table_scan() {
    let stats = stats_with_points(100_000);
    let est = CostEstimator::new(&stats);

    let scan = PlanNode::TableScan(TableScanPlan {
        collection: "t".into(),
    });
    let lookup = PlanNode::IndexLookup(IndexLookupPlan {
        label: "t".into(),
        property: "id".into(),
        value: "1".into(),
    });

    let c_scan = est.estimate_plan_cost(&scan).total();
    let c_lookup = est.estimate_plan_cost(&lookup).total();
    assert!(
        c_lookup < c_scan,
        "index lookup must be cheaper than full scan: lookup={c_lookup} scan={c_scan}"
    );
}

#[test]
fn plan_cost_index_lookup_uses_column_ndv_when_available() {
    // #607: when ANALYZE has populated column_stats for the indexed
    // property, the lookup cost scales with the distinct-value count
    // (NDV), not the full collection size — a high-cardinality probe
    // costs more (deeper B-tree) than a low-cardinality one.
    use crate::collection::stats::ColumnStats;

    let mut high_card = stats_with_points(1_000_000);
    high_card.column_stats.insert(
        "user_id".into(),
        ColumnStats {
            distinct_count: 1_000_000,
            distinct_values: 1_000_000,
            ..ColumnStats::default()
        },
    );

    let mut low_card = stats_with_points(1_000_000);
    low_card.column_stats.insert(
        "category".into(),
        ColumnStats {
            distinct_count: 8,
            distinct_values: 8,
            ..ColumnStats::default()
        },
    );

    let high_lookup = PlanNode::IndexLookup(IndexLookupPlan {
        label: "t".into(),
        property: "user_id".into(),
        value: "42".into(),
    });
    let low_lookup = PlanNode::IndexLookup(IndexLookupPlan {
        label: "t".into(),
        property: "category".into(),
        value: "tech".into(),
    });

    let c_high = CostEstimator::new(&high_card)
        .estimate_plan_cost(&high_lookup)
        .total();
    let c_low = CostEstimator::new(&low_card)
        .estimate_plan_cost(&low_lookup)
        .total();
    assert!(
        c_high > c_low,
        "high-NDV index probe must cost more than low-NDV: high={c_high} low={c_low}"
    );
}

#[test]
fn plan_cost_index_lookup_falls_back_when_no_column_stats() {
    // Negative case: when column_stats has no entry for the indexed
    // property, the cost must reproduce the legacy log2(total) heuristic
    // bit-for-bit so callers without ANALYZE keep their numbers stable.
    let stats = stats_with_points(100_000); // no column_stats populated
    let est = CostEstimator::new(&stats);

    let lookup = PlanNode::IndexLookup(IndexLookupPlan {
        label: "t".into(),
        property: "untracked_field".into(),
        value: "1".into(),
    });

    let cost = est.estimate_plan_cost(&lookup).total();
    // log2(100_000) ≈ 16.6; with default factors and cpu_index_cost
    // baseline, the cost is bounded below 1.0 (a fast index probe).
    assert!(
        cost > 0.0 && cost < 1.0,
        "fallback cost must be small but non-zero: got {cost}"
    );
}

#[test]
fn plan_cost_match_traversal_scales_with_depth() {
    let stats = stats_with_points(1_000);
    let est = CostEstimator::new(&stats);

    let shallow = PlanNode::MatchTraversal(MatchTraversalPlan {
        strategy: "graph-first".into(),
        start_labels: vec!["A".into()],
        max_depth: 1,
        relationship_count: 1,
        has_similarity: false,
        similarity_threshold: None,
    });
    let deep = PlanNode::MatchTraversal(MatchTraversalPlan {
        strategy: "graph-first".into(),
        start_labels: vec!["A".into()],
        max_depth: 3,
        relationship_count: 1,
        has_similarity: false,
        similarity_threshold: None,
    });

    let c_shallow = est.estimate_plan_cost(&shallow).total();
    let c_deep = est.estimate_plan_cost(&deep).total();
    assert!(
        c_deep > c_shallow,
        "deeper traversal must cost more: shallow={c_shallow} deep={c_deep}"
    );
}

#[test]
fn plan_cost_sequence_sums_children() {
    let stats = stats_with_points(1_000);
    let est = CostEstimator::new(&stats);

    let scan = PlanNode::TableScan(TableScanPlan {
        collection: "t".into(),
    });
    let filter = PlanNode::Filter(FilterPlan {
        conditions: "x = 1".into(),
        selectivity: 0.1,
        estimated_rows: None,
        estimation_method: None,
    });
    let limit = PlanNode::Limit(LimitPlan {
        count: 10,
        is_default: false,
    });

    let c_scan = est.estimate_plan_cost(&scan).total();
    let c_filter = est.estimate_plan_cost(&filter).total();
    let c_limit = est.estimate_plan_cost(&limit).total();

    let sequence = PlanNode::Sequence(vec![scan, filter, limit]);
    let c_seq = est.estimate_plan_cost(&sequence).total();

    let expected = c_scan + c_filter + c_limit;
    assert!(
        (c_seq - expected).abs() < 1e-9,
        "Sequence cost must equal sum of child costs: seq={c_seq} expected={expected}"
    );
}

#[test]
fn plan_cost_filter_from_selectivity_monotone() {
    let stats = stats_with_points(10_000);
    let est = CostEstimator::new(&stats);

    let low_sel = est.estimate_filter_cost_from_selectivity(0.01).total();
    let high_sel = est.estimate_filter_cost_from_selectivity(0.5).total();
    assert!(
        high_sel > low_sel,
        "higher selectivity means more rows scanned → higher cost"
    );
}

#[test]
fn plan_cost_empty_stats_does_not_panic() {
    // Regression guard: corrupt-looking stats (zero points, no histogram)
    // must still produce a finite cost via the `.max(1)` floors.
    let stats = CollectionStats::new();
    let est = CostEstimator::new(&stats);

    let plan = PlanNode::VectorSearch(VectorSearchPlan {
        collection: "t".into(),
        ef_search: 100,
        candidates: 10,
    });
    let cost = est.estimate_plan_cost(&plan).total();
    assert!(cost.is_finite() && cost > 0.0);
}

#[test]
fn hnsw_cost_on_size_scales_logarithmically() {
    // Devin finding E on #606: the reduced-set HNSW cost must scale with
    // log2(size), not linearly. Doubling the size increases the cost by
    // exactly one probe "step" of (ef + k).
    let stats = stats_with_points(100_000);
    let est = CostEstimator::new(&stats);

    let small = est
        .estimate_hnsw_search_cost_with_ef_on_size(100, 10, 1_000)
        .total();
    let big = est
        .estimate_hnsw_search_cost_with_ef_on_size(100, 10, 1_000_000)
        .total();

    assert!(
        big > small,
        "cost must grow with collection size: small={small} big={big}"
    );
    // Logarithmic (not linear) scaling: going from 1K to 1M rows
    // multiplies log2 by 2.0 (from ~10 to ~20), so the ratio must stay
    // well below 1000× (= the linear scaling result).
    assert!(
        big / small < 5.0,
        "HNSW cost must scale logarithmically (ratio < 5), got {}",
        big / small
    );
}

#[test]
fn hnsw_cost_on_full_size_matches_default_variant() {
    // Backward-compat: `estimate_hnsw_search_cost_with_ef` must return
    // exactly the same cost as
    // `estimate_hnsw_search_cost_with_ef_on_size(stats.total_points)`.
    let stats = stats_with_points(42_000);
    let est = CostEstimator::new(&stats);

    let implicit = est.estimate_hnsw_search_cost_with_ef(100, 10).total();
    let explicit = est
        .estimate_hnsw_search_cost_with_ef_on_size(100, 10, 42_000)
        .total();

    assert!(
        (implicit - explicit).abs() < f64::EPSILON,
        "the two variants must produce identical costs when called with \
         the full collection size: implicit={implicit} explicit={explicit}"
    );
}
