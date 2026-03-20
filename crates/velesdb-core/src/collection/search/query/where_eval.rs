//! Runtime evaluation of WHERE conditions on concrete records.
//!
//! This module is used when a query includes graph predicates (`MATCH (...)`)
//! inside SELECT WHERE so boolean semantics are preserved for AND/OR/NOT.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::{CompareOp, Condition, GraphMatchPredicate};
use std::collections::HashSet;

/// Cache for graph predicate anchor sets during a single query execution.
#[derive(Default)]
pub(crate) struct GraphMatchEvalCache {
    entries: Vec<(GraphMatchPredicate, HashSet<u64>)>,
}

impl GraphMatchEvalCache {
    fn get_or_compute(
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
        let idx = self.entries.len() - 1;
        Ok(&self.entries[idx].1)
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
                &mut cache,
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
            other => Ok(Self::evaluate_metadata_filter(other, ctx.payload)),
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
        let metric = self.config.read().metric;
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

    /// Evaluates a metadata-only condition via the filter engine.
    fn evaluate_metadata_filter(
        condition: &Condition,
        payload: Option<&serde_json::Value>,
    ) -> bool {
        let filter = crate::filter::Filter::new(crate::filter::Condition::from(condition.clone()));
        match payload {
            Some(p) => filter.matches(p),
            None => filter.matches(&serde_json::Value::Null),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql::{CompareOp, Comparison, Value};

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
