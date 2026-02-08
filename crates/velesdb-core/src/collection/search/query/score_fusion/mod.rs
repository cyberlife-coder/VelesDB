//! Multi-Score Fusion for hybrid search results (EPIC-049).
//!
//! This module provides score breakdown and combination strategies
//! for combining vector similarity, graph distance, and metadata boosts.

// SAFETY: Numeric casts in score fusion are intentional:
// - All casts are for score normalization and combination (0-1 range)
// - f64 precision loss acceptable for ranking heuristics
// - usize->i32 for powi() indices: values bounded by path lengths (typically < 100)
// - u64->i64 for timestamps: SystemTime::as_secs() values are within valid range
// - Values are bounded by similarity scores which are naturally limited
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]

mod boost;
mod explanation;
mod path;

pub use boost::{BoostCombination, BoostFunction, CompositeBoost, FieldBoost, RecencyBoost};
pub use explanation::{ComponentExplanation, ScoreExplanation};
pub use path::PathScorer;

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
        let boost = breakdown.metadata_boost.unwrap_or(1.0).max(0.0)
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
                // SAFETY: scores.len() is typically < 100, fits in f32 with full precision
                #[allow(clippy::cast_precision_loss)]
                let weight = 1.0 / scores.len() as f32;
                scores.iter().map(|&s| s * weight).sum()
            }
            Self::Maximum => scores.iter().copied().fold(f32::MIN, f32::max),
            Self::Minimum => scores.iter().copied().fold(f32::MAX, f32::min),
            Self::Product => scores.iter().copied().product(),
            // SAFETY: scores.len() is typically < 100, fits in f32 with full precision
            #[allow(clippy::cast_precision_loss)]
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

// Tests moved to score_fusion_tests.rs per project rules
