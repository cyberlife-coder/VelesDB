//! Generic recursive helpers for walking VelesQL condition trees.
//!
//! Eliminates per-module duplication of the `And/Or -> recurse, Group/Not -> recurse, _ -> base`
//! pattern that appears across validation, extraction, where_eval, and hybrid_sparse modules.

use crate::velesql::Condition;

/// Returns `true` if any subtree of `condition` satisfies `predicate`.
///
/// Walks `And`, `Or`, `Group`, and `Not` combinators recursively.
pub(crate) fn any_subtree(condition: &Condition, predicate: &dyn Fn(&Condition) -> bool) -> bool {
    if predicate(condition) {
        return true;
    }
    match condition {
        Condition::And(left, right) | Condition::Or(left, right) => {
            any_subtree(left, predicate) || any_subtree(right, predicate)
        }
        Condition::Group(inner) | Condition::Not(inner) => any_subtree(inner, predicate),
        _ => false,
    }
}

/// Recursively counts leaves matching `predicate`.
pub(crate) fn count_matching_leaves(
    condition: &Condition,
    predicate: fn(&Condition) -> bool,
) -> usize {
    if predicate(condition) {
        return 1;
    }
    match condition {
        Condition::And(left, right) | Condition::Or(left, right) => {
            count_matching_leaves(left, predicate) + count_matching_leaves(right, predicate)
        }
        Condition::Group(inner) | Condition::Not(inner) => count_matching_leaves(inner, predicate),
        _ => 0,
    }
}

/// Returns `true` if the condition is a vector-type leaf
/// (`Similarity`, `VectorSearch`, `VectorFusedSearch`).
pub(crate) fn is_vector_leaf(condition: &Condition) -> bool {
    matches!(
        condition,
        Condition::Similarity(_) | Condition::VectorSearch(_) | Condition::VectorFusedSearch(_)
    )
}

#[cfg(test)]
#[path = "condition_tree_tests.rs"]
mod tests;
