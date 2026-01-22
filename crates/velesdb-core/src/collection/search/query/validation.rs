//! Query validation for VelesQL similarity queries.
//!
//! Validates that similarity() queries don't use unsupported patterns:
//! - Multiple similarity() conditions
//! - similarity() in OR with non-similarity conditions
//! - NOT similarity() patterns

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::velesql::Condition;

impl Collection {
    /// Validate that similarity() queries don't use unsupported patterns.
    ///
    /// # Unsupported Patterns (BUG-4, BUG-5)
    ///
    /// 1. **similarity() in OR with non-similarity conditions** (BUG-4):
    ///    `WHERE similarity(v, $v) > 0.8 OR category = 'tech'`
    ///    This would require executing both a vector search AND a metadata scan,
    ///    then unioning results - not currently supported.
    ///
    /// 2. **Multiple similarity() conditions** (BUG-5):
    ///    `WHERE similarity(v, $v1) > 0.8 AND similarity(v, $v2) > 0.7`
    ///    Only one vector search can be executed per query.
    ///
    /// Returns Ok(()) if the query structure is valid, or an error describing the issue.
    pub(crate) fn validate_similarity_query_structure(condition: &Condition) -> Result<()> {
        let similarity_count = Self::count_similarity_conditions(condition);

        // BUG-5: Multiple similarity() conditions not supported
        if similarity_count > 1 {
            return Err(Error::Config(
                "Multiple similarity() conditions in a single query are not supported. \
                Use a single similarity() condition per query."
                    .to_string(),
            ));
        }

        // BUG-4: similarity() in OR with non-similarity conditions
        if similarity_count == 1 && Self::has_similarity_in_problematic_or(condition) {
            return Err(Error::Config(
                "similarity() in OR with non-vector conditions is not supported. \
                Use AND instead, or split into separate queries."
                    .to_string(),
            ));
        }

        // BUG FIX: NOT similarity() is not supported
        // Semantically this would mean "exclude similar items" which cannot be
        // efficiently executed with ANN indexes (would require full scan + exclusion)
        if similarity_count >= 1 && Self::has_similarity_under_not(condition) {
            return Err(Error::Config(
                "NOT similarity() is not supported. Negating similarity conditions \
                cannot be efficiently executed. Consider using a threshold filter instead."
                    .to_string(),
            ));
        }

        Ok(())
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
        match condition {
            Condition::Not(inner) => {
                // If there's any similarity inside NOT, it's unsupported
                Self::count_similarity_conditions(inner) > 0
            }
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::has_similarity_under_not(left) || Self::has_similarity_under_not(right)
            }
            Condition::Group(inner) => Self::has_similarity_under_not(inner),
            _ => false,
        }
    }

    /// Count the number of similarity() conditions in a condition tree.
    pub(crate) fn count_similarity_conditions(condition: &Condition) -> usize {
        match condition {
            Condition::Similarity(_) => 1,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::count_similarity_conditions(left) + Self::count_similarity_conditions(right)
            }
            // BUG FIX: Handle Condition::Not to find similarity inside NOT clauses
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::count_similarity_conditions(inner)
            }
            _ => 0,
        }
    }

    /// Check if similarity() appears in an OR clause with non-similarity conditions.
    /// This pattern cannot be correctly executed with current architecture.
    pub(crate) fn has_similarity_in_problematic_or(condition: &Condition) -> bool {
        match condition {
            Condition::Or(left, right) => {
                let left_has_sim = Self::count_similarity_conditions(left) > 0;
                let right_has_sim = Self::count_similarity_conditions(right) > 0;
                let left_has_other = Self::has_non_similarity_conditions(left);
                let right_has_other = Self::has_non_similarity_conditions(right);

                // Problematic: one side has similarity, other side has non-similarity
                // e.g., similarity() > 0.8 OR category = 'tech'
                (left_has_sim && right_has_other && !right_has_sim)
                    || (right_has_sim && left_has_other && !left_has_sim)
                    // Also check children recursively
                    || Self::has_similarity_in_problematic_or(left)
                    || Self::has_similarity_in_problematic_or(right)
            }
            Condition::And(left, right) => {
                // AND is fine, but check children for nested ORs
                Self::has_similarity_in_problematic_or(left)
                    || Self::has_similarity_in_problematic_or(right)
            }
            // BUG FIX: Handle Condition::Not to check nested ORs inside NOT clauses
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::has_similarity_in_problematic_or(inner)
            }
            _ => false,
        }
    }

    /// Check if a condition contains non-similarity conditions (metadata filters).
    pub(crate) fn has_non_similarity_conditions(condition: &Condition) -> bool {
        match condition {
            Condition::Similarity(_)
            | Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_) => false,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::has_non_similarity_conditions(left)
                    || Self::has_non_similarity_conditions(right)
            }
            // BUG FIX: Handle Condition::Not - NOT wraps another condition
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::has_non_similarity_conditions(inner)
            }
            // All other conditions (Compare, In, Between, Match, etc.) are non-similarity
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql::{CompareOp, Comparison, SimilarityCondition, Value, VectorExpr};

    fn make_similarity_condition() -> Condition {
        Condition::Similarity(SimilarityCondition {
            field: "vector".to_string(),
            vector: VectorExpr::Literal(vec![0.1, 0.2, 0.3]),
            operator: CompareOp::Gt,
            threshold: 0.8,
        })
    }

    fn make_compare_condition() -> Condition {
        Condition::Comparison(Comparison {
            column: "category".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("tech".to_string()),
        })
    }

    #[test]
    fn test_validate_single_similarity_and_metadata_ok() {
        // similarity() AND category = 'tech' - should be OK
        let cond = Condition::And(
            Box::new(make_similarity_condition()),
            Box::new(make_compare_condition()),
        );
        assert!(Collection::validate_similarity_query_structure(&cond).is_ok());
    }

    #[test]
    fn test_validate_similarity_or_metadata_fails() {
        // similarity() OR category = 'tech' - should FAIL (BUG-4)
        let cond = Condition::Or(
            Box::new(make_similarity_condition()),
            Box::new(make_compare_condition()),
        );
        let result = Collection::validate_similarity_query_structure(&cond);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("OR"));
    }

    #[test]
    fn test_validate_multiple_similarity_fails() {
        // similarity() AND similarity() - should FAIL (BUG-5)
        let cond = Condition::And(
            Box::new(make_similarity_condition()),
            Box::new(make_similarity_condition()),
        );
        let result = Collection::validate_similarity_query_structure(&cond);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Multiple"));
    }

    #[test]
    fn test_validate_metadata_only_ok() {
        // category = 'tech' AND status = 'active' - should be OK
        let cond = Condition::And(
            Box::new(make_compare_condition()),
            Box::new(make_compare_condition()),
        );
        assert!(Collection::validate_similarity_query_structure(&cond).is_ok());
    }

    #[test]
    fn test_validate_metadata_or_ok() {
        // category = 'tech' OR status = 'active' - should be OK (no similarity)
        let cond = Condition::Or(
            Box::new(make_compare_condition()),
            Box::new(make_compare_condition()),
        );
        assert!(Collection::validate_similarity_query_structure(&cond).is_ok());
    }

    #[test]
    fn test_count_similarity_conditions() {
        assert_eq!(
            Collection::count_similarity_conditions(&make_similarity_condition()),
            1
        );
        assert_eq!(
            Collection::count_similarity_conditions(&make_compare_condition()),
            0
        );

        let double = Condition::And(
            Box::new(make_similarity_condition()),
            Box::new(make_similarity_condition()),
        );
        assert_eq!(Collection::count_similarity_conditions(&double), 2);
    }
}
