//! Query validation for VelesQL similarity queries.
//!
//! Validates that similarity() queries don't use unsupported patterns:
//! - Multiple similarity() in OR (requires union of results)
//! - similarity() in OR with non-similarity conditions
//! - NOT similarity() patterns
//!
//! # Supported Patterns (EPIC-044 US-001)
//!
//! Multiple similarity() with AND is supported:
//! ```sql
//! WHERE similarity(v, $v1) > 0.8 AND similarity(v, $v2) > 0.7
//! ```
//! This applies filters sequentially (cascade).

use crate::collection::types::Collection;
use crate::error::Result;
use crate::velesql::Condition;

use super::condition_tree::{any_subtree, count_matching_leaves, is_vector_leaf};

impl Collection {
    /// Refuses the vector-predicate shapes no executor can run as written
    /// ([`crate::velesql::validate_vector_predicates`], shared with the WASM
    /// executor).
    pub(crate) fn validate_similarity_query_structure(condition: &Condition) -> Result<()> {
        crate::velesql::validate_vector_predicates(condition)
    }

    /// Check if similarity() appears under a NOT condition.
    /// This pattern is not supported because negating similarity cannot be efficiently executed.
    ///
    /// # Note
    ///
    /// This function is prepared for when VelesQL parser supports `NOT condition` syntax.
    /// Currently, the parser only supports `IS NOT NULL` and `!=` operators.
    /// When parser is extended (see EPIC-005), this validation will activate.
    pub(crate) fn has_similarity_under_not(condition: &Condition) -> bool {
        any_subtree(
            condition,
            &|c| matches!(c, Condition::Not(inner) if Self::count_similarity_conditions(inner) > 0),
        )
    }

    /// Count the number of vector search conditions in a condition tree.
    /// Includes Similarity, VectorSearch (NEAR), and VectorFusedSearch (NEAR_FUSED).
    pub(crate) fn count_similarity_conditions(condition: &Condition) -> usize {
        count_matching_leaves(condition, is_vector_leaf)
    }

    /// Check if a `similarity()` threshold appears in an OR clause with
    /// non-similarity conditions — the shape the union executor
    /// (`union_query.rs`) runs.
    ///
    /// Only a `similarity()` leaf qualifies. A `NEAR` leaf is TRUE in a
    /// boolean context (`where_eval.rs` evaluates `VectorSearch` to `true`)
    /// and ranks the rows, so `NEAR OR <metadata>` is the NEAR search over the
    /// WHERE with that leaf read as TRUE — which the regular NEAR dispatch
    /// runs, since [`extract_vector_search`](Self::extract_vector_search)
    /// finds the leaf under the OR and
    /// [`extract_metadata_filter`](Self::extract_metadata_filter) drops an OR
    /// that has a NEAR side. The union executor has no NEAR leg: routed there,
    /// the shape lost every NEAR hit and scored the metadata rows 1.0.
    pub(crate) fn has_similarity_in_problematic_or(condition: &Condition) -> bool {
        any_subtree(condition, &|c| {
            if let Condition::Or(left, right) = c {
                let left_has_sim = has_similarity_threshold(left);
                let right_has_sim = has_similarity_threshold(right);
                let left_has_other = Self::has_non_similarity_conditions(left);
                let right_has_other = Self::has_non_similarity_conditions(right);
                // Problematic: one side has similarity, other side has non-similarity
                // e.g., similarity() > 0.8 OR category = 'tech'
                (left_has_sim && right_has_other && !right_has_sim)
                    || (right_has_sim && left_has_other && !left_has_sim)
            } else {
                false
            }
        })
    }

    /// Check if a condition contains non-similarity conditions (metadata filters).
    pub(crate) fn has_non_similarity_conditions(condition: &Condition) -> bool {
        // A non-similarity leaf is any leaf that is NOT a vector type and NOT a
        // structural combinator (And/Or/Group/Not).
        count_matching_leaves(condition, |c| {
            !is_vector_leaf(c)
                && !matches!(
                    c,
                    Condition::And(..)
                        | Condition::Or(..)
                        | Condition::Group(_)
                        | Condition::Not(_)
                )
        }) > 0
    }
}

/// True if the tree holds a `similarity()` threshold leaf.
fn has_similarity_threshold(condition: &Condition) -> bool {
    count_matching_leaves(condition, |c| matches!(c, Condition::Similarity(_))) > 0
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
