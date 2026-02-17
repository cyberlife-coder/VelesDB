//! Cross-store execution strategies for combined Vector + Graph MATCH queries.
//!
//! Implements `VectorFirst` and Parallel strategies from `QueryPlanner`.
//! `GraphFirst` delegates to the existing `execute_match_with_similarity()`.
//!
//! # Strategies
//!
//! - **`VectorFirst`**: NEAR search (over-fetch) → graph MATCH validate → results
//! - **Parallel**: execute both V and G → fuse with RRF → results
//! - **`GraphFirst`**: delegates to `execute_match_with_similarity()`

use std::collections::HashMap;

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::MatchClause;

impl Collection {
    /// `VectorFirst` cross-store execution.
    ///
    /// 1. Execute vector NEAR search with over-fetch (2× limit)
    /// 2. For each candidate, validate against graph MATCH pattern
    /// 3. Return results sorted by vector score, truncated to limit
    ///
    /// # Errors
    ///
    /// Returns an error when vector search or MATCH execution fails.
    pub fn execute_vector_first_cross_store(
        &self,
        query_vector: &[f32],
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Over-fetch to compensate for graph filtering
        let over_fetch = limit.saturating_mul(3).max(20);
        let candidates = self.search(query_vector, over_fetch)?;

        // Execute graph MATCH to get the set of valid node IDs
        let match_results = self.execute_match(match_clause, params)?;
        let valid_ids: std::collections::HashSet<u64> = match_results
            .iter()
            .flat_map(|mr| {
                // Collect all node IDs from bindings (source and target nodes)
                let mut ids: Vec<u64> = mr.bindings.values().copied().collect();
                ids.push(mr.node_id);
                ids
            })
            .collect();

        // Filter vector candidates to only those in graph results
        let mut results: Vec<SearchResult> = candidates
            .into_iter()
            .filter(|r| valid_ids.contains(&r.point.id))
            .collect();

        // Already sorted by vector score from search()
        results.truncate(limit);
        Ok(results)
    }

    /// Parallel cross-store execution.
    ///
    /// 1. Execute vector search and graph MATCH independently
    /// 2. Fuse results using Reciprocal Rank Fusion (RRF)
    /// 3. Return fused results truncated to limit
    ///
    /// # Errors
    ///
    /// Returns an error when vector search, MATCH execution, or result conversion fails.
    pub fn execute_parallel_cross_store(
        &self,
        query_vector: &[f32],
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Execute both searches
        let vector_results = self.search(query_vector, limit.saturating_mul(2).max(20))?;
        let match_results = self.execute_match(match_clause, params)?;

        // Convert match results to search results for fusion
        let graph_results = self.match_results_to_search_results(match_results)?;

        // Fuse using RRF
        let vector_scored: Vec<crate::velesql::ScoredResult> = vector_results
            .iter()
            .map(|r| crate::velesql::ScoredResult::new(r.point.id, r.score))
            .collect();

        let graph_scored: Vec<crate::velesql::ScoredResult> = graph_results
            .iter()
            .map(|r| crate::velesql::ScoredResult::new(r.point.id, r.score))
            .collect();

        let rrf_config = crate::velesql::RrfConfig::default();
        let fused = crate::velesql::fuse_rrf(&vector_scored, &graph_scored, &rrf_config, limit);

        // Map fused results back to SearchResults
        // Build lookup from both result sets
        let mut result_map: HashMap<u64, SearchResult> = HashMap::new();
        for r in vector_results {
            result_map.entry(r.point.id).or_insert(r);
        }
        for r in graph_results {
            result_map.entry(r.point.id).or_insert(r);
        }

        let mut results: Vec<SearchResult> = fused
            .into_iter()
            .filter_map(|sr| {
                result_map.remove(&sr.id).map(|mut r| {
                    r.score = sr.score;
                    r
                })
            })
            .collect();

        results.truncate(limit);
        Ok(results)
    }
}
