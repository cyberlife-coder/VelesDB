//! Hybrid search combining graph patterns (MATCH) with vector similarity (NEAR).
//!
//! This module provides:
//! - RRF (Reciprocal Rank Fusion) for merging ranked result lists
//! - Weighted fusion for score-based merging
//! - Parallel and sequential execution strategies

// VP-010: Functions now actively used via re-exports in velesql/mod.rs
// and wired into cross-store query planner integration.

// SAFETY: Numeric casts in result fusion are intentional:
// - usize->f32 for rank calculations: ranks are small (< 1M results)
// - u32->f32 for config values: k parameter is small (< 100)
// - Precision loss acceptable for score fusion heuristics
#![allow(clippy::cast_precision_loss)]

use std::collections::HashMap;

/// A search result with ID and score.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredResult {
    /// Unique identifier of the result.
    pub id: u64,
    /// Score (higher is better).
    pub score: f32,
}

impl ScoredResult {
    /// Creates a new scored result.
    #[must_use]
    pub fn new(id: u64, score: f32) -> Self {
        Self { id, score }
    }
}

/// Configuration for RRF fusion.
#[derive(Debug, Clone)]
pub struct RrfConfig {
    /// The k parameter in RRF formula: 1/(k + rank).
    /// Default is 60 (standard value).
    pub k: u32,
}

impl Default for RrfConfig {
    fn default() -> Self {
        Self { k: 60 }
    }
}

impl RrfConfig {
    /// Creates RRF config with custom k parameter.
    #[must_use]
    pub fn with_k(k: u32) -> Self {
        Self { k }
    }
}

/// Configuration for weighted sum fusion.
#[derive(Debug, Clone)]
pub struct WeightedConfig {
    /// Weight for vector search results (0.0-1.0).
    pub vector_weight: f32,
    /// Weight for graph search results (0.0-1.0).
    pub graph_weight: f32,
}

impl Default for WeightedConfig {
    fn default() -> Self {
        Self {
            vector_weight: 0.5,
            graph_weight: 0.5,
        }
    }
}

impl WeightedConfig {
    /// Creates weighted config with custom weights.
    #[must_use]
    pub fn new(vector_weight: f32, graph_weight: f32) -> Self {
        Self {
            vector_weight,
            graph_weight,
        }
    }
}

/// Fuses two ranked result lists using Reciprocal Rank Fusion.
///
/// RRF formula: `score(d) = Σ 1/(k + rank_i(d))`
///
/// This method is preferred because:
/// - No score normalization needed
/// - Works well with heterogeneous result sources
/// - Robust to outliers
///
/// # Arguments
///
/// * `vector_results` - Results from vector similarity search (ranked by score).
/// * `graph_results` - Results from graph pattern matching (ranked by relevance).
/// * `config` - RRF configuration (k parameter).
/// * `limit` - Maximum number of results to return.
///
/// # Returns
///
/// Fused results sorted by combined RRF score (descending).
#[must_use]
pub fn fuse_rrf(
    vector_results: &[ScoredResult],
    graph_results: &[ScoredResult],
    config: &RrfConfig,
    limit: usize,
) -> Vec<ScoredResult> {
    let mut scores: HashMap<u64, f32> = HashMap::new();
    let k = config.k as f32;

    // Add RRF scores from vector results
    for (rank, result) in vector_results.iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank + 1) as f32);
        *scores.entry(result.id).or_insert(0.0) += rrf_score;
    }

    // Add RRF scores from graph results
    for (rank, result) in graph_results.iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank + 1) as f32);
        *scores.entry(result.id).or_insert(0.0) += rrf_score;
    }

    // Convert to sorted vector
    let mut fused: Vec<ScoredResult> = scores
        .into_iter()
        .map(|(id, score)| ScoredResult::new(id, score))
        .collect();

    // Sort by score descending
    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit results
    fused.truncate(limit);
    fused
}

/// Fuses two result lists using weighted sum of normalized scores.
///
/// Scores are first normalized to [0, 1] range, then combined:
/// `final_score = α * vector_score + (1-α) * graph_score`
///
/// # Arguments
///
/// * `vector_results` - Results from vector similarity search.
/// * `graph_results` - Results from graph pattern matching.
/// * `config` - Weight configuration.
/// * `limit` - Maximum number of results to return.
#[must_use]
pub fn fuse_weighted(
    vector_results: &[ScoredResult],
    graph_results: &[ScoredResult],
    config: &WeightedConfig,
    limit: usize,
) -> Vec<ScoredResult> {
    // Normalize scores to [0, 1]
    let vector_normalized = normalize_scores(vector_results);
    let graph_normalized = normalize_scores(graph_results);

    // Build score map
    let mut scores: HashMap<u64, f32> = HashMap::new();

    for result in &vector_normalized {
        *scores.entry(result.id).or_insert(0.0) += result.score * config.vector_weight;
    }

    for result in &graph_normalized {
        *scores.entry(result.id).or_insert(0.0) += result.score * config.graph_weight;
    }

    // Convert to sorted vector
    let mut fused: Vec<ScoredResult> = scores
        .into_iter()
        .map(|(id, score)| ScoredResult::new(id, score))
        .collect();

    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fused.truncate(limit);
    fused
}

/// Fuses results by taking the maximum score from either source.
#[must_use]
pub fn fuse_maximum(
    vector_results: &[ScoredResult],
    graph_results: &[ScoredResult],
    limit: usize,
) -> Vec<ScoredResult> {
    // Normalize first
    let vector_normalized = normalize_scores(vector_results);
    let graph_normalized = normalize_scores(graph_results);

    let mut scores: HashMap<u64, f32> = HashMap::new();

    for result in &vector_normalized {
        let entry = scores.entry(result.id).or_insert(0.0);
        *entry = entry.max(result.score);
    }

    for result in &graph_normalized {
        let entry = scores.entry(result.id).or_insert(0.0);
        *entry = entry.max(result.score);
    }

    let mut fused: Vec<ScoredResult> = scores
        .into_iter()
        .map(|(id, score)| ScoredResult::new(id, score))
        .collect();

    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fused.truncate(limit);
    fused
}

/// Normalizes scores to [0, 1] range using min-max normalization.
pub(crate) fn normalize_scores(results: &[ScoredResult]) -> Vec<ScoredResult> {
    if results.is_empty() {
        return Vec::new();
    }

    let min_score = results
        .iter()
        .map(|r| r.score)
        .fold(f32::INFINITY, f32::min);
    let max_score = results
        .iter()
        .map(|r| r.score)
        .fold(f32::NEG_INFINITY, f32::max);

    let range = max_score - min_score;

    if range.abs() < f32::EPSILON {
        // All scores are the same
        return results
            .iter()
            .map(|r| ScoredResult::new(r.id, 1.0))
            .collect();
    }

    results
        .iter()
        .map(|r| ScoredResult::new(r.id, (r.score - min_score) / range))
        .collect()
}

/// Filters results to only include IDs present in both sources.
/// Useful for "AND" semantics where both conditions must match.
#[must_use]
pub fn intersect_results(
    vector_results: &[ScoredResult],
    graph_results: &[ScoredResult],
) -> (Vec<ScoredResult>, Vec<ScoredResult>) {
    let graph_ids: std::collections::HashSet<u64> = graph_results.iter().map(|r| r.id).collect();

    let filtered_vector: Vec<ScoredResult> = vector_results
        .iter()
        .filter(|r| graph_ids.contains(&r.id))
        .cloned()
        .collect();

    let vector_ids: std::collections::HashSet<u64> = vector_results.iter().map(|r| r.id).collect();

    let filtered_graph: Vec<ScoredResult> = graph_results
        .iter()
        .filter(|r| vector_ids.contains(&r.id))
        .cloned()
        .collect();

    (filtered_vector, filtered_graph)
}

// Tests moved to hybrid_tests.rs per project rules
