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
        from_alias: Option<&str>,
    ) -> Result<&HashSet<u64>> {
        if let Some(idx) = self.entries.iter().position(|(p, _)| p == predicate) {
            return Ok(&self.entries[idx].1);
        }

        let ids = collection.evaluate_graph_match_anchor_ids(predicate, params, from_alias)?;
        self.entries.push((predicate.clone(), ids));
        let idx = self.entries.len() - 1;
        Ok(&self.entries[idx].1)
    }
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
        from_alias: Option<&str>,
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
                from_alias,
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
        from_alias: Option<&str>,
        graph_cache: &mut GraphMatchEvalCache,
    ) -> Result<bool> {
        match condition {
            Condition::GraphMatch(predicate) => {
                let ids = graph_cache.get_or_compute(self, predicate, params, from_alias)?;
                Ok(ids.contains(&id))
            }
            Condition::And(left, right) => Ok(self.evaluate_where_condition_for_record(
                left,
                id,
                payload,
                vector,
                params,
                from_alias,
                graph_cache,
            )? && self.evaluate_where_condition_for_record(
                right,
                id,
                payload,
                vector,
                params,
                from_alias,
                graph_cache,
            )?),
            Condition::Or(left, right) => Ok(self.evaluate_where_condition_for_record(
                left,
                id,
                payload,
                vector,
                params,
                from_alias,
                graph_cache,
            )? || self.evaluate_where_condition_for_record(
                right,
                id,
                payload,
                vector,
                params,
                from_alias,
                graph_cache,
            )?),
            Condition::Not(inner) => Ok(!self.evaluate_where_condition_for_record(
                inner,
                id,
                payload,
                vector,
                params,
                from_alias,
                graph_cache,
            )?),
            Condition::Group(inner) => self.evaluate_where_condition_for_record(
                inner,
                id,
                payload,
                vector,
                params,
                from_alias,
                graph_cache,
            ),
            Condition::Similarity(sim) => {
                let Some(record_vector) = vector else {
                    return Ok(false);
                };
                let query_vec = Self::resolve_vector(&sim.vector, params)?;
                let score = self.compute_metric_score(record_vector, &query_vec);
                let metric = self.config.read().metric;
                #[allow(clippy::cast_possible_truncation)]
                // Reason: similarity thresholds are approximate floating bounds.
                let threshold = sim.threshold as f32;

                let is_match = if metric.higher_is_better() {
                    match sim.operator {
                        CompareOp::Gt => score > threshold,
                        CompareOp::Gte => score >= threshold,
                        CompareOp::Lt => score < threshold,
                        CompareOp::Lte => score <= threshold,
                        CompareOp::Eq => (score - threshold).abs() < 0.001,
                        CompareOp::NotEq => (score - threshold).abs() >= 0.001,
                    }
                } else {
                    match sim.operator {
                        CompareOp::Gt => score < threshold,
                        CompareOp::Gte => score <= threshold,
                        CompareOp::Lt => score > threshold,
                        CompareOp::Lte => score >= threshold,
                        CompareOp::Eq => (score - threshold).abs() < 0.001,
                        CompareOp::NotEq => (score - threshold).abs() >= 0.001,
                    }
                };
                Ok(is_match)
            }
            Condition::VectorSearch(_) | Condition::VectorFusedSearch(_) => {
                // Candidate generation already enforced these predicates upstream.
                Ok(true)
            }
            other => {
                let filter =
                    crate::filter::Filter::new(crate::filter::Condition::from(other.clone()));
                Ok(match payload {
                    Some(p) => filter.matches(p),
                    None => filter.matches(&serde_json::Value::Null),
                })
            }
        }
    }
}
