//! The JOIN path of a SELECT: the base rows, filtered by the predicates on
//! the base collection alone, joined side by side, then filtered by every
//! predicate that reads a join side and windowed (DISTINCT, ORDER BY, OFFSET,
//! LIMIT) as joined rows.
//!
//! Extracted from `query_engine.rs` (file line budget).

use crate::{Result, SearchResult};

use super::Database;

#[cfg(test)]
#[path = "join_clock_tests.rs"]
mod clock_tests;

/// The leading collection owns the final output quota of a statement containing a JOIN.
pub(super) struct JoinedRead<'a> {
    statement: &'a crate::velesql::Query,
    ctx: crate::guardrails::QueryContext,
}

impl<'a> JoinedRead<'a> {
    pub(super) fn begin(
        query: &'a crate::velesql::Query,
        leading: &crate::collection::Collection,
    ) -> Option<Self> {
        let other_selects = query
            .compound
            .iter()
            .flat_map(|set| &set.operations)
            .map(|(_, select)| select);
        if std::iter::once(&query.select)
            .chain(other_selects)
            .all(|select| select.joins.is_empty())
        {
            return None;
        }
        Some(Self {
            statement: query,
            ctx: leading.guard_rails().create_context(),
        })
    }

    /// Every operand supplies candidates until the outer JOIN or set operation has its page.
    fn operand(
        &self,
        db: &Database,
        query: &crate::velesql::Query,
        collection: &crate::collection::Collection,
        params: &std::collections::HashMap<String, serde_json::Value>,
        probe: Option<&crate::database::StrategyProbeSlot>,
    ) -> Result<Vec<SearchResult>> {
        let mut ctx = collection
            .prepare_query_context(query, "default")?
            .for_join_input();
        ctx.window_rank_inputs =
            crate::velesql::compound_rank_inputs(self.statement, &query.select.columns);
        let rows = if query.select.joins.is_empty() {
            collection.execute_in_query_context(query, params, &ctx)?.0
        } else {
            db.execute_joined_select(query.clone(), &query.select, collection, params, &ctx)?
        };
        if query.select.from != self.statement.select.from {
            collection.guard_rails().circuit_breaker.record_success();
        }
        if let (Some(slot), Some(strategy)) = (probe, ctx.executed_filter_strategy()) {
            slot.record(strategy);
        }
        Ok(rows)
    }

    pub(super) fn finish(
        &self,
        collection: &crate::collection::Collection,
        returned: usize,
    ) -> Result<()> {
        collection.finish_returned_rows(&self.ctx, returned)
    }
}

impl Database {
    /// Runs one operand against its base collection under the statement's output window.
    pub(super) fn execute_select_operand(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
        probe: Option<&crate::database::StrategyProbeSlot>,
        leading: (&crate::velesql::Query, &crate::collection::Collection),
        joined: Option<&JoinedRead<'_>>,
    ) -> Result<Vec<SearchResult>> {
        let other;
        let collection = if query.select.from == leading.0.select.from {
            leading.1
        } else {
            other = self.resolve_collection(&query.select.from)?;
            &other
        };
        if let Some(joined) = joined {
            return joined.operand(self, query, collection, params, probe);
        }
        match probe {
            Some(slot) => collection.execute_query_probed(query, params, slot),
            None if leading.0.compound.is_some() => collection.execute_set_operand(
                query,
                params,
                crate::velesql::compound_rank_inputs(leading.0, &query.select.columns),
            ),
            None => collection.execute_query(query, params),
        }
    }

    /// A SELECT with JOINs (`single_query`, its compound removed; `select`
    /// as written): its joined rows, windowed. As in SQL, DISTINCT, ORDER BY,
    /// OFFSET and LIMIT window the joined rows, so the base query runs
    /// without them ([`JoinWindow`]).
    fn execute_joined_select(
        &self,
        mut single_query: crate::velesql::Query,
        select: &crate::velesql::SelectStatement,
        base_collection: &crate::collection::Collection,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let deferred_let = defer_join_windows(&mut single_query);
        let window = JoinWindow::take(&mut single_query.select);
        let validate_let = deferred_let
            .as_ref()
            .is_some_and(|bindings| !bindings.is_empty());
        let rows = self.joined_rows(
            single_query,
            select,
            base_collection,
            params,
            ctx,
            validate_let,
        )?;
        let rows = window.apply(
            rows,
            select,
            base_collection,
            params,
            deferred_let.as_deref(),
            &ctx.window_rank_inputs,
        )?;
        base_collection.check_read_deadline(ctx)?;
        Ok(rows)
    }

    /// The joined rows of `query` (a SELECT with JOINs, its window removed),
    /// WHERE applied, in base order. `budget_select` is the SELECT as written,
    /// whose window bounds how many joined rows are needed.
    pub(super) fn joined_rows(
        &self,
        mut query: crate::velesql::Query,
        budget_select: &crate::velesql::SelectStatement,
        base_collection: &crate::collection::Collection,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
        validate_deferred_let: bool,
    ) -> Result<Vec<SearchResult>> {
        let joins = query.select.joins.clone();
        let (analysis, post_join_filters) = self.prepare_join_filters(
            &mut query,
            base_collection,
            &joins,
            params,
            validate_deferred_let,
        )?;
        let row_budget = Self::join_row_budget(budget_select, &post_join_filters);
        // A RIGHT or FULL join tells an unmatched joined point only against
        // every base row, so it reads them all at once.
        let mut fetch = BaseFetch::new(if keeps_unmatched_side_rows(&joins) {
            BaseFetch::CEILING
        } else {
            row_budget
        });
        let plan = JoinPlan {
            select: budget_select,
            joins: &joins,
            analysis: &analysis,
            post_join_filters: &post_join_filters,
            row_budget,
            collection: base_collection,
            params,
        };
        loop {
            let base = Self::base_rows(base_collection, &mut query, fetch.rows, params, ctx)?;
            let base_len = base.len();
            let joined = self.join_base_rows(base, &plan)?;
            if fetch.is_enough(base_len, joined.len()) {
                return Ok(joined);
            }
            fetch.grow();
        }
    }

    /// The first `fetch` base rows of `query` (its joins and join-side
    /// predicates removed), under the statement's JOIN context.
    fn base_rows(
        collection: &crate::collection::Collection,
        query: &mut crate::velesql::Query,
        fetch: usize,
        params: &std::collections::HashMap<String, serde_json::Value>,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        query.select.limit = Some(u64::try_from(fetch).unwrap_or(u64::MAX));
        crate::velesql::QueryValidator::validate(query)?;
        collection
            .execute_in_query_context(query, params, ctx)
            .map(|(rows, _)| rows)
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
        let from_alias = crate::velesql::base_side(select);
        let mut results = scope_base_rows(base, from_alias);
        let mut prior_aliases = vec![from_alias];
        let mut after_outer_join = false;
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
            let step = JoinStep {
                pushed: &pushed,
                row_budget: plan.row_budget,
                prior_aliases: &prior_aliases,
                after_outer_join,
            };
            results = self.execute_single_join(results, join, &step)?;
            prior_aliases.push(crate::velesql::side_alias(
                join.alias.as_deref(),
                &join.table,
            ));
            after_outer_join |= keeps_unmatched_side_rows(std::slice::from_ref(join));
        }
        if !plan.post_join_filters.is_empty() {
            results = Self::apply_post_join_filters(
                plan.collection,
                results,
                plan.post_join_filters,
                plan.params,
                &select.from_alias,
            )?;
        }
        Ok(results)
    }

    /// Prepare the predicates for each side and validate any LET that will run
    /// on the joined rows against the same base query that normally checks it.
    fn prepare_join_filters(
        &self,
        query: &mut crate::velesql::Query,
        base: &crate::collection::Collection,
        joins: &[crate::velesql::JoinClause],
        params: &std::collections::HashMap<String, serde_json::Value>,
        validate_deferred_let: bool,
    ) -> Result<(
        crate::collection::search::query::pushdown::PushdownAnalysis,
        Vec<crate::velesql::Condition>,
    )> {
        let (analysis, filters) = Self::prepare_join_pushdown(query, params)?;
        if validate_deferred_let {
            crate::collection::Collection::validate_deferred_let(&query.select, params)?;
        }
        let filters = self.bind_post_join_text_matches(base, &query.select, joins, filters)?;
        Ok((analysis, filters))
    }

    /// Resolves which side each text `MATCH` reads
    /// ([`Self::resolve_join_text_matches`]) and the WHERE parameters, runs
    /// pushdown analysis, and strips from the base query every predicate the
    /// joined rows answer
    /// ([`PushdownAnalysis::joined_row_filters`]).
    ///
    /// [`PushdownAnalysis::joined_row_filters`]: crate::collection::search::query::pushdown::PushdownAnalysis::joined_row_filters
    ///
    /// All sides and retries read the same bound parameters and `NOW()` instant.
    fn prepare_join_pushdown(
        single_query: &mut crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(
        crate::collection::search::query::pushdown::PushdownAnalysis,
        Vec<crate::velesql::Condition>,
    )> {
        Self::resolve_join_text_matches(&mut single_query.select)?;
        if let Some(cond) = single_query.select.where_clause.take() {
            single_query.select.where_clause =
                Some(crate::velesql::bind_query_condition(&cond, params)?);
        }

        let analysis = Self::analyze_join_pushdown_for_select(&single_query.select);

        // The base rows are filtered by the predicates on the base collection
        // only; the rest run on the joined rows (`join_base_rows`). Below a
        // RIGHT or FULL join a base predicate would hide the base row a joined
        // point matches, so the whole WHERE holds on the joined rows there.
        let resolved_where = single_query.select.where_clause.clone();
        let outer = keeps_unmatched_side_rows(&single_query.select.joins);
        let base = crate::velesql::base_side(&single_query.select).to_string();
        single_query.select.joins.clear();
        if outer {
            single_query.select.where_clause = None;
            return Ok((analysis, resolved_where.into_iter().collect()));
        }
        let joined_row_filters = analysis.joined_row_filters();
        let base_where = if joined_row_filters.is_empty() {
            resolved_where
        } else {
            Self::strip_pushed_conditions(resolved_where.as_ref(), &joined_row_filters)
        };
        // What stays on the base query reads the base rows, before any join:
        // a column the FROM side's name qualifies is theirs.
        single_query.select.where_clause =
            base_where.map(|condition| crate::velesql::resolve_base_qualifiers(&condition, &base));
        Ok((analysis, joined_row_filters))
    }

    /// Computes the bound on joined rows to materialize.
    ///
    /// When the query has an explicit LIMIT, no post-join filters, and no
    /// ORDER BY (which could reorder past the window), the bound is the
    /// effective `LIMIT + OFFSET`. GROUP BY / HAVING / DISTINCT and window functions also disqualify
    /// the bounded shape: SQL LIMIT bounds output *groups/rows*, not input rows,
    /// so truncating joined input to `LIMIT` would drop rows that belong to
    /// groups still inside the window. Otherwise downstream stages may reorder or
    /// drop rows, so we fall back to the conservative server-side ceiling
    /// [`JOIN_ROW_CEILING`] — still bounding OOM without affecting correctness.
    ///
    /// [`JOIN_ROW_CEILING`]: crate::collection::search::query::JOIN_ROW_CEILING
    pub(super) fn join_row_budget(
        select: &crate::velesql::SelectStatement,
        post_join_filters: &[crate::velesql::Condition],
    ) -> usize {
        use crate::collection::search::query::JOIN_ROW_CEILING;
        let bounded_shape = post_join_filters.is_empty()
            && select.order_by.is_none()
            && select.group_by.is_none()
            && select.having.is_none()
            && !select.reshapes_rows_before_window();
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
    /// The predicates that hold on the joined rows.
    post_join_filters: &'a [crate::velesql::Condition],
    /// Joined rows needed at most.
    row_budget: usize,
    /// The base collection, which evaluates the cross-source filters.
    collection: &'a crate::collection::Collection,
    params: &'a std::collections::HashMap<String, serde_json::Value>,
}

/// What one JOIN of a chain reads besides the rows and the clause
/// ([`Database::execute_single_join`]).
pub(super) struct JoinStep<'a> {
    /// The pushed predicates on this join's side.
    pub(super) pushed: &'a [crate::velesql::Condition],
    /// Joined rows needed at most.
    pub(super) row_budget: usize,
    /// The sides the rows already carry, the FROM side first.
    pub(super) prior_aliases: &'a [&'a str],
    /// Whether an earlier join of the chain was RIGHT or FULL: its rows for
    /// unmatched points carry the joined point's id, not a base id.
    pub(super) after_outer_join: bool,
}

/// The base rows of a JOIN, each one's payload scoped under the FROM side's
/// name `alias` (`velesql::scope_base`).
fn scope_base_rows(base: Vec<SearchResult>, alias: &str) -> Vec<SearchResult> {
    base.into_iter()
        .map(|mut row| {
            let id = row.point.id;
            row.point.payload = Some(crate::velesql::scope_base(
                row.point.payload.take(),
                alias,
                id,
            ));
            row
        })
        .collect()
}

/// Whether any of `joins` is RIGHT or FULL: one that adds rows for the joined
/// points no base row matched.
fn keeps_unmatched_side_rows(joins: &[crate::velesql::JoinClause]) -> bool {
    use crate::velesql::JoinType;
    joins
        .iter()
        .any(|join| matches!(join.join_type, JoinType::Right | JoinType::Full))
}

/// Window partitions consist of the joined rows admitted by WHERE and
/// DISTINCT. Keep their SELECT outputs and dependent LETs off the base scan.
fn defer_join_windows(
    query: &mut crate::velesql::Query,
) -> Option<Vec<crate::velesql::LetBinding>> {
    let crate::velesql::SelectColumns::Mixed {
        window_functions, ..
    } = &mut query.select.columns
    else {
        return None;
    };
    if window_functions.is_empty() {
        return None;
    }
    window_functions.clear();
    Some(std::mem::take(&mut query.let_bindings))
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
        select: &crate::velesql::SelectStatement,
        collection: &crate::collection::Collection,
        params: &std::collections::HashMap<String, serde_json::Value>,
        deferred_let: Option<&[crate::velesql::LetBinding]>,
        rank_inputs: &[String],
    ) -> Result<Vec<SearchResult>> {
        let limit = usize::try_from(self.limit.unwrap_or(crate::velesql::DEFAULT_SELECT_LIMIT))
            .unwrap_or(usize::MAX);
        if let Some(bindings) = deferred_let {
            return collection.apply_select_postprocessing(
                select,
                rows,
                params,
                limit,
                bindings,
                rank_inputs,
            );
        }
        if self.distinct == crate::velesql::DistinctMode::All {
            rows = crate::velesql::distinct_evaluator::apply_distinct(rows, &select.columns);
        }
        if let Some(order_by) = &self.order_by {
            collection.apply_order_by(&mut rows, order_by, params)?;
        }
        let offset = usize::try_from(self.offset.unwrap_or(0)).unwrap_or(usize::MAX);
        Ok(rows.into_iter().skip(offset).take(limit).collect())
    }
}

/// How many base rows the JOIN path fetches. A bounded window keeps the base
/// order, so the first `row_budget` joined rows are the answer: fetch that
/// many base rows, twice as many again while an INNER join or a filter drops
/// some, up to [`JOIN_ROW_CEILING`]. An unbounded window fetches the ceiling
/// at once.
///
/// [`JOIN_ROW_CEILING`]: crate::collection::search::query::JOIN_ROW_CEILING
struct BaseFetch {
    /// Base rows the next fetch asks for.
    rows: usize,
    /// Joined rows needed (the ceiling when unbounded).
    row_budget: usize,
}

impl BaseFetch {
    const CEILING: usize = crate::collection::search::query::JOIN_ROW_CEILING;

    fn new(row_budget: usize) -> Self {
        let rows = if row_budget < Self::CEILING {
            row_budget.max(1)
        } else {
            Self::CEILING
        };
        Self { rows, row_budget }
    }

    /// Whether the joined rows a fetch of `base_len` base rows gave are the
    /// answer: the window is unbounded, it is filled, or no base row is left.
    fn is_enough(&self, base_len: usize, joined_len: usize) -> bool {
        let unbounded = self.row_budget >= Self::CEILING;
        let exhausted = base_len < self.rows || self.rows >= Self::CEILING;
        unbounded || joined_len >= self.row_budget || exhausted
    }

    /// The next fetch: twice as many base rows, up to the ceiling.
    fn grow(&mut self) {
        self.rows = self.rows.saturating_mul(2).min(Self::CEILING);
    }
}
