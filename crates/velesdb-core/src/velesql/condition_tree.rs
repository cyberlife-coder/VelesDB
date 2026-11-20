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

/// `condition` with the column of each leaf that names one replaced by what
/// `map` answers for it, kept where it answers `None`; its `AND`, `OR`,
/// `NOT` and groups as they are, and every leaf without a column (a vector
/// search, `similarity()`, a graph pattern) unchanged.
pub(crate) fn map_leaf_columns(
    condition: &Condition,
    map: &dyn Fn(&str) -> Option<String>,
) -> Condition {
    let operand = |c: &Condition| Box::new(map_leaf_columns(c, map));
    match condition {
        Condition::And(left, right) => Condition::And(operand(left), operand(right)),
        Condition::Or(left, right) => Condition::Or(operand(left), operand(right)),
        Condition::Not(inner) => Condition::Not(operand(inner)),
        Condition::Group(inner) => Condition::Group(operand(inner)),
        leaf => {
            let mut leaf = leaf.clone();
            if let Some(column) = leaf_column_mut(&mut leaf) {
                if let Some(mapped) = map(column) {
                    *column = mapped;
                }
            }
            leaf
        }
    }
}

/// The column a leaf condition reads, if it names one.
fn leaf_column_mut(leaf: &mut Condition) -> Option<&mut String> {
    use crate::velesql::{
        BetweenCondition, Comparison, ContainsCondition, ContainsTextCondition, GeoBboxCondition,
        GeoDistanceCondition, InCondition, IsNullCondition, LikeCondition, MatchCondition,
    };
    match leaf {
        Condition::Comparison(Comparison { column, .. })
        | Condition::In(InCondition { column, .. })
        | Condition::Between(BetweenCondition { column, .. })
        | Condition::Like(LikeCondition { column, .. })
        | Condition::IsNull(IsNullCondition { column, .. })
        | Condition::Match(MatchCondition { column, .. })
        | Condition::Contains(ContainsCondition { column, .. })
        | Condition::ContainsText(ContainsTextCondition { column, .. })
        | Condition::GeoDistance(GeoDistanceCondition { column, .. })
        | Condition::GeoBbox(GeoBboxCondition { column, .. }) => Some(column),
        _ => None,
    }
}

#[cfg(test)]
#[path = "condition_tree_tests.rs"]
mod tests;
