//! JOIN pushdown analysis helpers for database-level query execution.
//!
//! Extracted from `query_engine.rs` (Martin Fowler: Extract Module) to keep
//! file NLOC under 500. These functions classify WHERE conditions by data
//! source so the caller can route each filter to the correct execution stage.

use crate::collection::search::query::text_match::TextHits;
use crate::{Result, SearchResult};

use super::Database;

impl Database {
    /// Runs pushdown analysis on a SELECT statement's WHERE clause and JOINs.
    ///
    /// Returns the classified conditions so the caller can route each filter
    /// to the correct execution phase (pre-join, during-join, post-join).
    pub(super) fn analyze_join_pushdown_for_select(
        stmt: &crate::velesql::SelectStatement,
    ) -> crate::collection::search::query::pushdown::PushdownAnalysis {
        let join_tables =
            crate::collection::search::query::pushdown::pushable_join_tables(&stmt.joins);
        let graph_vars: std::collections::HashSet<String> =
            stmt.from_alias.iter().cloned().collect();
        let analysis = stmt.where_clause.as_ref().map_or_else(
            crate::collection::search::query::pushdown::PushdownAnalysis::default,
            |wc| {
                crate::collection::search::query::pushdown::analyze_for_pushdown(
                    wc,
                    &graph_vars,
                    &join_tables,
                )
            },
        );
        tracing::debug!(
            column_store = analysis.column_store_filters.len(),
            graph = analysis.graph_filters.len(),
            post_join = analysis.post_join_filters.len(),
            has_pushdown = analysis.has_pushdown(),
            "JOIN pushdown analysis"
        );
        analysis
    }

    /// The `filters` a JOIN's joined rows pass, each text `MATCH` bound to
    /// the BM25 hits of the side its column names ([`JoinSides`]) on that
    /// side's id, which a joined row carries under the side's alias
    /// (`velesql::merge_side`) — through the one membership routine every
    /// binding of a `MATCH` writes (`Collection::text_match_membership`),
    /// each side's hits for a query read once. `select` is the base query,
    /// whose FROM names the base side.
    ///
    /// # Errors
    ///
    /// When a joined table is no collection, or a `MATCH` qualifier names
    /// no side.
    pub(super) fn bind_post_join_text_matches(
        &self,
        base_collection: &crate::collection::Collection,
        select: &crate::velesql::SelectStatement,
        joins: &[crate::velesql::JoinClause],
        filters: Vec<crate::velesql::Condition>,
    ) -> Result<Vec<crate::velesql::Condition>> {
        use crate::velesql::text_match;
        if !filters.iter().any(text_match::holds_text_match) {
            return Ok(filters);
        }
        let sides = JoinSides::of(select, joins);
        let mut collections = vec![base_collection.clone()];
        for join in joins {
            collections.push(self.resolve_collection(&join.table)?);
        }
        let mut hits: Vec<TextHits> = collections.iter().map(|_| TextHits::default()).collect();
        let mut membership = |leaf: &crate::velesql::MatchCondition| {
            let side = sides.side_of(leaf)?;
            let column = format!("{}.id", sides.name(side));
            Ok(collections[side].text_match_membership(leaf, column, &mut hits[side]))
        };
        filters
            .iter()
            .map(|filter| text_match::bind_text_matches(filter, None, &mut membership))
            .collect()
    }

    /// `select`'s WHERE with each text `MATCH` read by the side of the JOIN
    /// its qualifier names: one qualified by the FROM side's name — its
    /// alias, or its table's when it has none — is the base's own, written
    /// unqualified so it stays in the base query, which ranks by it when the
    /// WHERE AND-requires it, as a `MATCH` with no qualifier does; one
    /// qualified by a joined side stays as written, a membership test of
    /// that side's hits on the joined row.
    ///
    /// # Errors
    ///
    /// A `MATCH` whose qualifier names no side ([`JoinSides::side_of`]).
    pub(super) fn resolve_join_text_matches(
        select: &mut crate::velesql::SelectStatement,
    ) -> Result<()> {
        let Some(condition) = select.where_clause.as_ref() else {
            return Ok(());
        };
        if !crate::velesql::text_match::holds_text_match(condition) {
            return Ok(());
        }
        let sides = JoinSides::of(select, &select.joins);
        let resolved = crate::velesql::text_match::map_text_matches(condition, &mut |leaf| {
            let mut leaf = leaf.clone();
            if sides.side_of(&leaf)? == JoinSides::BASE {
                if let Some(column) =
                    crate::velesql::base_column(&leaf.column, sides.name(JoinSides::BASE))
                {
                    leaf.column = column.to_string();
                }
            }
            Ok::<_, crate::Error>(crate::velesql::Condition::Match(leaf))
        })?;
        select.where_clause = Some(resolved);
        Ok(())
    }
    /// Applies post-join filters to merged results.
    ///
    /// Post-join filters are cross-source predicates that reference columns
    /// from both the base collection and joined tables. They can only be
    /// evaluated after the JOIN has merged payloads from both sides.
    pub(super) fn apply_post_join_filters(
        base_collection: &crate::collection::Collection,
        mut results: Vec<SearchResult>,
        post_join_filters: &[crate::velesql::Condition],
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
    ) -> Result<Vec<SearchResult>> {
        for filter in post_join_filters {
            results = base_collection.apply_where_condition_to_results(
                results,
                filter,
                params,
                from_aliases,
            )?;
        }
        Ok(results)
    }
}

/// The sides of a JOIN as its WHERE names them: the FROM side first, then
/// each joined one, each by its alias or — when it has none — its table's
/// name (`velesql::side_alias`). An aliased table's own name names no side,
/// as in SQL.
struct JoinSides<'s> {
    names: Vec<&'s str>,
}

impl<'s> JoinSides<'s> {
    /// The FROM side's position.
    const BASE: usize = 0;

    fn of(
        select: &'s crate::velesql::SelectStatement,
        joins: &'s [crate::velesql::JoinClause],
    ) -> Self {
        let base = crate::velesql::base_side(select);
        let joined = joins
            .iter()
            .map(|join| crate::velesql::side_alias(join.alias.as_deref(), &join.table));
        Self {
            names: std::iter::once(base).chain(joined).collect(),
        }
    }

    /// The name of the side at `side`.
    fn name(&self, side: usize) -> &'s str {
        self.names[side]
    }

    /// Which side's text index `leaf` reads: the one its qualifier names,
    /// the FROM side when it has none.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Query`] when the qualifier names no side: no text
    /// index answers it, and the base's must not in its place.
    fn side_of(&self, leaf: &crate::velesql::MatchCondition) -> Result<usize> {
        let Some((qualifier, _)) = leaf.column.split_once('.') else {
            return Ok(Self::BASE);
        };
        self.names
            .iter()
            .position(|name| *name == qualifier)
            .ok_or_else(|| {
                crate::Error::Query(format!(
                    "a text MATCH in a JOIN reads the text index of the side its column \
                     names, and '{qualifier}' names no side (the sides are: {})",
                    self.names.join(", ")
                ))
            })
    }
}
