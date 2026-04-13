//! JOIN pushdown analysis helpers for database-level query execution.
//!
//! Extracted from `query_engine.rs` (Martin Fowler: Extract Module) to keep
//! file NLOC under 500. These functions classify WHERE conditions by data
//! source so the caller can route each filter to the correct execution stage.

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
            crate::collection::search::query::pushdown::extract_join_tables(&stmt.joins);
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
