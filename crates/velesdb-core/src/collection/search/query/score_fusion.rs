//! Multi-Score Fusion for hybrid search results (EPIC-049).
//!
//! This module provides score breakdown and combination strategies
//! for combining vector similarity, graph distance, and metadata boosts.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Score breakdown showing individual components of a result's score (EPIC-049 US-001).
///
/// This structure allows developers to understand why a result is ranked
/// at a particular position by exposing all contributing score factors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    /// Vector similarity score (0-1 for cosine, unbounded for others).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_similarity: Option<f32>,

    /// Graph distance score (normalized 0-1, where 1 = directly connected).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_distance: Option<f32>,

    /// Path relevance score (based on relationship types traversed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_score: Option<f32>,

    /// Metadata boost factor (multiplicative, default 1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_boost: Option<f32>,

    /// Recency boost (time decay factor).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recency_boost: Option<f32>,

    /// Custom boost factors (extensible).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_boosts: HashMap<String, f32>,

    /// Final combined score after fusion.
    pub final_score: f32,
}

impl ScoreBreakdown {
    /// Creates a new empty score breakdown.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a score breakdown with only vector similarity.
    #[must_use]
    pub fn from_vector(similarity: f32) -> Self {
        Self {
            vector_similarity: Some(similarity),
            final_score: similarity,
            ..Default::default()
        }
    }

    /// Creates a score breakdown with only graph distance.
    #[must_use]
    pub fn from_graph(distance: f32) -> Self {
        Self {
            graph_distance: Some(distance),
            final_score: distance,
            ..Default::default()
        }
    }

    /// Builder: set vector similarity.
    #[must_use]
    pub fn with_vector(mut self, score: f32) -> Self {
        self.vector_similarity = Some(score);
        self
    }

    /// Builder: set graph distance.
    #[must_use]
    pub fn with_graph(mut self, score: f32) -> Self {
        self.graph_distance = Some(score);
        self
    }

    /// Builder: set path score.
    #[must_use]
    pub fn with_path(mut self, score: f32) -> Self {
        self.path_score = Some(score);
        self
    }

    /// Builder: set metadata boost.
    #[must_use]
    pub fn with_metadata_boost(mut self, boost: f32) -> Self {
        self.metadata_boost = Some(boost);
        self
    }

    /// Builder: set recency boost.
    #[must_use]
    pub fn with_recency_boost(mut self, boost: f32) -> Self {
        self.recency_boost = Some(boost);
        self
    }

    /// Builder: add a custom boost.
    #[must_use]
    pub fn with_custom_boost(mut self, name: impl Into<String>, boost: f32) -> Self {
        self.custom_boosts.insert(name.into(), boost);
        self
    }

    /// Compute final score using the specified strategy.
    pub fn compute_final(&mut self, strategy: &FusionStrategy) {
        self.final_score = strategy.combine(self);
    }

    /// Get all non-None scores as a vector of (name, value) pairs.
    #[must_use]
    pub fn components(&self) -> Vec<(&'static str, f32)> {
        let mut components = Vec::new();

        if let Some(v) = self.vector_similarity {
            components.push(("vector_similarity", v));
        }
        if let Some(g) = self.graph_distance {
            components.push(("graph_distance", g));
        }
        if let Some(p) = self.path_score {
            components.push(("path_score", p));
        }
        if let Some(m) = self.metadata_boost {
            components.push(("metadata_boost", m));
        }
        if let Some(r) = self.recency_boost {
            components.push(("recency_boost", r));
        }

        components
    }
}

/// Strategy for combining multiple scores (EPIC-049 US-004).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum FusionStrategy {
    /// Reciprocal Rank Fusion (RRF) - good for combining ranked lists.
    #[default]
    Rrf,

    /// Weighted average of scores.
    Weighted,

    /// Take the maximum score.
    Maximum,

    /// Take the minimum score.
    Minimum,

    /// Multiply all scores together.
    Product,

    /// Simple average of all scores.
    Average,
}

impl FusionStrategy {
    /// Combine scores from a breakdown using this strategy.
    #[must_use]
    pub fn combine(&self, breakdown: &ScoreBreakdown) -> f32 {
        let scores: Vec<f32> = [
            breakdown.vector_similarity,
            breakdown.graph_distance,
            breakdown.path_score,
        ]
        .into_iter()
        .flatten()
        .collect();

        if scores.is_empty() {
            return 0.0;
        }

        // Apply multiplicative boosts
        let boost = breakdown
            .metadata_boost
            .unwrap_or(1.0)
            .max(0.0)
            * breakdown.recency_boost.unwrap_or(1.0).max(0.0)
            * breakdown
                .custom_boosts
                .values()
                .fold(1.0, |acc, &b| acc * b.max(0.0));

        let base_score = match self {
            Self::Rrf => {
                // RRF: sum of 1/(k + rank) - here we use score as proxy
                // Higher scores get lower "ranks"
                let k = 60.0_f32;
                scores.iter().map(|&s| 1.0 / (k + (1.0 - s) * 100.0)).sum()
            }
            Self::Weighted => {
                // Equal weights for now - could be configurable
                let weight = 1.0 / scores.len() as f32;
                scores.iter().map(|&s| s * weight).sum()
            }
            Self::Maximum => scores.iter().copied().fold(f32::MIN, f32::max),
            Self::Minimum => scores.iter().copied().fold(f32::MAX, f32::min),
            Self::Product => scores.iter().copied().product(),
            Self::Average => scores.iter().sum::<f32>() / scores.len() as f32,
        };

        base_score * boost
    }

    /// Returns the strategy as a string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Rrf => "rrf",
            Self::Weighted => "weighted",
            Self::Maximum => "maximum",
            Self::Minimum => "minimum",
            Self::Product => "product",
            Self::Average => "average",
        }
    }
}

/// A search result with detailed score breakdown (EPIC-049 US-001).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredResult {
    /// Point/node ID.
    pub id: u64,

    /// Document payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,

    /// Final combined score.
    pub score: f32,

    /// Detailed breakdown of score components.
    pub score_breakdown: ScoreBreakdown,
}

impl ScoredResult {
    /// Creates a new scored result.
    #[must_use]
    pub fn new(id: u64, score: f32) -> Self {
        Self {
            id,
            payload: None,
            score,
            score_breakdown: ScoreBreakdown {
                final_score: score,
                ..Default::default()
            },
        }
    }

    /// Creates a scored result with full breakdown.
    #[must_use]
    pub fn with_breakdown(id: u64, breakdown: ScoreBreakdown) -> Self {
        Self {
            id,
            payload: None,
            score: breakdown.final_score,
            score_breakdown: breakdown,
        }
    }

    /// Builder: set payload.
    #[must_use]
    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_breakdown_new() {
        let breakdown = ScoreBreakdown::new();
        assert!(breakdown.vector_similarity.is_none());
        assert!(breakdown.graph_distance.is_none());
        assert_eq!(breakdown.final_score, 0.0);
    }

    #[test]
    fn test_score_breakdown_from_vector() {
        let breakdown = ScoreBreakdown::from_vector(0.85);
        assert_eq!(breakdown.vector_similarity, Some(0.85));
        assert_eq!(breakdown.final_score, 0.85);
    }

    #[test]
    fn test_score_breakdown_builder() {
        let breakdown = ScoreBreakdown::new()
            .with_vector(0.9)
            .with_graph(0.8)
            .with_metadata_boost(1.2);

        assert_eq!(breakdown.vector_similarity, Some(0.9));
        assert_eq!(breakdown.graph_distance, Some(0.8));
        assert_eq!(breakdown.metadata_boost, Some(1.2));
    }

    #[test]
    fn test_score_breakdown_components() {
        let breakdown = ScoreBreakdown::new()
            .with_vector(0.9)
            .with_graph(0.8);

        let components = breakdown.components();
        assert_eq!(components.len(), 2);
        assert!(components.contains(&("vector_similarity", 0.9)));
        assert!(components.contains(&("graph_distance", 0.8)));
    }

    #[test]
    fn test_fusion_strategy_average() {
        let mut breakdown = ScoreBreakdown::new()
            .with_vector(0.9)
            .with_graph(0.7);

        breakdown.compute_final(&FusionStrategy::Average);
        assert!((breakdown.final_score - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_fusion_strategy_maximum() {
        let mut breakdown = ScoreBreakdown::new()
            .with_vector(0.9)
            .with_graph(0.7);

        breakdown.compute_final(&FusionStrategy::Maximum);
        assert!((breakdown.final_score - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_fusion_strategy_minimum() {
        let mut breakdown = ScoreBreakdown::new()
            .with_vector(0.9)
            .with_graph(0.7);

        breakdown.compute_final(&FusionStrategy::Minimum);
        assert!((breakdown.final_score - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_fusion_strategy_product() {
        let mut breakdown = ScoreBreakdown::new()
            .with_vector(0.9)
            .with_graph(0.8);

        breakdown.compute_final(&FusionStrategy::Product);
        assert!((breakdown.final_score - 0.72).abs() < 0.001);
    }

    #[test]
    fn test_fusion_with_metadata_boost() {
        let mut breakdown = ScoreBreakdown::new()
            .with_vector(0.8)
            .with_metadata_boost(1.5);

        breakdown.compute_final(&FusionStrategy::Average);
        // 0.8 * 1.5 = 1.2
        assert!((breakdown.final_score - 1.2).abs() < 0.001);
    }

    #[test]
    fn test_fusion_with_multiple_boosts() {
        let mut breakdown = ScoreBreakdown::new()
            .with_vector(0.8)
            .with_metadata_boost(1.2)
            .with_recency_boost(1.1);

        breakdown.compute_final(&FusionStrategy::Average);
        // 0.8 * 1.2 * 1.1 = 1.056
        assert!((breakdown.final_score - 1.056).abs() < 0.01);
    }

    #[test]
    fn test_fusion_with_custom_boost() {
        let mut breakdown = ScoreBreakdown::new()
            .with_vector(0.5)
            .with_custom_boost("popularity", 2.0);

        breakdown.compute_final(&FusionStrategy::Average);
        // 0.5 * 2.0 = 1.0
        assert!((breakdown.final_score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_scored_result_new() {
        let result = ScoredResult::new(42, 0.95);
        assert_eq!(result.id, 42);
        assert_eq!(result.score, 0.95);
        assert!(result.payload.is_none());
    }

    #[test]
    fn test_scored_result_with_breakdown() {
        let breakdown = ScoreBreakdown::new()
            .with_vector(0.9)
            .with_graph(0.8);

        let mut bd = breakdown.clone();
        bd.compute_final(&FusionStrategy::Average);

        let result = ScoredResult::with_breakdown(1, bd);
        assert_eq!(result.id, 1);
        assert!(result.score_breakdown.vector_similarity.is_some());
    }

    #[test]
    fn test_fusion_strategy_as_str() {
        assert_eq!(FusionStrategy::Rrf.as_str(), "rrf");
        assert_eq!(FusionStrategy::Weighted.as_str(), "weighted");
        assert_eq!(FusionStrategy::Maximum.as_str(), "maximum");
        assert_eq!(FusionStrategy::Average.as_str(), "average");
    }

    #[test]
    fn test_score_breakdown_json_serialization() {
        let breakdown = ScoreBreakdown::new()
            .with_vector(0.9)
            .with_graph(0.8);

        let json = serde_json::to_string(&breakdown).unwrap();
        assert!(json.contains("vector_similarity"));
        assert!(json.contains("0.9"));
        // Should NOT contain None fields
        assert!(!json.contains("path_score"));
    }

    #[test]
    fn test_scored_result_json_serialization() {
        let mut breakdown = ScoreBreakdown::new().with_vector(0.9);
        breakdown.compute_final(&FusionStrategy::Average);

        let result = ScoredResult::with_breakdown(42, breakdown)
            .with_payload(serde_json::json!({"title": "Test"}));

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("score_breakdown"));
        assert!(json.contains("title"));
        assert!(json.contains("Test"));
    }
}
