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
