//! Grouped aggregation query execution (GROUP BY).
//!
//! Extracted from aggregation.rs for complexity reduction (Plan 04-04).
//! Rows are grouped by the engine-shared [`crate::velesql::Grouper`]; sorting
//! and parameter resolution are in `having.rs`.

use super::super::match_exec::MatchStorageGuards;
use super::super::where_eval::GraphMatchEvalCache;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::PayloadStorage;
use crate::velesql::{GroupInput, Grouper, HavingClause, Query};
use std::collections::HashMap;

impl Collection {
    /// Execute a grouped aggregation query (GROUP BY) with optional HAVING
    /// filter, `having` with its parameters bound.
    pub(crate) fn execute_grouped_aggregate(
        &self,
        query: &Query,
        having: Option<&HavingClause>,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let stmt = &query.select;
        // The grouper returns its groups by GROUP BY key already: only an
        // ORDER BY sorts them again.
        let sort_columns = if stmt.order_by.is_some() {
            stmt.grouped_sort_keys()
                .map_err(|e| crate::error::Error::Query(e.to_string()))?
        } else {
            Vec::new()
        };

        // Rows carry no search score here, so the metric direction is moot.
        let mut grouper = Grouper::new(stmt, having, true)?;
        self.scan_and_group(stmt, &mut grouper, params)?;

        let rows = grouper
            .finish()
            .into_iter()
            .map(|group| group.row)
            .collect();
        let results = Self::order_and_page(rows, &sort_columns, stmt.limit, stmt.offset);
        Ok(serde_json::Value::Array(results))
    }

    /// Scans all points, applies WHERE filter, and feeds each kept row to
    /// `grouper`.
    // Both storage guards are lent by reference into MatchStorageGuards and
    // read throughout the scan. Acquisition order is the decree recorded
    // below (2 then 3); see .investigation/http-deadlock-2026-07-22/.
    #[expect(clippy::significant_drop_tightening)]
    fn scan_and_group(
        &self,
        stmt: &crate::velesql::SelectStatement,
        grouper: &mut Grouper<'_>,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        let where_clause = stmt.where_clause.as_ref();
        let use_runtime = Self::needs_runtime_where_eval(where_clause);
        let needs_vector_eval = where_clause.is_some_and(Self::condition_requires_vector_eval);
        let filter = Self::build_static_filter(where_clause, use_runtime, params)?;

        // LOCK ORDER: metric snapshot (config, rank 1) first, then
        // vector_storage(2) before payload_storage(3) — the pair was
        // reversed here. See .investigation/http-deadlock-2026-07-22/.
        let metric = self.storage.config.read().metric;
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let match_guards = MatchStorageGuards {
            metric,
            vector_guard: &vector_storage,
            payload_guard: &payload_storage,
            payload_memo: crate::collection::search::query::match_exec::PayloadMemo::new(
                &payload_storage,
            ),
            where_filters: crate::collection::search::query::match_exec::WhereFilterMemo::default(),
        };
        // Rows feed the grouper in id order, so a float SUM or AVG adds its
        // values in the same order on every run, whatever the storage order.
        let ids = Self::aggregate_scan_ids(&*vector_storage, filter.as_ref());
        let mut graph_cache = GraphMatchEvalCache::default();

        for id in ids {
            let payload = payload_storage.retrieve(id).ok().flatten();
            let passes = if use_runtime {
                let mut rt_ctx = super::RuntimeWhereCtx {
                    vector_storage: &*vector_storage,
                    stmt,
                    params,
                    needs_vector_eval,
                    graph_cache: &mut graph_cache,
                    match_guards: &match_guards,
                };
                self.runtime_where_passes(id, payload.as_ref(), &mut rt_ctx)?
            } else if let Some(ref f) = filter {
                f.matches_point(id, payload.as_ref())
            } else {
                true
            };
            if !passes {
                continue;
            }
            grouper.push(GroupInput::new(id, 0.0, payload.as_ref()))?;
        }

        Ok(())
    }

    /// Checks whether runtime WHERE evaluation is needed for the given clause.
    fn needs_runtime_where_eval(where_clause: Option<&crate::velesql::Condition>) -> bool {
        where_clause.is_some_and(|cond| {
            Self::condition_contains_graph_match(cond) || Self::condition_requires_vector_eval(cond)
        })
    }

    /// Builds a static `Filter` from the WHERE clause when runtime eval is not needed.
    ///
    /// # Errors
    ///
    /// Returns an error when a WHERE parameter placeholder is missing from
    /// `params` (never silently converts it to `NULL`).
    pub(super) fn build_static_filter(
        where_clause: Option<&crate::velesql::Condition>,
        use_runtime: bool,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Option<crate::filter::Filter>> {
        if use_runtime {
            return Ok(None);
        }
        where_clause
            .map(|cond| {
                let resolved = Self::resolve_condition_params(cond, params)?;
                Ok(crate::filter::Filter::new(crate::filter::Condition::from(
                    resolved,
                )))
            })
            .transpose()
    }

    /// Orders the group rows by ORDER BY, then applies OFFSET/LIMIT (issue
    /// #1556). Unlike plain SELECT there is no default cap: a LIMIT-less
    /// GROUP BY returns every group, matching the WASM aggregate pipeline.
    fn order_and_page(
        mut results: Vec<serde_json::Value>,
        sort_columns: &[(String, bool)],
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Vec<serde_json::Value> {
        Self::sort_aggregation_results(&mut results, sort_columns);

        let skip = usize::try_from(offset.unwrap_or(0)).unwrap_or(usize::MAX);
        let take = limit.map_or(usize::MAX, |l| usize::try_from(l).unwrap_or(usize::MAX));
        if skip > 0 || take < results.len() {
            results = results.into_iter().skip(skip).take(take).collect();
        }

        results
    }
}
