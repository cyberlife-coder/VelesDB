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
                            arr.iter()
                                .map(|v| {
                                    v.as_f64()
                                        .and_then(|f| {
                                            // Validate f64 value is within f32 representable range
                                            if f.is_finite()
                                                && f >= f64::from(f32::MIN)
                                                && f <= f64::from(f32::MAX)
                                            {
                                                #[allow(clippy::cast_possible_truncation)]
                                                Some(f as f32)
                                            } else if f.is_finite() {
                                                None
                                            } else {
                                                #[allow(clippy::cast_possible_truncation)]
                                                Some(f as f32)
                                            }
                                        })
                                        .ok_or_else(|| {
                                            Error::Config(format!(
                                                "Invalid vector parameter ${name}: value out of f32 range or not a number"
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
                            arr.iter()
                                .map(|v| {
                                    v.as_f64()
                                        .and_then(|f| {
                                            if f.is_finite()
                                                && f >= f64::from(f32::MIN)
                                                && f <= f64::from(f32::MAX)
                                            {
                                                #[allow(clippy::cast_possible_truncation)]
                                                Some(f as f32)
                                            } else if f.is_finite() {
                                                None
                                            } else {
                                                #[allow(clippy::cast_possible_truncation)]
                                                Some(f as f32)
                                            }
                                        })
                                        .ok_or_else(|| {
                                            Error::Config(format!(
                                                "Invalid vector parameter ${name}: value out of f32 range or not a number"
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
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The parameter is missing
    /// - The parameter is not an array
    /// - Any value is not a number or is outside f32 representable range
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
                    arr.iter()
                        .map(|v| {
                            v.as_f64()
                                .and_then(|f| {
                                    if f.is_finite()
                                        && f >= f64::from(f32::MIN)
                                        && f <= f64::from(f32::MAX)
                                    {
                                        #[allow(clippy::cast_possible_truncation)]
                                        Some(f as f32)
                                    } else if f.is_finite() {
                                        None
                                    } else {
                                        #[allow(clippy::cast_possible_truncation)]
                                        Some(f as f32)
                                    }
                                })
                                .ok_or_else(|| {
                                    Error::Config(format!(
                                        "Invalid vector parameter ${name}: value out of f32 range or not a number"
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

// Tests moved to extraction_tests.rs per project rules
