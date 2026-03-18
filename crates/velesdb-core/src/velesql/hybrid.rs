//! Hybrid search combining graph patterns (MATCH) with vector similarity (NEAR).
//!
//! This module provides:
//! - RRF (Reciprocal Rank Fusion) for merging ranked result lists
//! - Weighted fusion for score-based merging
//! - Parallel and sequential execution strategies

// Hybrid fusion helpers used by VelesQL hybrid scoring tests and utility paths.
// Kept as a dedicated module to preserve deterministic fusion behavior.
#![allow(dead_code)]
// SAFETY: Numeric casts in result fusion are intentional:
// - usize->f32 for rank calculations: ranks are small (< 1M results)
// - u32->f32 for config values: k parameter is small (< 100)
// - Precision loss acceptable for score fusion heuristics
#![allow(clippy::cast_precision_loss)]

use std::collections::HashMap;

// Re-export the canonical ScoredResult from the crate root.
pub use crate::scored_result::ScoredResult;

/// Fusion strategy for combining search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FusionStrategy {
    /// Reciprocal Rank Fusion - recommended default.
    #[default]
    Rrf,
    /// Weighted sum of normalized scores.
    WeightedSum,
    /// Take maximum score from either source.
    Maximum,
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

/// Sorts `ScoredResult` values by score descending and truncates to `limit`.
///
/// Shared finalization step for all fusion strategies.
fn sort_descending_and_truncate(results: &mut Vec<ScoredResult>, limit: usize) {
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
}

/// Converts a score accumulator map into a sorted, truncated result vector.
fn finalize_scores(scores: HashMap<u64, f32>, limit: usize) -> Vec<ScoredResult> {
    let mut fused: Vec<ScoredResult> = scores
        .into_iter()
        .map(|(id, score)| ScoredResult::new(id, score))
        .collect();
    sort_descending_and_truncate(&mut fused, limit);
    fused
}

/// Accumulates RRF rank scores from a result list into the score map.
///
/// RRF score for rank `r` (1-based): `1 / (k + r)`
fn accumulate_rrf_scores(scores: &mut HashMap<u64, f32>, results: &[ScoredResult], k: f32) {
    for (rank, result) in results.iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank + 1) as f32);
        *scores.entry(result.id).or_insert(0.0) += rrf_score;
    }
}

/// Accumulates weighted scores from normalized results into the score map.
fn accumulate_weighted_scores(
    scores: &mut HashMap<u64, f32>,
    results: &[ScoredResult],
    weight: f32,
) {
    for result in results {
        *scores.entry(result.id).or_insert(0.0) += result.score * weight;
    }
}

/// Accumulates maximum scores from normalized results into the score map.
fn accumulate_max_scores(scores: &mut HashMap<u64, f32>, results: &[ScoredResult]) {
    for result in results {
        let entry = scores.entry(result.id).or_insert(0.0);
        *entry = entry.max(result.score);
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

    accumulate_rrf_scores(&mut scores, vector_results, k);
    accumulate_rrf_scores(&mut scores, graph_results, k);

    finalize_scores(scores, limit)
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
    let vector_normalized = normalize_scores(vector_results);
    let graph_normalized = normalize_scores(graph_results);

    let mut scores: HashMap<u64, f32> = HashMap::new();
    accumulate_weighted_scores(&mut scores, &vector_normalized, config.vector_weight);
    accumulate_weighted_scores(&mut scores, &graph_normalized, config.graph_weight);

    finalize_scores(scores, limit)
}

/// Fuses results by taking the maximum score from either source.
#[must_use]
pub fn fuse_maximum(
    vector_results: &[ScoredResult],
    graph_results: &[ScoredResult],
    limit: usize,
) -> Vec<ScoredResult> {
    let vector_normalized = normalize_scores(vector_results);
    let graph_normalized = normalize_scores(graph_results);

    let mut scores: HashMap<u64, f32> = HashMap::new();
    accumulate_max_scores(&mut scores, &vector_normalized);
    accumulate_max_scores(&mut scores, &graph_normalized);

    finalize_scores(scores, limit)
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
        .copied()
        .collect();

    let vector_ids: std::collections::HashSet<u64> = vector_results.iter().map(|r| r.id).collect();

    let filtered_graph: Vec<ScoredResult> = graph_results
        .iter()
        .filter(|r| vector_ids.contains(&r.id))
        .copied()
        .collect();

    (filtered_vector, filtered_graph)
}

// Tests moved to hybrid_tests.rs per project rules
