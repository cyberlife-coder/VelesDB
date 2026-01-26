//! Condition extraction utilities for VelesQL queries.
//!
//! Extracts vector searches, similarity conditions, and metadata filters
//! from complex WHERE clause condition trees.

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::velesql::Condition;

impl Collection {
    /// Helper to extract MATCH query from any nested condition.
    pub(crate) fn extract_match_query(condition: &Condition) -> Option<String> {
        match condition {
            Condition::Match(m) => Some(m.query.clone()),
            Condition::And(left, right) => {
                Self::extract_match_query(left).or_else(|| Self::extract_match_query(right))
            }
            Condition::Group(inner) => Self::extract_match_query(inner),
            _ => None,
        }
    }

    /// Internal helper to extract vector search from WHERE clause.
    #[allow(clippy::self_only_used_in_recursion)]
    pub(crate) fn extract_vector_search(
        &self,
        condition: &mut Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<Vec<f32>>> {
        use crate::velesql::VectorExpr;

        match condition {
            Condition::VectorSearch(vs) => {
                let vec = match &vs.vector {
                    VectorExpr::Literal(v) => v.clone(),
                    VectorExpr::Parameter(name) => {
                        let val = params.get(name).ok_or_else(|| {
                            Error::Config(format!("Missing query parameter: ${name}"))
                        })?;
                        if let serde_json::Value::Array(arr) = val {
                            #[allow(clippy::cast_possible_truncation)]
                            arr.iter()
                                .map(|v| {
                                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                                        Error::Config(format!(
                                            "Invalid vector parameter ${name}: expected numbers"
                                        ))
                                    })
                                })
                                .collect::<Result<Vec<f32>>>()?
                        } else {
                            return Err(Error::Config(format!(
                                "Invalid vector parameter ${name}: expected array"
                            )));
                        }
                    }
                };
                Ok(Some(vec))
            }
            Condition::And(left, right) => {
                if let Some(v) = self.extract_vector_search(left, params)? {
                    return Ok(Some(v));
                }
                self.extract_vector_search(right, params)
            }
            Condition::Group(inner) => self.extract_vector_search(inner, params),
            _ => Ok(None),
        }
    }

    /// Extract ALL similarity conditions from WHERE clause (EPIC-044 US-001).
    /// Returns Vec of (field, vector, operator, threshold) for cascade filtering.
    #[allow(clippy::type_complexity)]
    #[allow(clippy::only_used_in_recursion)]
    #[allow(clippy::self_only_used_in_recursion)]
    pub(crate) fn extract_all_similarity_conditions(
        &self,
        condition: &Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<(String, Vec<f32>, crate::velesql::CompareOp, f64)>> {
        use crate::velesql::VectorExpr;

        match condition {
            Condition::Similarity(sim) => {
                let vec = match &sim.vector {
                    VectorExpr::Literal(v) => v.clone(),
                    VectorExpr::Parameter(name) => {
                        let val = params.get(name).ok_or_else(|| {
                            Error::Config(format!("Missing query parameter: ${name}"))
                        })?;
                        if let serde_json::Value::Array(arr) = val {
                            #[allow(clippy::cast_possible_truncation)]
                            arr.iter()
                                .map(|v| {
                                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                                        Error::Config(format!(
                                            "Invalid vector parameter ${name}: expected numbers"
                                        ))
                                    })
                                })
                                .collect::<Result<Vec<f32>>>()?
                        } else {
                            return Err(Error::Config(format!(
                                "Invalid vector parameter ${name}: expected array"
                            )));
                        }
                    }
                };
                Ok(vec![(sim.field.clone(), vec, sim.operator, sim.threshold)])
            }
            // AND/OR: collect from both sides (AND=cascade, OR=validation only)
            Condition::And(left, right) | Condition::Or(left, right) => {
                let mut results = self.extract_all_similarity_conditions(left, params)?;
                results.extend(self.extract_all_similarity_conditions(right, params)?);
                Ok(results)
            }
            Condition::Group(inner) | Condition::Not(inner) => {
                self.extract_all_similarity_conditions(inner, params)
            }
            _ => Ok(vec![]),
        }
    }

    /// Extract non-similarity parts of a condition for metadata filtering.
    ///
    /// This removes `SimilarityFilter` conditions from the tree and returns
    /// only the metadata filter parts (e.g., `category = 'tech'`).
    pub(crate) fn extract_metadata_filter(condition: &Condition) -> Option<Condition> {
        match condition {
            // Remove vector search conditions - they're handled separately by the query executor
            Condition::Similarity(_)
            | Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_) => None,
            // For AND: keep both sides if they exist, or just one side
            Condition::And(left, right) => {
                let left_filter = Self::extract_metadata_filter(left);
                let right_filter = Self::extract_metadata_filter(right);
                match (left_filter, right_filter) {
                    (Some(l), Some(r)) => Some(Condition::And(Box::new(l), Box::new(r))),
                    (Some(l), None) => Some(l),
                    (None, Some(r)) => Some(r),
                    (None, None) => None,
                }
            }
            // For OR: both sides must exist
            // FLAG-13: This is intentionally asymmetric with AND.
            // AND can work with partial conditions (e.g., similarity() AND metadata)
            // but OR semantically requires both sides to be evaluable.
            // Without both sides, we cannot properly evaluate the OR condition.
            Condition::Or(left, right) => {
                let left_filter = Self::extract_metadata_filter(left);
                let right_filter = Self::extract_metadata_filter(right);
                match (left_filter, right_filter) {
                    (Some(l), Some(r)) => Some(Condition::Or(Box::new(l), Box::new(r))),
                    _ => None, // OR requires both sides
                }
            }
            // Unwrap groups
            Condition::Group(inner) => {
                Self::extract_metadata_filter(inner).map(|c| Condition::Group(Box::new(c)))
            }
            // Handle NOT: preserve NOT wrapper if inner condition exists
            // Note: NOT similarity() is rejected earlier in validation, so we only
            // need to handle NOT with metadata conditions here
            Condition::Not(inner) => {
                Self::extract_metadata_filter(inner).map(|c| Condition::Not(Box::new(c)))
            }
            // Keep all other conditions (comparisons, IN, BETWEEN, etc.)
            other => Some(other.clone()),
        }
    }

    /// Resolve a vector expression to actual vector values.
    pub(crate) fn resolve_vector(
        &self,
        vector: &crate::velesql::VectorExpr,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<f32>> {
        use crate::velesql::VectorExpr;

        match vector {
            VectorExpr::Literal(v) => Ok(v.clone()),
            VectorExpr::Parameter(name) => {
                let val = params
                    .get(name)
                    .ok_or_else(|| Error::Config(format!("Missing query parameter: ${name}")))?;
                if let serde_json::Value::Array(arr) = val {
                    #[allow(clippy::cast_possible_truncation)]
                    arr.iter()
                        .map(|v| {
                            v.as_f64().map(|f| f as f32).ok_or_else(|| {
                                Error::Config(format!(
                                    "Invalid vector parameter ${name}: expected numbers"
                                ))
                            })
                        })
                        .collect::<Result<Vec<f32>>>()
                } else {
                    Err(Error::Config(format!(
                        "Invalid vector parameter ${name}: expected array"
                    )))
                }
            }
        }
    }

    /// Compute the metric score between two vectors using the collection's configured metric.
    ///
    /// **Note:** This returns the raw metric score, not a normalized similarity.
    /// The interpretation depends on the metric:
    /// - **Cosine**: Returns cosine similarity (higher = more similar)
    /// - **DotProduct**: Returns dot product (higher = more similar)
    /// - **Euclidean**: Returns euclidean distance (lower = more similar)
    /// - **Hamming**: Returns hamming distance (lower = more similar)
    /// - **Jaccard**: Returns jaccard similarity (higher = more similar)
    ///
    /// Use `metric.higher_is_better()` to determine score interpretation.
    pub(crate) fn compute_metric_score(&self, a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        // Use the collection's configured metric for consistent behavior
        let metric = self.config.read().metric;
        metric.calculate(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql::{
        CompareOp, Comparison, MatchCondition, SimilarityCondition, Value, VectorExpr, VectorSearch,
    };

    fn make_comparison(column: &str, val: i64) -> Condition {
        Condition::Comparison(Comparison {
            column: column.to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(val),
        })
    }

    fn make_match(column: &str, query: &str) -> Condition {
        Condition::Match(MatchCondition {
            column: column.to_string(),
            query: query.to_string(),
        })
    }

    fn make_similarity(field: &str, threshold: f64) -> Condition {
        Condition::Similarity(SimilarityCondition {
            field: field.to_string(),
            vector: VectorExpr::Parameter("v".to_string()),
            operator: CompareOp::Gt,
            threshold,
        })
    }

    fn make_vector_search() -> Condition {
        Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Parameter("v".to_string()),
        })
    }

    // =========================================================================
    // extract_match_query tests
    // =========================================================================

    #[test]
    fn test_extract_match_query_direct() {
        let cond = make_match("text", "hello world");
        let result = Collection::extract_match_query(&cond);
        assert_eq!(result, Some("hello world".to_string()));
    }

    #[test]
    fn test_extract_match_query_in_and() {
        let cond = Condition::And(
            Box::new(make_comparison("a", 1)),
            Box::new(make_match("text", "search term")),
        );
        let result = Collection::extract_match_query(&cond);
        assert_eq!(result, Some("search term".to_string()));
    }

    #[test]
    fn test_extract_match_query_in_group() {
        let cond = Condition::Group(Box::new(make_match("text", "query")));
        let result = Collection::extract_match_query(&cond);
        assert_eq!(result, Some("query".to_string()));
    }

    #[test]
    fn test_extract_match_query_none() {
        let cond = make_comparison("a", 1);
        let result = Collection::extract_match_query(&cond);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_match_query_nested_and() {
        let inner = Condition::And(
            Box::new(make_match("text", "inner query")),
            Box::new(make_comparison("b", 2)),
        );
        let cond = Condition::And(Box::new(make_comparison("a", 1)), Box::new(inner));
        let result = Collection::extract_match_query(&cond);
        assert_eq!(result, Some("inner query".to_string()));
    }

    // =========================================================================
    // extract_metadata_filter tests
    // =========================================================================

    #[test]
    fn test_extract_metadata_filter_comparison() {
        let cond = make_comparison("category", 1);
        let result = Collection::extract_metadata_filter(&cond);
        assert!(result.is_some());
    }

    #[test]
    fn test_extract_metadata_filter_removes_similarity() {
        let cond = make_similarity("embedding", 0.8);
        let result = Collection::extract_metadata_filter(&cond);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_metadata_filter_removes_vector_search() {
        let cond = make_vector_search();
        let result = Collection::extract_metadata_filter(&cond);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_metadata_filter_and_with_similarity() {
        let cond = Condition::And(
            Box::new(make_similarity("embedding", 0.8)),
            Box::new(make_comparison("category", 1)),
        );
        let result = Collection::extract_metadata_filter(&cond);
        assert!(result.is_some());
        // Should only contain the comparison
        assert!(matches!(result, Some(Condition::Comparison(_))));
    }

    #[test]
    fn test_extract_metadata_filter_and_both_metadata() {
        let cond = Condition::And(
            Box::new(make_comparison("a", 1)),
            Box::new(make_comparison("b", 2)),
        );
        let result = Collection::extract_metadata_filter(&cond);
        assert!(matches!(result, Some(Condition::And(_, _))));
    }

    #[test]
    fn test_extract_metadata_filter_and_both_similarity() {
        let cond = Condition::And(
            Box::new(make_similarity("e1", 0.8)),
            Box::new(make_similarity("e2", 0.9)),
        );
        let result = Collection::extract_metadata_filter(&cond);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_metadata_filter_or_both_metadata() {
        let cond = Condition::Or(
            Box::new(make_comparison("a", 1)),
            Box::new(make_comparison("b", 2)),
        );
        let result = Collection::extract_metadata_filter(&cond);
        assert!(matches!(result, Some(Condition::Or(_, _))));
    }

    #[test]
    fn test_extract_metadata_filter_or_with_similarity_returns_none() {
        // OR requires both sides, so if one side is similarity, result is None
        let cond = Condition::Or(
            Box::new(make_similarity("embedding", 0.8)),
            Box::new(make_comparison("category", 1)),
        );
        let result = Collection::extract_metadata_filter(&cond);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_metadata_filter_group() {
        let cond = Condition::Group(Box::new(make_comparison("a", 1)));
        let result = Collection::extract_metadata_filter(&cond);
        assert!(matches!(result, Some(Condition::Group(_))));
    }

    #[test]
    fn test_extract_metadata_filter_not() {
        let cond = Condition::Not(Box::new(make_comparison("deleted", 1)));
        let result = Collection::extract_metadata_filter(&cond);
        assert!(matches!(result, Some(Condition::Not(_))));
    }

    #[test]
    fn test_extract_metadata_filter_not_similarity_returns_none() {
        let cond = Condition::Not(Box::new(make_similarity("embedding", 0.8)));
        let result = Collection::extract_metadata_filter(&cond);
        assert!(result.is_none());
    }
}
