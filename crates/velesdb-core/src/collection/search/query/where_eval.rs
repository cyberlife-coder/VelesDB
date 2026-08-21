//! Runtime evaluation of WHERE conditions on concrete records.
//!
//! This module is used when a query includes graph predicates (`MATCH (...)`)
//! inside SELECT WHERE so boolean semantics are preserved for AND/OR/NOT.

use super::match_exec::MatchStorageGuards;
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
    /// Resolved query vectors for `similarity()` leaves, keyed the same way
    /// (leaf pointer): `resolve_vector` cloned the literal (or re-parsed the
    /// `$param` JSON) for EVERY candidate row — a 3 KB alloc+copy per row at
    /// 768 dims. The vector is invariant across the evaluation.
    similarity_vectors: Vec<(usize, Vec<f32>)>,
    /// Metric snapshot for similarity comparisons: `evaluate_similarity` took
    /// the `config` read lock TWICE per row (once inside
    /// `compute_metric_score`, once for the threshold direction).
    metric: Option<crate::distance::DistanceMetric>,
}

impl GraphMatchEvalCache {
    /// `guards`: pass `Some` when the caller already holds the storage
    /// guards (vector then payload, decree order) so the nested MATCH
    /// traversal reuses them instead of re-acquiring the locks; `None`
    /// otherwise (the traversal then acquires its own guards).
    pub(super) fn get_or_compute(
        &mut self,
        collection: &Collection,
        predicate: &GraphMatchPredicate,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
        guards: Option<&MatchStorageGuards<'_>>,
    ) -> Result<&HashSet<u64>> {
        if let Some(idx) = self.entries.iter().position(|(p, _)| p == predicate) {
            return Ok(&self.entries[idx].1);
        }

        let ids =
            collection.evaluate_graph_match_anchor_ids(predicate, params, from_aliases, guards)?;
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

    /// Returns the resolved query vector for a `similarity()` leaf, resolving
    /// it exactly once per evaluation (same pointer-keyed scheme as
    /// [`Self::metadata_filter`]).
    fn similarity_query_vector(
        &mut self,
        sim: &crate::velesql::SimilarityCondition,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<&[f32]> {
        let key = std::ptr::from_ref(sim) as usize;
        if let Some(idx) = self.similarity_vectors.iter().position(|(k, _)| *k == key) {
            return Ok(&self.similarity_vectors[idx].1);
        }
        let resolved = Collection::resolve_vector(&sim.vector, params)?;
        self.similarity_vectors.push((key, resolved));
        let idx = self.similarity_vectors.len() - 1;
        Ok(&self.similarity_vectors[idx].1)
    }

    /// Returns the collection's metric, reading the `config` lock at most
    /// once per evaluation (the per-row path read it twice per candidate).
    fn metric_snapshot(&mut self, collection: &Collection) -> crate::distance::DistanceMetric {
        if let Some(metric) = self.metric {
            return metric;
        }
        let metric = collection.storage.config.read().metric;
        self.metric = Some(metric);
        metric
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
    /// Storage guards already held by the caller (decree order), forwarded to
    /// nested MATCH-in-WHERE traversals so they never re-acquire the locks.
    guards: Option<&'a MatchStorageGuards<'a>>,
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
                None,
            )? {
                filtered.push(result);
            }
        }

        Ok(filtered)
    }

    /// Evaluate WHERE condition for one record.
    ///
    /// `guards`: `Some` when the caller already holds the storage guards in
    /// decree order (aggregation scan loops) — forwarded so a nested
    /// MATCH-in-WHERE traversal reuses them instead of re-acquiring the
    /// locks; `None` when no storage guard is held.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::collection::search::query) fn evaluate_where_condition_for_record(
        &self,
        condition: &Condition,
        id: u64,
        payload: Option<&serde_json::Value>,
        vector: Option<&[f32]>,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
        graph_cache: &mut GraphMatchEvalCache,
        guards: Option<&MatchStorageGuards<'_>>,
    ) -> Result<bool> {
        let ctx = WhereEvalCtx {
            id,
            payload,
            vector,
            params,
            from_aliases,
            guards,
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
                let ids = graph_cache.get_or_compute(
                    self,
                    predicate,
                    ctx.params,
                    ctx.from_aliases,
                    ctx.guards,
                )?;
                Ok(ids.contains(&ctx.id))
            }
            Condition::And(left, right) => {
                self.eval_short_circuit_and(left, right, ctx, graph_cache)
            }
            Condition::Or(left, right) => self.eval_short_circuit_or(left, right, ctx, graph_cache),
            Condition::Not(inner) => self.eval_condition(inner, ctx, graph_cache).map(|v| !v),
            Condition::Group(inner) => self.eval_condition(inner, ctx, graph_cache),
            Condition::Similarity(sim) => {
                self.evaluate_similarity(sim, ctx.vector, ctx.params, graph_cache)
            }
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
        graph_cache: &mut GraphMatchEvalCache,
    ) -> Result<bool> {
        let Some(record_vector) = vector else {
            return Ok(false);
        };
        // Per-query invariants come from the eval cache: the resolved query
        // vector (was one alloc+copy or `$param` re-parse per row) and the
        // metric snapshot (was TWO `config` read-lock acquisitions per row —
        // one inside `compute_metric_score`, one for the direction).
        let metric = graph_cache.metric_snapshot(self);
        let query_vec = graph_cache.similarity_query_vector(sim, params)?;
        let score = if record_vector.len() == query_vec.len() && !record_vector.is_empty() {
            metric.calculate(record_vector, query_vec)
        } else {
            0.0
        };
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
#[path = "where_eval_tests.rs"]
mod tests;
