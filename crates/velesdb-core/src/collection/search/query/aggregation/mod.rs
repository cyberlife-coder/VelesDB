//! Aggregation query execution for VelesQL (EPIC-017 US-002, US-003, US-006).
//!
//! Implements streaming aggregation with O(1) memory complexity.
//! Supports GROUP BY for grouped aggregations (US-003).
//! Supports HAVING for filtering groups (US-006).
//! Supports parallel aggregation with rayon (EPIC-018 US-001).

mod grouped;
mod having;
#[cfg(test)]
mod having_tests;

use super::match_exec::MatchStorageGuards;
use super::row_universe::{RowScope, RowUniverse};
use super::where_eval::GraphMatchEvalCache;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::grouping::AggregateColumns;
use crate::velesql::{AggregateFunction, Aggregator, GroupInput, Query, SelectColumns};
use std::collections::HashMap;

/// Context for runtime WHERE evaluation during aggregation.
pub(super) struct RuntimeWhereCtx<'a> {
    pub(super) vector_storage: &'a dyn VectorStorage,
    pub(super) stmt: &'a crate::velesql::SelectStatement,
    pub(super) params: &'a HashMap<String, serde_json::Value>,
    pub(super) needs_vector_eval: bool,
    pub(super) graph_cache: &'a mut GraphMatchEvalCache,
    /// The scan loop's hoisted vector/payload guards (decree order),
    /// forwarded into MATCH-in-WHERE evaluation so the nested traversal
    /// never re-acquires the locks this loop already holds — a nested read
    /// acquisition on the same thread deadlocks once a writer queues.
    pub(super) match_guards: &'a MatchStorageGuards<'a>,
}

/// Context for sequential aggregation over a set of IDs.
struct SequentialAggCtx<'a> {
    payload_storage: &'a dyn PayloadStorage,
    vector_storage: &'a dyn VectorStorage,
    stmt: &'a crate::velesql::SelectStatement,
    params: &'a HashMap<String, serde_json::Value>,
    filter: Option<&'a crate::filter::Filter>,
    columns: &'a AggregateColumns,
    use_runtime_where_eval: bool,
    /// See [`RuntimeWhereCtx::match_guards`].
    match_guards: &'a MatchStorageGuards<'a>,
}

/// The rows of `ids` an aggregate reads, each with its payload (`None` when
/// it has none): every one but a row TTL-expired at `now_secs` — an
/// aggregate counts the rows a SELECT returns, and a SELECT returns no
/// expired row. Every aggregate scan reads its rows through this one rule.
pub(super) fn live_rows<'a>(
    ids: &'a [u64],
    payload_storage: &'a dyn PayloadStorage,
    now_secs: u64,
) -> impl Iterator<Item = (u64, Option<serde_json::Value>)> + 'a {
    ids.iter().filter_map(move |&id| {
        let payload = payload_storage.retrieve(id).ok().flatten();
        (!crate::collection::expiry::is_payload_expired(payload.as_ref(), now_secs))
            .then_some((id, payload))
    })
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
    /// Returns an error when the query fails validation (`V018` for HAVING
    /// without GROUP BY among them), when SELECT does not contain
    /// aggregations, for a vector GROUP BY (`NEAR` + `GROUP BY`, a search
    /// that `execute_query` runs), or when underlying scan/filter/aggregation
    /// operations fail.
    pub fn execute_aggregate(
        &self,
        query: &Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        // Validate here too: the Tauri plugin and embedders call this entry
        // point directly, not through `Database::execute_aggregate`, and every
        // V0xx rule, a bad `mode` included (#2267), must hold on every path —
        // as must the query-vector check every SELECT entry point runs.
        self.validate_aggregate(query, params)?;
        // Resolve scalar WHERE parameters once (same guard as the SELECT
        // pipeline) so both the static-filter and runtime-eval paths see
        // bound values and missing parameters fail loudly.
        let resolved_query =
            self.bind_query_where(query, params, super::text_match::WhereReader::Aggregate)?;
        let query = resolved_query.as_ref().unwrap_or(query);
        let stmt = &query.select;

        let aggregations: &[AggregateFunction] = match &stmt.columns {
            SelectColumns::Aggregations(aggs) => aggs,
            SelectColumns::Mixed { aggregations, .. } => aggregations,
            _ => {
                return Err(crate::error::Error::Query(
                    "execute_aggregate requires aggregation functions in SELECT".to_string(),
                ))
            }
        };

        // A vector GROUP BY groups the results of its search, which its
        // LIMIT bounds; run here it would group every point, with no score
        // and no group budget.
        if stmt.is_vector_group_by() {
            return Err(crate::error::Error::Query(
                "a vector GROUP BY (NEAR + GROUP BY) groups search results: run it \
                 with execute_query"
                    .to_string(),
            ));
        }

        if stmt.group_by.is_some() {
            // HAVING thresholds are not part of the WHERE condition tree, so
            // they need their own resolution pass (same loud-failure guard).
            let having = stmt
                .having
                .as_ref()
                .map(|h| Self::resolve_having_params(h, params))
                .transpose()?;
            return self.execute_grouped_aggregate(query, having.as_ref(), params);
        }

        let agg_result = self.run_ungrouped_aggregation(stmt, params)?;
        Ok(Self::build_aggregate_result(aggregations, &agg_result))
    }

    /// Runs the ungrouped aggregation scan (parallel or sequential).
    // Both storage guards are lent by reference into MatchStorageGuards and
    // read throughout the scan, including the rayon parallel path. Order is
    // the decree recorded below; see tests/tokio_blocking_lock_contention_repro.rs.
    #[expect(clippy::significant_drop_tightening)]
    fn run_ungrouped_aggregation(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<crate::velesql::AggregateResult> {
        let where_clause = stmt.where_clause.as_ref();
        let use_runtime_where_eval = where_clause.is_some_and(|cond| {
            Self::condition_contains_graph_match(cond) || Self::condition_requires_vector_eval(cond)
        });

        let filter = Self::build_static_filter(where_clause, use_runtime_where_eval, params)?;
        // The same columns, read the same way, as a GROUP BY's groups.
        let columns = AggregateColumns::new(stmt, None);

        // LOCK ORDER: metric snapshot (config, rank 1) first, then
        // vector_storage(2) before payload_storage(3) — the pair was
        // reversed here. See tests/tokio_blocking_lock_contention_repro.rs. This
        // site also feeds `aggregate_parallel`, which scans under these guards
        // on `pools::on_guarded_pool`, never the global rayon pool.
        let (metric, rows) = self.aggregate_config_snapshot();
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let match_guards =
            MatchStorageGuards::single(metric, rows, &vector_storage, &payload_storage);
        // Id order, so a float SUM or AVG is the same on every run, whatever
        // the storage order: the parallel path sums fixed chunks of this
        // order and merges them in order.
        let ids = Self::aggregate_scan_ids(
            RowScope::of_where(where_clause).universe(rows),
            &vector_storage,
            &payload_storage,
            filter.as_ref(),
        );

        if ids.len() >= PARALLEL_THRESHOLD && !use_runtime_where_eval {
            Ok(Self::aggregate_parallel(
                &ids,
                &*payload_storage,
                filter.as_ref(),
                &columns,
            ))
        } else {
            let ctx = SequentialAggCtx {
                payload_storage: &*payload_storage,
                vector_storage: &*vector_storage,
                stmt,
                params,
                filter: filter.as_ref(),
                columns: &columns,
                use_runtime_where_eval,
                match_guards: &match_guards,
            };
            self.aggregate_sequential(&ids, &ctx)
        }
    }

    /// The checks an aggregate passes before any row is read:
    /// `QueryValidator`'s rules, the WHERE shapes a SELECT refuses
    /// ([`Self::validate_similarity_query_structure`]), then its query
    /// vectors against this collection ([`Self::check_select_query_vectors`]).
    fn validate_aggregate(
        &self,
        query: &Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        crate::velesql::QueryValidator::validate(query)?;
        if let Some(cond) = query.select.where_clause.as_ref() {
            Self::validate_similarity_query_structure(cond)?;
        }
        self.check_select_query_vectors(&query.select, params)
    }

    /// The metric and the collection's rows ([`RowUniverse::of`]), read from
    /// `config` (rank 1) before an aggregate takes its storage guards.
    pub(super) fn aggregate_config_snapshot(
        &self,
    ) -> (crate::distance::DistanceMetric, RowUniverse) {
        let config = self.storage.config.read();
        (config.metric, RowUniverse::of(&config))
    }

    /// The points an aggregate scans, in id order (so a float SUM or AVG is
    /// the same on every run): the existing rows `id = n` / `id IN (...)`
    /// names when the static WHERE does — the lookup EXPLAIN reports — else
    /// every row of `universe`, the rows a SELECT reads: a point no SELECT
    /// can reach counts in no aggregate either. Each is then read through
    /// [`live_rows`], which skips a TTL-expired row, as a SELECT does.
    pub(super) fn aggregate_scan_ids(
        universe: RowUniverse,
        vector_storage: &crate::storage::MmapStorage,
        payload_storage: &crate::storage::LogPayloadStorage,
        filter: Option<&crate::filter::Filter>,
    ) -> Vec<u64> {
        let Some(named) = filter.and_then(|f| f.condition.point_ids()) else {
            let mut ids = universe.ids(vector_storage, payload_storage);
            ids.sort_unstable();
            return ids;
        };
        named
            .into_iter()
            .filter(|&id| universe.contains(vector_storage, payload_storage, id))
            .collect()
    }

    /// Parallel aggregation that streams payloads per chunk (#901).
    ///
    /// Each chunk of IDs retrieves its own payloads inside the closure,
    /// so the full payload set is never materialized at once. The chunks run
    /// on `pools::on_guarded_pool`: the caller holds the storage guards
    /// `payload_storage` borrows from.
    fn aggregate_parallel(
        ids: &[u64],
        payload_storage: &dyn PayloadStorage,
        filter: Option<&crate::filter::Filter>,
        columns: &AggregateColumns,
    ) -> crate::velesql::AggregateResult {
        let now_secs = crate::collection::expiry::now_unix_secs();
        let aggregate_chunk = |chunk: &[u64]| {
            let mut chunk_agg = Aggregator::new();
            for (id, payload) in live_rows(chunk, payload_storage, now_secs) {
                if let Some(f) = filter {
                    if !f.matches_point(id, payload.as_ref()) {
                        continue;
                    }
                }
                columns.accumulate(&mut chunk_agg, &GroupInput::new(id, 0.0, payload.as_ref()));
            }
            chunk_agg
        };
        // The caller holds the storage guards across this scan, so its chunks
        // run on the pool for work over guarded data, never the global one.
        let partial_aggregators: Vec<Aggregator> = crate::pools::on_guarded_pool(
            || crate::pools::map_chunks(ids, CHUNK_SIZE, aggregate_chunk),
            || ids.chunks(CHUNK_SIZE).map(aggregate_chunk).collect(),
        );

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
                Some(ctx.match_guards),
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
                match_guards: ctx.match_guards,
            };
            self.runtime_where_passes(id, payload, &mut where_ctx)
        } else if let Some(f) = ctx.filter {
            Ok(f.matches_point(id, payload))
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
        let now_secs = crate::collection::expiry::now_unix_secs();

        for (id, payload) in live_rows(ids, ctx.payload_storage, now_secs) {
            if !self.record_passes_filter(
                id,
                payload.as_ref(),
                ctx,
                needs_vector_eval,
                &mut graph_cache,
            )? {
                continue;
            }
            ctx.columns
                .accumulate(&mut aggregator, &GroupInput::new(id, 0.0, payload.as_ref()));
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
            result.insert(agg.result_key(), agg_result.value_of(agg));
        }

        serde_json::Value::Object(result)
    }
}
