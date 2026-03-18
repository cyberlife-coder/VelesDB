//! Aggregation query execution for VelesQL (EPIC-017 US-002, US-003, US-006).
//!
//! Implements streaming aggregation with O(1) memory complexity.
//! Supports GROUP BY for grouped aggregations (US-003).
//! Supports HAVING for filtering groups (US-006).
//! Supports parallel aggregation with rayon (EPIC-018 US-001).

// SAFETY: Numeric casts in aggregation are intentional:
// - All casts are for computing aggregate statistics (sum, avg, count)
// - f64/u64 casts for maintaining precision in intermediate calculations
// - i64->usize for group limits: limits bounded by MAX_GROUPS (1M)
// - Values bounded by result set size and field cardinality
// - Precision loss acceptable for aggregation results
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

mod grouped;
mod having;
#[cfg(test)]
mod having_tests;

use super::where_eval::GraphMatchEvalCache;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{AggregateFunction, Aggregator, Query, SelectColumns};
use rayon::prelude::*;
use rustc_hash::FxHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Group key for GROUP BY operations with pre-computed hash.
/// Avoids JSON serialization overhead by using direct value hashing.
#[derive(Clone)]
pub(crate) struct GroupKey {
    /// Original values for result construction
    pub(crate) values: Vec<serde_json::Value>,
    /// Pre-computed hash for fast HashMap lookup
    hash: u64,
}

impl GroupKey {
    pub(crate) fn new(values: Vec<serde_json::Value>) -> Self {
        let hash = Self::compute_hash(&values);
        Self { values, hash }
    }

    fn compute_hash(values: &[serde_json::Value]) -> u64 {
        let mut hasher = FxHasher::default();
        for v in values {
            Self::hash_value(v, &mut hasher);
        }
        hasher.finish()
    }

    fn hash_value(value: &serde_json::Value, hasher: &mut FxHasher) {
        match value {
            serde_json::Value::Null => 0u8.hash(hasher),
            serde_json::Value::Bool(b) => {
                1u8.hash(hasher);
                b.hash(hasher);
            }
            serde_json::Value::Number(n) => {
                2u8.hash(hasher);
                // Use bits for consistent hashing of floats
                if let Some(f) = n.as_f64() {
                    f.to_bits().hash(hasher);
                }
            }
            serde_json::Value::String(s) => {
                3u8.hash(hasher);
                s.hash(hasher);
            }
            _ => {
                // Arrays and objects: fallback to string representation
                4u8.hash(hasher);
                value.to_string().hash(hasher);
            }
        }
    }
}

impl Hash for GroupKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl PartialEq for GroupKey {
    fn eq(&self, other: &Self) -> bool {
        // Fast path: different hash means definitely different
        self.hash == other.hash && self.values == other.values
    }
}

impl Eq for GroupKey {}

/// Context for runtime WHERE evaluation during aggregation.
pub(super) struct RuntimeWhereCtx<'a> {
    pub(super) vector_storage: &'a dyn VectorStorage,
    pub(super) stmt: &'a crate::velesql::SelectStatement,
    pub(super) params: &'a HashMap<String, serde_json::Value>,
    pub(super) needs_vector_eval: bool,
    pub(super) graph_cache: &'a mut GraphMatchEvalCache,
}

/// Context for sequential aggregation over a set of IDs.
struct SequentialAggCtx<'a> {
    payload_storage: &'a dyn PayloadStorage,
    vector_storage: &'a dyn VectorStorage,
    stmt: &'a crate::velesql::SelectStatement,
    params: &'a HashMap<String, serde_json::Value>,
    filter: Option<&'a crate::filter::Filter>,
    columns_to_aggregate: &'a [String],
    has_count_star: bool,
    use_runtime_where_eval: bool,
}

/// Threshold for switching to parallel aggregation.
/// Below this, sequential is faster due to overhead.
const PARALLEL_THRESHOLD: usize = 10_000;

/// Chunk size for parallel processing.
const CHUNK_SIZE: usize = 1000;

impl Collection {
    /// Execute an aggregation query and return results as JSON.
    ///
    /// Supports COUNT(*), COUNT(column), SUM, AVG, MIN, MAX.
    /// Uses streaming aggregation - O(1) memory, single pass over data.
    ///
    /// # Arguments
    ///
    /// * `query` - Parsed VelesQL query with aggregation functions
    /// * `params` - Query parameters for placeholders
    ///
    /// # Returns
    ///
    /// JSON object with aggregation results, e.g.:
    /// ```json
    /// {"count": 100, "sum_price": 5000.0, "avg_rating": 4.5}
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when SELECT does not contain aggregations, when HAVING is
    /// used without GROUP BY, or when underlying scan/filter/aggregation operations fail.
    pub fn execute_aggregate(
        &self,
        query: &Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let stmt = &query.select;

        let aggregations: &[AggregateFunction] = match &stmt.columns {
            SelectColumns::Aggregations(aggs) => aggs,
            SelectColumns::Mixed { aggregations, .. } => aggregations,
            _ => {
                return Err(crate::error::Error::Config(
                    "execute_aggregate requires aggregation functions in SELECT".to_string(),
                ))
            }
        };

        if let Some(ref group_by) = stmt.group_by {
            return self.execute_grouped_aggregate(
                query,
                aggregations,
                &group_by.columns,
                stmt.having.as_ref(),
                params,
            );
        }

        if stmt.having.is_some() {
            return Err(crate::error::Error::Config(
                "HAVING clause requires GROUP BY clause".to_string(),
            ));
        }

        let agg_result = self.run_ungrouped_aggregation(stmt, aggregations, params)?;
        Ok(Self::build_aggregate_result(aggregations, &agg_result))
    }

    /// Runs the ungrouped aggregation scan (parallel or sequential).
    fn run_ungrouped_aggregation(
        &self,
        stmt: &crate::velesql::SelectStatement,
        aggregations: &[AggregateFunction],
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<crate::velesql::AggregateResult> {
        let where_clause = stmt.where_clause.as_ref();
        let use_runtime_where_eval = where_clause.is_some_and(|cond| {
            Self::condition_contains_graph_match(cond) || Self::condition_requires_vector_eval(cond)
        });

        let filter = Self::build_static_filter(where_clause, use_runtime_where_eval, params);
        let (columns_vec, has_count_star) = Self::prepare_agg_columns(aggregations);

        let payload_storage = self.payload_storage.read();
        let vector_storage = self.vector_storage.read();
        let ids: Vec<u64> = vector_storage.ids();

        if ids.len() >= PARALLEL_THRESHOLD && !use_runtime_where_eval {
            Ok(Self::run_parallel_path(
                &ids,
                &*payload_storage,
                filter.as_ref(),
                &columns_vec,
                has_count_star,
            ))
        } else {
            let ctx = SequentialAggCtx {
                payload_storage: &*payload_storage,
                vector_storage: &*vector_storage,
                stmt,
                params,
                filter: filter.as_ref(),
                columns_to_aggregate: &columns_vec,
                has_count_star,
                use_runtime_where_eval,
            };
            self.aggregate_sequential(&ids, &ctx)
        }
    }

    /// Pre-fetches payloads and runs parallel aggregation.
    fn run_parallel_path(
        ids: &[u64],
        payload_storage: &dyn PayloadStorage,
        filter: Option<&crate::filter::Filter>,
        columns_vec: &[String],
        has_count_star: bool,
    ) -> crate::velesql::AggregateResult {
        let payloads: Vec<Option<serde_json::Value>> = ids
            .iter()
            .map(|&id| payload_storage.retrieve(id).ok().flatten())
            .collect();

        Self::aggregate_parallel(&payloads, filter, columns_vec, has_count_star)
    }

    /// Returns true if the payload passes the static filter.
    pub(super) fn payload_passes_filter(
        filter: &crate::filter::Filter,
        payload: Option<&serde_json::Value>,
    ) -> bool {
        match payload {
            Some(p) => filter.matches(p),
            None => filter.matches(&serde_json::Value::Null),
        }
    }

    /// Feeds one record into an aggregator (count-star + per-column values).
    pub(super) fn accumulate_record(
        aggregator: &mut Aggregator,
        payload: Option<&serde_json::Value>,
        columns_to_aggregate: &[String],
        has_count_star: bool,
    ) {
        if has_count_star {
            aggregator.process_count();
        }
        if let Some(p) = payload {
            for col in columns_to_aggregate {
                if let Some(value) = Self::get_nested_value(p, col) {
                    aggregator.process_value(col, value);
                }
            }
        }
    }

    /// Parallel aggregation on pre-fetched payloads.
    fn aggregate_parallel(
        payloads: &[Option<serde_json::Value>],
        filter: Option<&crate::filter::Filter>,
        columns_to_aggregate: &[String],
        has_count_star: bool,
    ) -> crate::velesql::AggregateResult {
        let partial_aggregators: Vec<Aggregator> = payloads
            .par_chunks(CHUNK_SIZE)
            .map(|chunk| {
                let mut chunk_agg = Aggregator::new();
                for payload in chunk {
                    if let Some(f) = filter {
                        if !Self::payload_passes_filter(f, payload.as_ref()) {
                            continue;
                        }
                    }
                    Self::accumulate_record(
                        &mut chunk_agg,
                        payload.as_ref(),
                        columns_to_aggregate,
                        has_count_star,
                    );
                }
                chunk_agg
            })
            .collect();

        let mut final_agg = Aggregator::new();
        for partial in partial_aggregators {
            final_agg.merge(partial);
        }
        final_agg.finalize()
    }

    /// Evaluates runtime WHERE for a single record, returning whether it passes.
    pub(super) fn runtime_where_passes(
        &self,
        id: u64,
        payload: Option<&serde_json::Value>,
        ctx: &mut RuntimeWhereCtx<'_>,
    ) -> Result<bool> {
        let vector = if ctx.needs_vector_eval {
            ctx.vector_storage.retrieve(id).ok().flatten()
        } else {
            None
        };
        match ctx.stmt.where_clause.as_ref() {
            Some(cond) => self.evaluate_where_condition_for_record(
                cond,
                id,
                payload,
                vector.as_deref(),
                ctx.params,
                &ctx.stmt.from_alias,
                ctx.graph_cache,
            ),
            None => Ok(true),
        }
    }

    /// Checks whether a single record passes the WHERE filter (runtime or static).
    fn record_passes_filter(
        &self,
        id: u64,
        payload: Option<&serde_json::Value>,
        ctx: &SequentialAggCtx<'_>,
        needs_vector_eval: bool,
        graph_cache: &mut GraphMatchEvalCache,
    ) -> Result<bool> {
        if ctx.use_runtime_where_eval {
            let mut where_ctx = RuntimeWhereCtx {
                vector_storage: ctx.vector_storage,
                stmt: ctx.stmt,
                params: ctx.params,
                needs_vector_eval,
                graph_cache,
            };
            self.runtime_where_passes(id, payload, &mut where_ctx)
        } else if let Some(f) = ctx.filter {
            Ok(Self::payload_passes_filter(f, payload))
        } else {
            Ok(true)
        }
    }

    /// Sequential aggregation with optional runtime WHERE evaluation.
    fn aggregate_sequential(
        &self,
        ids: &[u64],
        ctx: &SequentialAggCtx<'_>,
    ) -> Result<crate::velesql::AggregateResult> {
        let needs_vector_eval = ctx
            .stmt
            .where_clause
            .as_ref()
            .is_some_and(Self::condition_requires_vector_eval);
        let mut aggregator = Aggregator::new();
        let mut graph_cache = GraphMatchEvalCache::default();

        for &id in ids {
            let payload = ctx.payload_storage.retrieve(id).ok().flatten();
            if !self.record_passes_filter(
                id,
                payload.as_ref(),
                ctx,
                needs_vector_eval,
                &mut graph_cache,
            )? {
                continue;
            }
            Self::accumulate_record(
                &mut aggregator,
                payload.as_ref(),
                ctx.columns_to_aggregate,
                ctx.has_count_star,
            );
        }
        Ok(aggregator.finalize())
    }

    /// Builds the JSON result object from aggregation results.
    fn build_aggregate_result(
        aggregations: &[AggregateFunction],
        agg_result: &crate::velesql::AggregateResult,
    ) -> serde_json::Value {
        let mut result = serde_json::Map::new();

        for agg in aggregations {
            let key = Self::aggregation_result_key(agg);
            let value = Self::aggregation_result_value(agg, agg_result);
            result.insert(key, value);
        }

        serde_json::Value::Object(result)
    }

    // NOTE: aggregation_result_key and aggregation_result_value are in grouped.rs
    // and shared by both grouped and ungrouped aggregation paths.

    /// Get a nested value from JSON payload using dot notation.
    pub(crate) fn get_nested_value<'a>(
        payload: &'a serde_json::Value,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = payload;

        for part in parts {
            match current {
                serde_json::Value::Object(map) => {
                    current = map.get(part)?;
                }
                _ => return None,
            }
        }

        Some(current)
    }
}
