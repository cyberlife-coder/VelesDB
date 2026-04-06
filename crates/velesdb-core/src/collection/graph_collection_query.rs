//! VelesQL query execution for [`GraphCollection`].

use std::collections::HashMap;

use crate::collection::search::query::match_exec::MatchResult;
use crate::error::Result;
use crate::point::SearchResult;

use super::graph_collection::GraphCollection;

impl GraphCollection {
    /// Executes a parsed `VelesQL` query.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is invalid or execution fails.
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.execute_query(query, params)
    }

    /// Executes a query with instrumentation and returns plan + actual stats.
    ///
    /// Combines plan generation with execution, measuring wall-clock time
    /// and collecting per-node statistics.
    ///
    /// # Errors
    ///
    /// - Returns an error if the query is invalid or execution fails.
    pub fn explain_analyze_query(
        &self,
        query: &crate::velesql::Query,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<crate::velesql::ExplainOutput> {
        use crate::velesql::{build_leaf_node_stats, ActualStats, ExplainOutput, QueryPlan};

        let plan = if let Some(mc) = query.match_clause.as_ref() {
            let stats = crate::collection::search::query::match_planner::CollectionStats::default();
            QueryPlan::from_match(mc, &stats)
        } else {
            QueryPlan::from_select(&query.select)
        };

        let start = std::time::Instant::now();
        let results = self.inner.execute_query(query, params)?;
        let elapsed = start.elapsed();

        let actual_rows = results.len() as u64;
        let actual_time_ms = elapsed.as_secs_f64() * 1000.0;
        let is_match = query.is_match_query();
        let (nodes_visited, edges_traversed) = if is_match {
            (actual_rows, actual_rows)
        } else {
            (0, 0)
        };

        let stats = ActualStats {
            actual_rows,
            actual_time_ms,
            loops: 1,
            nodes_visited,
            edges_traversed,
        };

        let node_stats = build_leaf_node_stats(&plan.root, actual_rows, actual_time_ms);
        Ok(ExplainOutput::with_stats(plan, stats, node_stats))
    }

    /// Executes a raw VelesQL string, parsing it before execution.
    ///
    /// # Errors
    ///
    /// - Returns an error if the SQL string cannot be parsed.
    /// - Returns an error if query execution fails.
    pub fn execute_query_str(
        &self,
        sql: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        self.inner.execute_query_str(sql, params)
    }

    /// Executes a MATCH graph pattern query.
    ///
    /// # Errors
    ///
    /// Returns an error if the query cannot be executed.
    pub fn execute_match(
        &self,
        match_clause: &crate::velesql::MatchClause,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MatchResult>> {
        self.inner.execute_match(match_clause, params)
    }

    /// Executes a MATCH query with vector similarity scoring.
    ///
    /// # Errors
    ///
    /// Returns an error on dimension mismatch or execution failure.
    pub fn execute_match_with_similarity(
        &self,
        match_clause: &crate::velesql::MatchClause,
        query_vector: &[f32],
        similarity_threshold: f32,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MatchResult>> {
        self.inner.execute_match_with_similarity(
            match_clause,
            query_vector,
            similarity_threshold,
            params,
        )
    }
}
