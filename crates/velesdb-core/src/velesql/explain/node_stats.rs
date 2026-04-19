//! Per-node execution statistics helpers for EXPLAIN ANALYZE.
//!
//! Extracted from `explain.rs` for maintainability (module splitting).
//! Builds heuristic per-node stats by distributing total execution time
//! across leaf nodes using normalized weights.

use super::{NodeStats, PlanNode};
use crate::collection::stats::CollectionStats as CoreCollectionStats;
use crate::velesql::cost_estimator::CostEstimator;

/// Scaling factor converting `CostEstimator` arbitrary units to milliseconds.
///
/// Picked so that a default-factored plan on a 1K-row collection produces a
/// cost in the same order of magnitude as the legacy heuristic. The constant
/// is calibratable via a future micro-bench (TODO(EPIC-046): measure and pin
/// this empirically).
const COST_UNIT_TO_MS: f64 = 0.001;

/// Estimates selectivity (placeholder - would need statistics in production).
pub(super) fn estimate_selectivity(
    conditions: &[String],
    _stats: Option<&CoreCollectionStats>,
) -> f64 {
    // For now `stats` is ignored — the plan-builder passes raw condition
    // strings, so histogram-based selectivity is computed upstream in
    // `plan_builder::append_filter_nodes` when stats are available. This
    // function remains a string-level fallback.
    estimate_selectivity_heuristic(conditions)
}

/// Heuristic fallback: more conditions = lower selectivity.
pub(super) fn estimate_selectivity_heuristic(conditions: &[String]) -> f64 {
    let base = 0.5_f64;
    base.powi(i32::try_from(conditions.len()).unwrap_or(i32::MAX))
}

/// Estimates execution cost in milliseconds for the entire plan.
///
/// When `stats` is `Some`, uses the calibrated `CostEstimator` pipeline.
/// When `stats` is `None`, falls back to the historical heuristic formula
/// bit-for-bit (backward compatibility with ~50 existing EXPLAIN tests).
pub(super) fn estimate_cost(
    root: &PlanNode,
    has_vector_search: bool,
    stats: Option<&CoreCollectionStats>,
) -> f64 {
    match stats {
        Some(s) => {
            let cost = CostEstimator::new(s).estimate_plan_cost(root);
            let ms = cost.total() * COST_UNIT_TO_MS;
            // Guard against pathological zero costs for empty plans — keep a
            // small positive floor so downstream asserts (> 0.0) still hold.
            if ms > 0.0 {
                ms
            } else {
                estimate_cost_heuristic(root, has_vector_search)
            }
        }
        None => estimate_cost_heuristic(root, has_vector_search),
    }
}

/// Historical heuristic cost formula — kept unchanged for backward compat.
pub(super) fn estimate_cost_heuristic(root: &PlanNode, has_vector_search: bool) -> f64 {
    let base_cost = if has_vector_search { 0.05 } else { 1.0 };

    match root {
        PlanNode::Sequence(nodes) => nodes.iter().fold(base_cost, |acc, n| acc + node_cost(n)),
        _ => base_cost + node_cost(root),
    }
}

/// Returns the heuristic cost for a single plan node.
pub(super) fn node_cost(node: &PlanNode) -> f64 {
    match node {
        PlanNode::VectorSearch(_) => 0.05,
        PlanNode::Filter(f) => 0.01 * (1.0 - f.selectivity),
        PlanNode::Limit(_) | PlanNode::Offset(_) => 0.001,
        PlanNode::TableScan(_) => 1.0,
        PlanNode::IndexLookup(_) => 0.0001, // O(1) lookup is very fast
        PlanNode::Sequence(nodes) => nodes.iter().map(node_cost).sum(),
        PlanNode::MatchTraversal(mt) => {
            // Cost depends on depth and strategy
            let base = 0.1;
            let depth_factor = f64::from(mt.max_depth) * 0.05;
            let similarity_factor = if mt.has_similarity { 0.05 } else { 0.0 };
            base + depth_factor + similarity_factor
        }
    }
}

/// Returns the label and a relative time weight for a plan node.
///
/// Weights are unitless and will be **normalized** so that all leaf nodes
/// in a plan sum to exactly `total_time_ms`.  This avoids the previous bug
/// where fixed fractions could exceed 100 % when multiple heavy nodes
/// (e.g. `VectorSearch` + `TableScan`) appeared in the same plan.
fn node_label_and_weight(node: &PlanNode) -> (&'static str, f64) {
    match node {
        PlanNode::VectorSearch(_) => ("VectorSearch", 0.95),
        PlanNode::TableScan(_) => ("TableScan", 0.95),
        PlanNode::MatchTraversal(_) => ("MatchTraversal", 0.95),
        PlanNode::IndexLookup(_) => ("IndexLookup", 0.90),
        PlanNode::Filter(_) => ("Filter", 0.03),
        PlanNode::Limit(_) => ("Limit", 0.01),
        PlanNode::Offset(_) => ("Offset", 0.01),
        PlanNode::Sequence(_) => ("Sequence", 0.0),
    }
}

/// Estimates the number of rows entering a node given the number leaving it,
/// working *backwards* through the pipeline so that the last node's
/// `rows_out` equals `actual_rows`.
fn estimate_rows_in(node: &PlanNode, rows_out: u64) -> u64 {
    match node {
        PlanNode::Filter(f) => {
            if f.selectivity > 0.0 && f.selectivity < 1.0 {
                // Invert selectivity: if 50 % of rows pass, twice as many entered.
                // Reason: rows_out and selectivity are bounded positive values;
                // precision loss and truncation are acceptable for estimation.
                #[allow(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss
                )]
                let estimated = (rows_out as f64 / f.selectivity).ceil() as u64;
                estimated.max(rows_out)
            } else {
                rows_out
            }
        }
        PlanNode::Offset(o) => {
            // Offset skips `count` rows, so more rows entered than left.
            rows_out.saturating_add(o.count)
        }
        // Limit / scan / search: we cannot tell how many rows were available
        // beyond what was returned, so rows_in = rows_out (conservative).
        _ => rows_out,
    }
}

/// Intermediate representation used while building stats.
struct LeafEntry<'a> {
    label: &'static str,
    weight: f64,
    node: &'a PlanNode,
}

/// Recursively collects leaf-node metadata in pipeline order.
fn collect_leaves<'a>(node: &'a PlanNode, out: &mut Vec<LeafEntry<'a>>) {
    if let PlanNode::Sequence(children) = node {
        for child in children {
            collect_leaves(child, out);
        }
    } else {
        let (label, weight) = node_label_and_weight(node);
        out.push(LeafEntry {
            label,
            weight,
            node,
        });
    }
}

/// Builds per-node stats for all leaf nodes in the plan tree.
///
/// 1. Collects leaf nodes in pipeline order.
/// 2. **Normalizes** time weights so they always sum to `total_time_ms`.
/// 3. Propagates `actual_rows` **backwards** through the pipeline so that
///    `Filter` and `Offset` nodes show realistic `rows_in` / `rows_out`
///    estimates instead of blindly copying the top-level row count.
#[must_use]
pub fn build_leaf_node_stats(
    root: &PlanNode,
    actual_rows: u64,
    total_time_ms: f64,
) -> Vec<NodeStats> {
    let mut leaves = Vec::new();
    collect_leaves(root, &mut leaves);

    if leaves.is_empty() {
        return Vec::new();
    }

    let total_weight: f64 = leaves.iter().map(|l| l.weight).sum();

    // --- Row propagation (reverse pass) ---
    // The last node's rows_out equals the top-level actual_rows.
    // Each preceding node's rows_out is the next node's rows_in.
    let len = leaves.len();
    let mut rows_in = vec![actual_rows; len];
    let mut rows_out = vec![actual_rows; len];

    rows_out[len - 1] = actual_rows;
    rows_in[len - 1] = estimate_rows_in(leaves[len - 1].node, actual_rows);

    for i in (0..len - 1).rev() {
        rows_out[i] = rows_in[i + 1];
        rows_in[i] = estimate_rows_in(leaves[i].node, rows_out[i]);
    }

    leaves
        .iter()
        .enumerate()
        .map(|(i, leaf)| {
            let time_ms = if total_weight > 0.0 {
                total_time_ms * leaf.weight / total_weight
            } else {
                0.0
            };
            NodeStats {
                node_label: leaf.label.to_string(),
                actual_time_ms: time_ms,
                actual_rows_in: rows_in[i],
                actual_rows_out: rows_out[i],
                loops: 1,
                estimated: true,
            }
        })
        .collect()
}
