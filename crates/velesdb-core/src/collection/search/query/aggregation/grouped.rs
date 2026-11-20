//! Grouped aggregation query execution (GROUP BY).
//!
//! Extracted from aggregation.rs for complexity reduction (Plan 04-04).
//! Rows are grouped by the engine-shared [`crate::velesql::Grouper`]; sorting
//! and parameter resolution are in `having.rs`.

use super::super::match_exec::MatchStorageGuards;
use super::super::row_universe::RowScope;
use super::super::where_eval::GraphMatchEvalCache;
use crate::collection::types::Collection;
use crate::error::Result;
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
            stmt.grouped_sort_keys()?
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

    /// Aggregates rows the caller produced — a JOIN's joined rows, WHERE
    /// already applied — through the same [`Grouper`] as a scan: an array of
    /// groups for a GROUP BY (ordered and paged as
    /// [`Self::execute_grouped_aggregate`] pages them), else the one group's
    /// object.
    ///
    /// # Errors
    ///
    /// An unbound HAVING parameter, an ORDER BY naming no grouped field, or
    /// more groups than the query allows.
    pub(crate) fn aggregate_rows(
        query: &Query,
        rows: &[crate::point::SearchResult],
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let stmt = &query.select;
        let having = stmt
            .having
            .as_ref()
            .map(|h| Self::resolve_having_params(h, params))
            .transpose()?;
        let mut grouper = Grouper::new(stmt, having.as_ref(), true)?;
        // Id order, as a scan feeds the grouper (stable for 1:N joins).
        let mut ordered: Vec<&crate::point::SearchResult> = rows.iter().collect();
        ordered.sort_by_key(|row| row.point.id);
        for row in ordered {
            grouper.push(GroupInput::new(
                row.point.id,
                0.0,
                row.point.payload.as_ref(),
            ))?;
        }
        let groups: Vec<serde_json::Value> = grouper
            .finish()
            .into_iter()
            .map(|group| group.row)
            .collect();
        Self::shape_groups(stmt, groups)
    }

    /// The aggregate's result from its groups: without GROUP BY, the one
    /// group's object (an empty object when no row came); with it, the groups
    /// ordered and paged as [`Self::execute_grouped_aggregate`] pages them.
    fn shape_groups(
        stmt: &crate::velesql::SelectStatement,
        groups: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        if stmt.group_by.is_none() {
            return Ok(groups
                .into_iter()
                .next()
                .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new())));
        }
        let sort_columns = if stmt.order_by.is_some() {
            stmt.grouped_sort_keys()?
        } else {
            Vec::new()
        };
        Ok(serde_json::Value::Array(Self::order_and_page(
            groups,
            &sort_columns,
            stmt.limit,
            stmt.offset,
        )))
    }

    /// Scans all points, applies WHERE filter, and feeds each kept row to
    /// `grouper`.
    // Both storage guards are lent by reference into MatchStorageGuards and
    // read throughout the scan. Acquisition order is the decree recorded
    // below (2 then 3); see tests/tokio_blocking_lock_contention_repro.rs.
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
        // reversed here. See tests/tokio_blocking_lock_contention_repro.rs.
        let (metric, rows) = self.aggregate_config_snapshot();
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let match_guards =
            MatchStorageGuards::single(metric, rows, &vector_storage, &payload_storage);
        // Rows feed the grouper in id order, so a float SUM or AVG adds its
        // values in the same order on every run, whatever the storage order.
        let ids = Self::aggregate_scan_ids(
            RowScope::of_where(where_clause).universe(rows),
            &vector_storage,
            &payload_storage,
            filter.as_ref(),
        );
        let mut graph_cache = GraphMatchEvalCache::default();
        let now_secs = crate::collection::expiry::now_unix_secs();

        for (id, payload) in super::live_rows(&ids, &*payload_storage, now_secs) {
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
                let resolved = crate::velesql::bind_condition(cond, params)?;
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
