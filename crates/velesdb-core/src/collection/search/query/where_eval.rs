//! Runtime evaluation of WHERE conditions on concrete records.
//!
//! This module is used when a query includes graph predicates (`MATCH (...)`)
//! inside SELECT WHERE so boolean semantics are preserved for AND/OR/NOT.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::{CompareOp, Condition, GraphMatchPredicate};
use std::collections::HashSet;

/// Per-query evaluation cache shared across all result rows.
///
/// Holds graph-predicate anchor sets and (#904) the `Filter` built for each
/// metadata-leaf condition node, so neither is recomputed per row.
#[derive(Default)]
pub(crate) struct GraphMatchEvalCache {
    entries: Vec<(GraphMatchPredicate, HashSet<u64>)>,
    /// #904: cached `Filter`s for metadata-leaf conditions, keyed by the leaf
    /// node's pointer address. The borrowed condition AST is the *same* across
    /// every row of a single evaluation, so pointer identity is a stable key
    /// and lets us build each leaf `Filter` exactly once instead of per row.
    filters: Vec<(usize, crate::filter::Filter)>,
}

impl GraphMatchEvalCache {
    pub(super) fn get_or_compute(
        &mut self,
        collection: &Collection,
        predicate: &GraphMatchPredicate,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
    ) -> Result<&HashSet<u64>> {
        if let Some(idx) = self.entries.iter().position(|(p, _)| p == predicate) {
            return Ok(&self.entries[idx].1);
        }

        let ids = collection.evaluate_graph_match_anchor_ids(predicate, params, from_aliases)?;
        self.entries.push((predicate.clone(), ids));
        let entry_idx = self.entries.len() - 1;
        Ok(&self.entries[entry_idx].1)
    }

    /// Returns the cached `Filter` for a metadata-leaf `condition`, building it
    /// once on first use (#904).
    fn metadata_filter(&mut self, condition: &Condition) -> &crate::filter::Filter {
        let key = std::ptr::from_ref(condition) as usize;
        if let Some(idx) = self.filters.iter().position(|(k, _)| *k == key) {
            return &self.filters[idx].1;
        }
        let filter = crate::filter::Filter::new(crate::filter::Condition::from(condition.clone()));
        self.filters.push((key, filter));
        let idx = self.filters.len() - 1;
        &self.filters[idx].1
    }

    /// Test seam (#904): number of distinct metadata-leaf `Filter`s built.
    #[cfg(test)]
    pub(crate) fn filters_built(&self) -> usize {
        self.filters.len()
    }
}

/// Bundled record context for WHERE condition evaluation.
///
/// Groups the per-record fields to reduce argument count in recursive calls.
struct WhereEvalCtx<'a> {
    id: u64,
    payload: Option<&'a serde_json::Value>,
    vector: Option<&'a [f32]>,
    params: &'a std::collections::HashMap<String, serde_json::Value>,
    from_aliases: &'a [String],
}

impl Collection {
    /// Returns true when condition tree contains graph MATCH predicates.
    pub(crate) fn condition_contains_graph_match(condition: &Condition) -> bool {
        match condition {
            Condition::GraphMatch(_) => true,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::condition_contains_graph_match(left)
                    || Self::condition_contains_graph_match(right)
            }
            Condition::Not(inner) | Condition::Group(inner) => {
                Self::condition_contains_graph_match(inner)
            }
            _ => false,
        }
    }

    /// Returns true when condition tree contains any OR node.
    pub(crate) fn condition_contains_or(condition: &Condition) -> bool {
        match condition {
            Condition::Or(_, _) => true,
            Condition::And(left, right) => {
                Self::condition_contains_or(left) || Self::condition_contains_or(right)
            }
            Condition::Not(inner) | Condition::Group(inner) => Self::condition_contains_or(inner),
            _ => false,
        }
    }

    /// Returns true when condition evaluation needs vector values.
    pub(crate) fn condition_requires_vector_eval(condition: &Condition) -> bool {
        match condition {
            Condition::Similarity(_) => true,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::condition_requires_vector_eval(left)
                    || Self::condition_requires_vector_eval(right)
            }
            Condition::Not(inner) | Condition::Group(inner) => {
                Self::condition_requires_vector_eval(inner)
            }
            _ => false,
        }
    }

    /// Applies full WHERE semantics to already-fetched results.
    pub(crate) fn apply_where_condition_to_results(
        &self,
        results: Vec<SearchResult>,
        condition: &Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
    ) -> Result<Vec<SearchResult>> {
        let mut cache = GraphMatchEvalCache::default();
        self.apply_where_condition_to_results_with_cache(
            results,
            condition,
            params,
            from_aliases,
            &mut cache,
        )
    }

    /// Like [`Self::apply_where_condition_to_results`], reusing a caller's
    /// evaluation cache — graph anchor sets computed by a GraphFirst
    /// prefilter are not re-evaluated for the exact post-filter pass.
    pub(crate) fn apply_where_condition_to_results_with_cache(
        &self,
        results: Vec<SearchResult>,
        condition: &Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
        cache: &mut GraphMatchEvalCache,
    ) -> Result<Vec<SearchResult>> {
        let requires_vector = Self::condition_requires_vector_eval(condition);
        let mut filtered = Vec::with_capacity(results.len());

        for result in results {
            let vector = if requires_vector {
                Some(result.point.vector.as_slice())
            } else {
                None
            };
            if self.evaluate_where_condition_for_record(
                condition,
                result.point.id,
                result.point.payload.as_ref(),
                vector,
                params,
                from_aliases,
                cache,
            )? {
                filtered.push(result);
            }
        }

        Ok(filtered)
    }

    /// Evaluate WHERE condition for one record.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn evaluate_where_condition_for_record(
        &self,
        condition: &Condition,
        id: u64,
        payload: Option<&serde_json::Value>,
        vector: Option<&[f32]>,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
        graph_cache: &mut GraphMatchEvalCache,
    ) -> Result<bool> {
        let ctx = WhereEvalCtx {
            id,
            payload,
            vector,
            params,
            from_aliases,
        };
        self.eval_condition(condition, &ctx, graph_cache)
    }

    /// Recursively evaluates a single condition node.
    fn eval_condition(
        &self,
        condition: &Condition,
        ctx: &WhereEvalCtx<'_>,
        graph_cache: &mut GraphMatchEvalCache,
    ) -> Result<bool> {
        match condition {
            Condition::GraphMatch(predicate) => {
                let ids =
                    graph_cache.get_or_compute(self, predicate, ctx.params, ctx.from_aliases)?;
                Ok(ids.contains(&ctx.id))
            }
            Condition::And(left, right) => {
                self.eval_short_circuit_and(left, right, ctx, graph_cache)
            }
            Condition::Or(left, right) => self.eval_short_circuit_or(left, right, ctx, graph_cache),
            Condition::Not(inner) => self.eval_condition(inner, ctx, graph_cache).map(|v| !v),
            Condition::Group(inner) => self.eval_condition(inner, ctx, graph_cache),
            Condition::Similarity(sim) => self.evaluate_similarity(sim, ctx.vector, ctx.params),
            Condition::VectorSearch(_) | Condition::VectorFusedSearch(_) => Ok(true),
            // #904: reuse the per-query cached `Filter` for this metadata leaf
            // instead of rebuilding it (and cloning the AST) on every row.
            other => {
                let filter = graph_cache.metadata_filter(other);
                Ok(Self::payload_passes_filter(filter, ctx.payload))
            }
        }
    }

    /// Evaluates AND with short-circuit: returns false immediately if left is false.
    fn eval_short_circuit_and(
        &self,
        left: &Condition,
        right: &Condition,
        ctx: &WhereEvalCtx<'_>,
        graph_cache: &mut GraphMatchEvalCache,
    ) -> Result<bool> {
        if !self.eval_condition(left, ctx, graph_cache)? {
            return Ok(false);
        }
        self.eval_condition(right, ctx, graph_cache)
    }

    /// Evaluates OR with short-circuit: returns true immediately if left is true.
    fn eval_short_circuit_or(
        &self,
        left: &Condition,
        right: &Condition,
        ctx: &WhereEvalCtx<'_>,
        graph_cache: &mut GraphMatchEvalCache,
    ) -> Result<bool> {
        if self.eval_condition(left, ctx, graph_cache)? {
            return Ok(true);
        }
        self.eval_condition(right, ctx, graph_cache)
    }

    /// Evaluates a similarity condition against a record's vector.
    fn evaluate_similarity(
        &self,
        sim: &crate::velesql::SimilarityCondition,
        vector: Option<&[f32]>,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<bool> {
        let Some(record_vector) = vector else {
            return Ok(false);
        };
        let query_vec = Self::resolve_vector(&sim.vector, params)?;
        let score = self.compute_metric_score(record_vector, &query_vec);
        let metric = self.storage.config.read().metric;
        #[allow(clippy::cast_possible_truncation)]
        // Reason: similarity thresholds are approximate floating bounds.
        let threshold = sim.threshold as f32;
        Ok(Self::compare_score(
            score,
            threshold,
            sim.operator,
            metric.higher_is_better(),
        ))
    }

    /// Compares a score against a threshold using the given operator and metric direction.
    pub(crate) fn compare_score(
        score: f32,
        threshold: f32,
        op: CompareOp,
        higher_is_better: bool,
    ) -> bool {
        if higher_is_better {
            match op {
                CompareOp::Gt => score > threshold,
                CompareOp::Gte => score >= threshold,
                CompareOp::Lt => score < threshold,
                CompareOp::Lte => score <= threshold,
                CompareOp::Eq => (score - threshold).abs() < 0.001,
                CompareOp::NotEq => (score - threshold).abs() >= 0.001,
            }
        } else {
            match op {
                CompareOp::Gt => score < threshold,
                CompareOp::Gte => score <= threshold,
                CompareOp::Lt => score > threshold,
                CompareOp::Lte => score >= threshold,
                CompareOp::Eq => (score - threshold).abs() < 0.001,
                CompareOp::NotEq => (score - threshold).abs() >= 0.001,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql::{CompareOp, Comparison, Value};

    /// #904: the metadata `Filter` for a leaf condition is built **once** and
    /// reused across repeated evaluations of the same AST node (one per result
    /// row), instead of rebuilding + cloning per row. Results are unchanged.
    #[test]
    fn test_metadata_filter_built_once_per_leaf() {
        let cond = Condition::Comparison(Comparison {
            column: "status".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("active".to_string()),
        });

        let mut cache = GraphMatchEvalCache::default();
        let active = serde_json::json!({"status": "active"});
        let inactive = serde_json::json!({"status": "inactive"});

        // Evaluate the SAME borrowed node many times (simulates N result rows).
        for _ in 0..100 {
            let f = cache.metadata_filter(&cond);
            assert!(f.matches(&active));
            assert!(!f.matches(&inactive));
        }

        assert_eq!(
            cache.filters_built(),
            1,
            "metadata Filter must be built exactly once, not per row"
        );
    }

    /// #904: distinct leaf nodes each get their own cached `Filter`.
    #[test]
    fn test_metadata_filter_distinct_leaves_cached_separately() {
        let cond_a = Condition::Comparison(Comparison {
            column: "a".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        });
        let cond_b = Condition::Comparison(Comparison {
            column: "b".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(2),
        });

        let mut cache = GraphMatchEvalCache::default();
        let _ = cache.metadata_filter(&cond_a);
        let _ = cache.metadata_filter(&cond_b);
        let _ = cache.metadata_filter(&cond_a);

        assert_eq!(cache.filters_built(), 2);
    }

    #[test]
    fn test_condition_contains_or_detects_nested_or() {
        let cond = Condition::And(
            Box::new(Condition::Comparison(Comparison {
                column: "status".to_string(),
                operator: CompareOp::Eq,
                value: Value::String("active".to_string()),
            })),
            Box::new(Condition::Group(Box::new(Condition::Or(
                Box::new(Condition::Comparison(Comparison {
                    column: "tier".to_string(),
                    operator: CompareOp::Eq,
                    value: Value::String("pro".to_string()),
                })),
                Box::new(Condition::Comparison(Comparison {
                    column: "tier".to_string(),
                    operator: CompareOp::Eq,
                    value: Value::String("enterprise".to_string()),
                })),
            )))),
        );

        assert!(Collection::condition_contains_or(&cond));
    }

    #[test]
    fn test_condition_contains_or_false_without_or() {
        let cond = Condition::And(
            Box::new(Condition::Comparison(Comparison {
                column: "status".to_string(),
                operator: CompareOp::Eq,
                value: Value::String("active".to_string()),
            })),
            Box::new(Condition::Not(Box::new(Condition::Comparison(
                Comparison {
                    column: "deleted".to_string(),
                    operator: CompareOp::Eq,
                    value: Value::Boolean(true),
                },
            )))),
        );

        assert!(!Collection::condition_contains_or(&cond));
    }
}
