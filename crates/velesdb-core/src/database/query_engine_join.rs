//! The JOIN path of a SELECT: the base rows, filtered by the predicates on
//! the base collection alone, joined side by side, then filtered by every
//! predicate that reads a join side and windowed (DISTINCT, ORDER BY, OFFSET,
//! LIMIT) as joined rows.
//!
//! Extracted from `query_engine.rs` (file line budget).

use crate::{Result, SearchResult};

use super::Database;

impl Database {
    /// A SELECT with JOINs (`single_query`, its compound removed; `select`
    /// as written): its joined rows, windowed. As in SQL, DISTINCT, ORDER BY,
    /// OFFSET and LIMIT window the joined rows, so the base query runs
    /// without them ([`JoinWindow`]).
    pub(super) fn execute_joined_select(
        &self,
        mut single_query: crate::velesql::Query,
        select: &crate::velesql::SelectStatement,
        base_collection: &crate::collection::Collection,
        params: &std::collections::HashMap<String, serde_json::Value>,
        probe: Option<&crate::database::StrategyProbeSlot>,
    ) -> Result<Vec<SearchResult>> {
        let window = JoinWindow::take(&mut single_query.select);
        let rows = self.joined_rows(single_query, select, params, probe)?;
        window.apply(rows, &select.columns, base_collection, params)
    }

    /// The joined rows of `query` (a SELECT with JOINs, its window removed),
    /// WHERE applied, in base order. `budget_select` is the SELECT as written,
    /// whose window bounds how many joined rows are needed.
    pub(super) fn joined_rows(
        &self,
        mut query: crate::velesql::Query,
        budget_select: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        probe: Option<&crate::database::StrategyProbeSlot>,
    ) -> Result<Vec<SearchResult>> {
        let base_collection = self.resolve_collection(&query.select.from)?;
        let joins = query.select.joins.clone();
        let analysis = Self::prepare_join_pushdown(&mut query, params)?;
        let row_budget = Self::join_row_budget(budget_select, &analysis);
        let ceiling = crate::collection::search::query::JOIN_ROW_CEILING;
        let bounded = row_budget < ceiling;
        // A bounded window keeps the base order, so the first `row_budget`
        // joined rows are the answer: fetch that many base rows, more while
        // an INNER join or a filter drops some.
        let mut fetch = if bounded { row_budget.max(1) } else { ceiling };
        loop {
            query.select.limit = Some(u64::try_from(fetch).unwrap_or(u64::MAX));
            let base = match probe {
                Some(slot) => base_collection.execute_query_probed(&query, params, slot)?,
                None => base_collection.execute_query(&query, params)?,
            };
            let exhausted = base.len() < fetch || fetch >= ceiling;
            let joined = self.join_base_rows(
                base,
                &JoinPlan {
                    select: budget_select,
                    joins: &joins,
                    analysis: &analysis,
                    row_budget,
                    collection: &base_collection,
                    params,
                },
            )?;
            if !bounded || joined.len() >= row_budget || exhausted {
                return Ok(joined);
            }
            fetch = fetch.saturating_mul(2).min(ceiling);
        }
    }

    /// Joins the base rows of a SELECT: each scoped under the FROM alias
    /// (`velesql::scope_base`), every JOIN in turn, then the cross-source
    /// filters.
    fn join_base_rows(
        &self,
        base: Vec<SearchResult>,
        plan: &JoinPlan<'_>,
    ) -> Result<Vec<SearchResult>> {
        let select = plan.select;
        let from_alias =
            crate::velesql::side_alias(select.from_alias.first().map(String::as_str), &select.from);
        let mut results: Vec<SearchResult> = base
            .into_iter()
            .map(|mut r| {
                r.point.payload = Some(crate::velesql::scope_base(
                    r.point.payload.take(),
                    from_alias,
                    r.point.id,
                ));
                r
            })
            .collect();
        for join in plan.joins {
            // Each join side is filtered by the pushed predicates on ITS
            // columns only: another side's predicate would empty its store.
            let sides = crate::collection::search::query::pushdown::extract_join_tables(
                std::slice::from_ref(join),
            );
            let pushed: Vec<crate::velesql::Condition> = plan
                .analysis
                .column_store_filters
                .iter()
                .filter(|c| crate::collection::search::query::pushdown::reads_only(c, &sides))
                .cloned()
                .collect();
            results = self.execute_single_join(results, join, &pushed, plan.row_budget)?;
        }
        // Every predicate that reads a join side, on the joined rows.
        let joined_row_filters = plan.analysis.joined_row_filters();
        if !joined_row_filters.is_empty() {
            results = Self::apply_post_join_filters(
                plan.collection,
                results,
                &joined_row_filters,
                plan.params,
                &select.from_alias,
            )?;
        }
        Ok(results)
    }

    /// Resolves WHERE parameters, runs pushdown analysis, and strips from the
    /// base query every predicate the joined rows answer
    /// ([`PushdownAnalysis::joined_row_filters`]).
    ///
    /// [`PushdownAnalysis::joined_row_filters`]: crate::collection::search::query::pushdown::PushdownAnalysis::joined_row_filters
    ///
    /// Parameter placeholders are resolved before the analysis so pushed-down
    /// filters never silently convert them to NULL at the `ColumnStore` layer
    /// (the collection pipeline resolves its own copy independently).
    fn prepare_join_pushdown(
        single_query: &mut crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<crate::collection::search::query::pushdown::PushdownAnalysis> {
        if let Some(cond) = single_query.select.where_clause.take() {
            single_query.select.where_clause = Some(crate::velesql::bind_condition(&cond, params)?);
        }

        let analysis = Self::analyze_join_pushdown_for_select(&single_query.select);

        // The base rows are filtered by the predicates on the base collection
        // only; the rest run on the joined rows (`join_base_rows`).
        let resolved_where = single_query.select.where_clause.clone();
        single_query.select.joins.clear();
        let joined_row_filters = analysis.joined_row_filters();
        if !joined_row_filters.is_empty() {
            single_query.select.where_clause =
                Self::strip_pushed_conditions(resolved_where.as_ref(), &joined_row_filters);
        }
        Ok(analysis)
    }

    /// Computes the bound on joined rows to materialize.
    ///
    /// When the query has an explicit LIMIT, no post-join filters, and no
    /// ORDER BY (which could reorder past the window), the bound is the
    /// effective `LIMIT + OFFSET`. GROUP BY / HAVING / DISTINCT also disqualify
    /// the bounded shape: SQL LIMIT bounds output *groups/rows*, not input rows,
    /// so truncating joined input to `LIMIT` would drop rows that belong to
    /// groups still inside the window. Otherwise downstream stages may reorder or
    /// drop rows, so we fall back to the conservative server-side ceiling
    /// [`JOIN_ROW_CEILING`] — still bounding OOM without affecting correctness.
    ///
    /// [`JOIN_ROW_CEILING`]: crate::collection::search::query::JOIN_ROW_CEILING
    pub(super) fn join_row_budget(
        select: &crate::velesql::SelectStatement,
        analysis: &crate::collection::search::query::pushdown::PushdownAnalysis,
    ) -> usize {
        use crate::collection::search::query::JOIN_ROW_CEILING;
        use crate::velesql::DistinctMode;
        let bounded_shape = analysis.post_join_filters.is_empty()
            && select.order_by.is_none()
            && select.group_by.is_none()
            && select.having.is_none()
            && select.distinct == DistinctMode::None;
        match select.limit {
            Some(limit) if bounded_shape => {
                let limit = usize::try_from(limit).unwrap_or(JOIN_ROW_CEILING);
                let offset = select
                    .offset
                    .map_or(0, |o| usize::try_from(o).unwrap_or(JOIN_ROW_CEILING));
                limit.saturating_add(offset).min(JOIN_ROW_CEILING)
            }
            _ => JOIN_ROW_CEILING,
        }
    }
}

/// What joining a SELECT's base rows reads ([`Database::joined_rows`]).
struct JoinPlan<'a> {
    /// The SELECT as written (its FROM alias, its window).
    select: &'a crate::velesql::SelectStatement,
    joins: &'a [crate::velesql::JoinClause],
    analysis: &'a crate::collection::search::query::pushdown::PushdownAnalysis,
    /// Joined rows needed at most.
    row_budget: usize,
    /// The base collection, which evaluates the cross-source filters.
    collection: &'a crate::collection::Collection,
    params: &'a std::collections::HashMap<String, serde_json::Value>,
}

/// The clauses a JOIN applies to its joined rows rather than to the base
/// collection: DISTINCT, ORDER BY, OFFSET, LIMIT (the SELECT default when
/// absent).
struct JoinWindow {
    distinct: crate::velesql::DistinctMode,
    order_by: Option<Vec<crate::velesql::SelectOrderBy>>,
    offset: Option<u64>,
    limit: Option<u64>,
}

impl JoinWindow {
    /// Moves the window out of `select`, leaving the base query unwindowed.
    fn take(select: &mut crate::velesql::SelectStatement) -> Self {
        Self {
            distinct: std::mem::take(&mut select.distinct),
            order_by: select.order_by.take(),
            offset: select.offset.take(),
            limit: select.limit.take(),
        }
    }

    /// The joined rows, deduplicated, ordered and windowed.
    fn apply(
        self,
        mut rows: Vec<SearchResult>,
        columns: &crate::velesql::SelectColumns,
        collection: &crate::collection::Collection,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        if self.distinct == crate::velesql::DistinctMode::All {
            rows = crate::velesql::distinct_evaluator::apply_distinct(rows, columns);
        }
        if let Some(order_by) = &self.order_by {
            collection.apply_order_by(&mut rows, order_by, params)?;
        }
        let offset = usize::try_from(self.offset.unwrap_or(0)).unwrap_or(usize::MAX);
        let limit = usize::try_from(self.limit.unwrap_or(crate::velesql::DEFAULT_SELECT_LIMIT))
            .unwrap_or(usize::MAX);
        Ok(rows.into_iter().skip(offset).take(limit).collect())
    }
}
