//! Condition extraction utilities for VelesQL queries.
//!
//! Extracts vector searches, similarity conditions, and metadata filters
//! from complex WHERE clause condition trees.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::velesql::Condition;

impl Collection {
    /// The text of a WHERE's `MATCH`, found under `AND` and groups: the only
    /// place a validated query holds one, and only one
    /// ([`crate::velesql::validate_vector_predicates`] refuses a `MATCH`
    /// under `OR` / `NOT`, a second one, and one beside a ranking no
    /// executor fuses with text).
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
    ///
    /// Walks `AND`, `OR` and groups. A `NEAR` under an `OR` is still the
    /// query's ranking vector: the leaf is TRUE in a boolean context, so the
    /// disjunction it sits in admits every candidate the search ranks, and
    /// [`extract_metadata_filter`](Self::extract_metadata_filter) drops that
    /// `OR` from the payload filter accordingly. `NOT` is not walked: a
    /// negated vector leaf takes the full-scan path
    /// (`has_similarity_under_not`) before the dispatch that reads this.
    ///
    /// Delegates to [`bind_vector`](crate::velesql::bind_vector) for parameter
    /// resolution, eliminating the duplicated f64-to-f32 conversion logic.
    pub(crate) fn extract_vector_search(
        condition: &Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<Vec<f32>>> {
        match condition {
            Condition::VectorSearch(vs) => {
                let vec = crate::velesql::bind_vector(&vs.vector, params)?;
                Ok(Some(vec))
            }
            Condition::And(left, right) | Condition::Or(left, right) => {
                if let Some(v) = Self::extract_vector_search(left, params)? {
                    return Ok(Some(v));
                }
                Self::extract_vector_search(right, params)
            }
            Condition::Group(inner) => Self::extract_vector_search(inner, params),
            _ => Ok(None),
        }
    }

    /// Extracts a `NEAR_FUSED` multi-vector search from the WHERE clause: the
    /// resolved query vectors plus the fusion config, routed to
    /// [`multi_query_search`](Self::multi_query_search). Walks `AND` and
    /// groups only — a `NEAR_FUSED` under `OR`/`NOT` is refused first by
    /// [`validate_similarity_query_structure`](Self::validate_similarity_query_structure)
    /// — and reuses [`bind_vector`](crate::velesql::bind_vector) for each `VectorExpr`.
    pub(crate) fn extract_fused_vectors(
        condition: &Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<(Vec<Vec<f32>>, crate::velesql::FusionConfig)>> {
        match condition {
            Condition::VectorFusedSearch(vfs) => {
                let vectors = vfs
                    .vectors
                    .iter()
                    .map(|v| crate::velesql::bind_vector(v, params))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Some((vectors, vfs.fusion.clone())))
            }
            Condition::And(left, right) => {
                if let Some(v) = Self::extract_fused_vectors(left, params)? {
                    return Ok(Some(v));
                }
                Self::extract_fused_vectors(right, params)
            }
            Condition::Group(inner) => Self::extract_fused_vectors(inner, params),
            _ => Ok(None),
        }
    }

    /// Extract ALL similarity conditions from WHERE clause (EPIC-044 US-001).
    /// Returns Vec of (field, vector, operator, threshold) for cascade filtering.
    ///
    /// Delegates to [`bind_vector`](crate::velesql::bind_vector) for parameter
    /// resolution, eliminating the duplicated f64-to-f32 conversion logic.
    #[allow(clippy::type_complexity)]
    pub(crate) fn extract_all_similarity_conditions(
        condition: &Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<(String, Vec<f32>, crate::velesql::CompareOp, f64)>> {
        match condition {
            Condition::Similarity(sim) => {
                let vec = crate::velesql::bind_vector(&sim.vector, params)?;
                Ok(vec![(sim.field.clone(), vec, sim.operator, sim.threshold)])
            }
            // AND/OR: collect from both sides (AND=cascade, OR=validation only)
            Condition::And(left, right) | Condition::Or(left, right) => {
                let mut results = Self::extract_all_similarity_conditions(left, params)?;
                results.extend(Self::extract_all_similarity_conditions(right, params)?);
                Ok(results)
            }
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::extract_all_similarity_conditions(inner, params)
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
            // Remove vector/sparse/graph/text search conditions — they are
            // handled separately by the query executor and must not be pushed
            // down as payload filters. A text `MATCH` is the BM25 ranking the
            // dispatch routes the query to, this filter its admission
            // (`text_search_where`); `validate_vector_predicates` refuses every
            // shape where dropping it here would lose it.
            Condition::Similarity(_)
            | Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_)
            | Condition::SparseVectorSearch(_)
            | Condition::GraphMatch(_)
            | Condition::Match(_) => None,
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
            // Handle NOT: preserve NOT wrapper if inner condition exists.
            //
            // This is only sound when the negation applies to metadata alone.
            // Re-wrapping what survives the recursion drops any non-metadata
            // leaf out of the negation first, so `NOT (similarity AND meta)`
            // reduces to `NOT meta` — the similarity conjunct silently gone,
            // and the negation distributed over an `AND` it no longer spans.
            //
            // A comment here used to justify that with "NOT similarity() is
            // rejected earlier in validation". It was true until EPIC-044
            // US-003 enabled the full-scan path, and nothing updated it.
            // The `NOT similarity()` scan therefore no longer uses this
            // function to decide anything: it evaluates the whole condition
            // per candidate through the shared WHERE evaluator, which applies
            // De Morgan correctly (#2112). Callers that still push the result
            // down as a payload filter must not hand it a `NOT` spanning a
            // similarity leaf.
            Condition::Not(inner) => {
                Self::extract_metadata_filter(inner).map(|c| Condition::Not(Box::new(c)))
            }
            // Keep all other conditions (comparisons, IN, BETWEEN, etc.)
            other => Some(other.clone()),
        }
    }

    /// Collect graph MATCH predicates embedded in SELECT WHERE.
    pub(crate) fn collect_graph_match_predicates(
        condition: &Condition,
        out: &mut Vec<crate::velesql::GraphMatchPredicate>,
    ) {
        match condition {
            Condition::GraphMatch(gm) => out.push(gm.clone()),
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::collect_graph_match_predicates(left, out);
                Self::collect_graph_match_predicates(right, out);
            }
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::collect_graph_match_predicates(inner, out);
            }
            _ => {}
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
    ///
    /// Returns `None` when the pair can't be scored (length mismatch, or both
    /// empty) — the metric has no value there. Reporting `0.0` used to stand
    /// in for that case, but `0.0` reads as a perfect match for a distance
    /// metric like Euclidean, letting a malformed vector pass a
    /// `similarity() < threshold` filter as the best candidate. Callers must
    /// skip the candidate on `None`, not substitute a score.
    pub(crate) fn compute_metric_score(&self, a: &[f32], b: &[f32]) -> Option<f32> {
        let metric = self.storage.config.read().metric;
        metric.checked_calculate(a, b)
    }
}

// Tests moved to extraction_tests.rs per project rules
