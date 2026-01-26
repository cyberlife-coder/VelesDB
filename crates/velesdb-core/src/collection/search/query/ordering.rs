//! ORDER BY clause execution for VelesQL queries.
//!
//! Handles multi-column sorting with support for:
//! - Metadata field sorting (ASC/DESC)
//! - similarity() function sorting
//! - Mixed type JSON value comparison with total ordering

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use std::cmp::Ordering;

/// Compare two JSON values for sorting with total ordering.
///
/// Ordering priority (ascending): Null < Bool < Number < String < Array < Object
/// This ensures deterministic sorting even with mixed types.
pub fn compare_json_values(
    a: Option<&serde_json::Value>,
    b: Option<&serde_json::Value>,
) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(va), Some(vb)) => {
            // BUG FIX: Define total ordering for mixed JSON types
            // Type priority: Null(0) < Bool(1) < Number(2) < String(3) < Array(4) < Object(5)
            let type_rank = |v: &serde_json::Value| -> u8 {
                match v {
                    serde_json::Value::Null => 0,
                    serde_json::Value::Bool(_) => 1,
                    serde_json::Value::Number(_) => 2,
                    serde_json::Value::String(_) => 3,
                    serde_json::Value::Array(_) => 4,
                    serde_json::Value::Object(_) => 5,
                }
            };

            let rank_a = type_rank(va);
            let rank_b = type_rank(vb);

            // First compare by type rank
            if rank_a != rank_b {
                return rank_a.cmp(&rank_b);
            }

            // Same type: compare values
            match (va, vb) {
                (serde_json::Value::Number(na), serde_json::Value::Number(nb)) => {
                    let fa = na.as_f64().unwrap_or(0.0);
                    let fb = nb.as_f64().unwrap_or(0.0);
                    fa.total_cmp(&fb) // Use total_cmp for NaN safety
                }
                (serde_json::Value::String(sa), serde_json::Value::String(sb)) => sa.cmp(sb),
                (serde_json::Value::Bool(ba), serde_json::Value::Bool(bb)) => ba.cmp(bb),
                // Null vs Null, Array vs Array, Object vs Object: treat as equal
                // (comparing array/object contents would be complex and rarely needed)
                _ => Ordering::Equal,
            }
        }
    }
}

impl Collection {
    /// Apply ORDER BY clause to results.
    ///
    /// Supports multiple ORDER BY columns with stable sorting.
    /// Each column is compared in order; ties are broken by subsequent columns.
    ///
    /// # Examples
    ///
    /// ```sql
    /// SELECT * FROM collection ORDER BY category ASC, priority DESC
    /// SELECT * FROM collection ORDER BY similarity() DESC, timestamp ASC
    /// ```
    pub(crate) fn apply_order_by(
        &self,
        results: &mut [SearchResult],
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        use crate::velesql::OrderByExpr;

        if order_by.is_empty() {
            return Ok(());
        }

        // BUG-3 FIX: Pre-compute similarity scores for ALL ORDER BY similarity() columns
        // Each similarity() can use a different vector, so we need separate score vectors
        let mut similarity_scores_map: std::collections::HashMap<usize, Vec<f32>> =
            std::collections::HashMap::new();
        for (idx, ob) in order_by.iter().enumerate() {
            if let OrderByExpr::Similarity(sim) = &ob.expr {
                let order_vec = self.resolve_vector(&sim.vector, params)?;
                let scores: Vec<f32> = results
                    .iter()
                    .map(|r| self.compute_metric_score(&r.point.vector, &order_vec))
                    .collect();
                similarity_scores_map.insert(idx, scores);
            }
        }

        // Get metric for similarity comparison direction
        let metric = self.config.read().metric;
        let higher_is_better = metric.higher_is_better();

        // Create index-based sorting to maintain score association
        let mut indices: Vec<usize> = (0..results.len()).collect();

        // BUG-5 FIX: Use stable sort to preserve relative order of equal elements
        // This ensures documented stable multi-column sorting behavior
        indices.sort_by(|&i, &j| {
            // Compare by each ORDER BY column in sequence
            for (idx, ob) in order_by.iter().enumerate() {
                let cmp = match &ob.expr {
                    OrderByExpr::Similarity(_) => {
                        // BUG-3 FIX: Use scores for THIS specific similarity() column
                        if let Some(scores) = similarity_scores_map.get(&idx) {
                            let score_i = scores[i];
                            let score_j = scores[j];
                            score_i.total_cmp(&score_j)
                        } else {
                            Ordering::Equal
                        }
                    }
                    OrderByExpr::Field(field_name) => {
                        let val_i = results[i]
                            .point
                            .payload
                            .as_ref()
                            .and_then(|p| p.get(field_name));
                        let val_j = results[j]
                            .point
                            .payload
                            .as_ref()
                            .and_then(|p| p.get(field_name));
                        compare_json_values(val_i, val_j)
                    }
                    OrderByExpr::Aggregate(_) => {
                        // EPIC-040 US-002: ORDER BY aggregate requires pre-computed values
                        // For raw results (not grouped), aggregates don't apply - skip
                        Ordering::Equal
                    }
                };

                // Apply direction (ASC/DESC)
                let directed_cmp = if ob.descending {
                    // For similarity with distance metrics, invert the comparison
                    if matches!(&ob.expr, OrderByExpr::Similarity(_)) && !higher_is_better {
                        cmp // Distance: lower is better, DESC means keep natural order
                    } else {
                        cmp.reverse()
                    }
                } else {
                    // ASC
                    if matches!(&ob.expr, OrderByExpr::Similarity(_)) && !higher_is_better {
                        cmp.reverse() // Distance: lower is better, ASC means reverse
                    } else {
                        cmp
                    }
                };

                // If not equal, return this comparison
                if directed_cmp != Ordering::Equal {
                    return directed_cmp;
                }
                // Otherwise, continue to next ORDER BY column
            }
            Ordering::Equal
        });

        // Reorder results based on sorted indices
        let sorted_results: Vec<SearchResult> =
            indices.iter().map(|&i| results[i].clone()).collect();
        results.clone_from_slice(&sorted_results);

        // Update scores if similarity was used (use first similarity column's scores)
        if let Some(scores) = similarity_scores_map.get(&0) {
            for (i, result) in results.iter_mut().enumerate() {
                result.score = scores[indices[i]];
            }
        }

        Ok(())
    }
}
