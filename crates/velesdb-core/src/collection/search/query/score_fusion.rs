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

// ============================================================================
// Score Explanation API (EPIC-049 US-005)
// ============================================================================

/// Detailed explanation of a score's components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreExplanation {
    /// Final computed score.
    pub final_score: f32,
    /// Name of the fusion strategy used.
    pub strategy: String,
    /// Breakdown of individual score components.
    pub components: Vec<ComponentExplanation>,
    /// Human-readable explanation text.
    pub human_readable: String,
}

/// Explanation of a single score component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentExplanation {
    /// Component name (e.g., "vector_similarity").
    pub name: String,
    /// Raw value of the component.
    pub value: f32,
    /// Weight applied to this component (if weighted strategy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
    /// Contribution to final score.
    pub contribution: f32,
    /// Human-readable description.
    pub description: String,
}

impl ScoreBreakdown {
    /// Generates a detailed explanation of the score breakdown.
    #[must_use]
    pub fn explain(&self, strategy: &FusionStrategy) -> ScoreExplanation {
        let mut components = Vec::new();
        let total_components = self.count_components();
        let default_weight = if total_components > 0 {
            1.0 / total_components as f32
        } else {
            1.0
        };

        if let Some(v) = self.vector_similarity {
            components.push(ComponentExplanation {
                name: "vector_similarity".to_string(),
                value: v,
                weight: Some(default_weight),
                contribution: v * default_weight,
                description: format!("Cosine similarity to query vector: {v:.3}"),
            });
        }

        if let Some(g) = self.graph_distance {
            components.push(ComponentExplanation {
                name: "graph_distance".to_string(),
                value: g,
                weight: Some(default_weight),
                contribution: g * default_weight,
                description: format!("Normalized graph proximity: {g:.3}"),
            });
        }

        if let Some(p) = self.path_score {
            components.push(ComponentExplanation {
                name: "path_score".to_string(),
                value: p,
                weight: Some(default_weight),
                contribution: p * default_weight,
                description: format!("Path relevance (decay + rel types): {p:.3}"),
            });
        }

        if let Some(m) = self.metadata_boost {
            components.push(ComponentExplanation {
                name: "metadata_boost".to_string(),
                value: m,
                weight: None, // Multiplicative, not weighted
                contribution: 0.0,
                description: format!("Metadata multiplier: {m:.2}x"),
            });
        }

        if let Some(r) = self.recency_boost {
            components.push(ComponentExplanation {
                name: "recency_boost".to_string(),
                value: r,
                weight: None,
                contribution: 0.0,
                description: format!("Recency multiplier: {r:.2}x"),
            });
        }

        for (name, &boost) in &self.custom_boosts {
            components.push(ComponentExplanation {
                name: format!("custom:{name}"),
                value: boost,
                weight: None,
                contribution: 0.0,
                description: format!("Custom boost '{name}': {boost:.2}x"),
            });
        }

        let human_readable = Self::generate_human_readable(self.final_score, &components);

        ScoreExplanation {
            final_score: self.final_score,
            strategy: strategy.as_str().to_string(),
            components,
            human_readable,
        }
    }

    fn count_components(&self) -> usize {
        let mut count = 0;
        if self.vector_similarity.is_some() {
            count += 1;
        }
        if self.graph_distance.is_some() {
            count += 1;
        }
        if self.path_score.is_some() {
            count += 1;
        }
        count
    }

    fn generate_human_readable(final_score: f32, components: &[ComponentExplanation]) -> String {
        let mut lines = vec![format!("Final score: {final_score:.3}")];

        for c in components {
            if let Some(w) = c.weight {
                lines.push(format!(
                    "  • {}: {:.3} (weight: {:.0}%)",
                    c.name,
                    c.value,
                    w * 100.0
                ));
            } else {
                lines.push(format!("  • {}: {:.2}x (multiplier)", c.name, c.value));
            }
        }

        lines.join("\n")
    }
}

// ============================================================================
// Metadata Boost Functions (EPIC-049 US-003)
// ============================================================================

/// Trait for boost functions that modify scores based on document metadata.
pub trait BoostFunction: Send + Sync {
    /// Computes a boost multiplier for a document.
    ///
    /// Returns a value where:
    /// - 1.0 = no boost (neutral)
    /// - > 1.0 = positive boost (increases score)
    /// - < 1.0 = negative boost (decreases score)
    fn compute(&self, document: &serde_json::Value) -> f32;

    /// Returns the name of this boost function for debugging.
    fn name(&self) -> &'static str;
}

/// Recency boost: favors recent documents with exponential decay.
#[derive(Debug, Clone)]
pub struct RecencyBoost {
    /// Field containing timestamp (RFC3339 or Unix epoch).
    pub field: String,
    /// Decay half-life in days.
    pub half_life_days: f64,
    /// Maximum boost for brand new documents.
    pub max_boost: f32,
}

impl Default for RecencyBoost {
    fn default() -> Self {
        Self {
            field: "created_at".to_string(),
            half_life_days: 30.0,
            max_boost: 1.5,
        }
    }
}

impl RecencyBoost {
    /// Creates a new recency boost.
    #[must_use]
    pub fn new(field: impl Into<String>, half_life_days: f64, max_boost: f32) -> Self {
        Self {
            field: field.into(),
            half_life_days: half_life_days.max(0.1),
            max_boost: max_boost.max(1.0),
        }
    }
}

impl BoostFunction for RecencyBoost {
    fn compute(&self, document: &serde_json::Value) -> f32 {
        let age_days = self.extract_age_days(document);

        match age_days {
            Some(days) if days >= 0.0 => {
                let decay = 0.5_f64.powf(days / self.half_life_days);
                1.0 + (self.max_boost - 1.0) * decay as f32
            }
            _ => 1.0, // No timestamp or future date -> neutral
        }
    }

    fn name(&self) -> &'static str {
        "recency"
    }
}

impl RecencyBoost {
    fn extract_age_days(&self, document: &serde_json::Value) -> Option<f64> {
        let field_value = document.get(&self.field)?;

        // Get current Unix timestamp
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;

        // Try Unix timestamp (seconds) - most common for APIs
        if let Some(epoch) = field_value.as_i64() {
            return Some((now_secs - epoch) as f64 / 86400.0);
        }

        // Try Unix timestamp as float
        if let Some(epoch) = field_value.as_f64() {
            return Some((now_secs as f64 - epoch) / 86400.0);
        }

        None
    }
}

/// Field boost: boosts based on a numeric metadata field.
#[derive(Debug, Clone)]
pub struct FieldBoost {
    /// Field name containing numeric value.
    pub field: String,
    /// Scale factor (multiplied with field value).
    pub scale: f32,
    /// Minimum boost value (floor).
    pub min_boost: f32,
    /// Maximum boost value (ceiling).
    pub max_boost: f32,
}

impl Default for FieldBoost {
    fn default() -> Self {
        Self {
            field: "importance".to_string(),
            scale: 0.1,
            min_boost: 0.5,
            max_boost: 2.0,
        }
    }
}

impl FieldBoost {
    /// Creates a new field boost.
    #[must_use]
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            ..Default::default()
        }
    }

    /// Builder: set scale factor.
    #[must_use]
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Builder: set min/max bounds.
    #[must_use]
    pub fn with_bounds(mut self, min: f32, max: f32) -> Self {
        self.min_boost = min;
        self.max_boost = max;
        self
    }
}

impl BoostFunction for FieldBoost {
    fn compute(&self, document: &serde_json::Value) -> f32 {
        let value = document
            .get(&self.field)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32;

        let boost = 1.0 + value * self.scale;
        boost.clamp(self.min_boost, self.max_boost)
    }

    fn name(&self) -> &'static str {
        "field"
    }
}

/// Strategy for combining multiple boost functions.
#[derive(Debug, Clone, Copy, Default)]
pub enum BoostCombination {
    /// Multiply all boosts together.
    #[default]
    Multiply,
    /// Add boosts (subtracting n-1 to keep neutral at 1.0).
    Add,
    /// Take maximum boost.
    Max,
    /// Take minimum boost.
    Min,
}

/// Composite boost: combines multiple boost functions.
#[derive(Default)]
pub struct CompositeBoost {
    boosts: Vec<Box<dyn BoostFunction>>,
    combination: BoostCombination,
}

impl CompositeBoost {
    /// Creates a new composite boost.
    #[must_use]
    pub fn new(combination: BoostCombination) -> Self {
        Self {
            boosts: Vec::new(),
            combination,
        }
    }

    /// Adds a boost function to the composite.
    pub fn add(&mut self, boost: impl BoostFunction + 'static) {
        self.boosts.push(Box::new(boost));
    }

    /// Builder: add a boost function.
    #[must_use]
    pub fn with_boost(mut self, boost: impl BoostFunction + 'static) -> Self {
        self.add(boost);
        self
    }
}

impl BoostFunction for CompositeBoost {
    fn compute(&self, document: &serde_json::Value) -> f32 {
        if self.boosts.is_empty() {
            return 1.0;
        }

        let values: Vec<f32> = self.boosts.iter().map(|b| b.compute(document)).collect();

        match self.combination {
            BoostCombination::Multiply => values.iter().product(),
            BoostCombination::Add => {
                // Sum boosts, subtract (n-1) to keep neutral at 1.0
                values.iter().sum::<f32>() - (values.len() as f32 - 1.0)
            }
            BoostCombination::Max => values.iter().copied().fold(1.0_f32, f32::max),
            BoostCombination::Min => values.iter().copied().fold(f32::MAX, f32::min),
        }
    }

    fn name(&self) -> &'static str {
        "composite"
    }
}

// ============================================================================
// Path Scorer (EPIC-049 US-002)
// ============================================================================

/// Path scorer for graph traversal scoring (EPIC-049 US-002).
///
/// Scores paths based on:
/// - Distance decay: shorter paths score higher
/// - Relationship type weights: some relationships are more valuable
#[derive(Debug, Clone)]
pub struct PathScorer {
    /// Decay factor per hop (0-1). Default 0.8 means each hop reduces score by 20%.
    pub distance_decay: f32,
    /// Weights for relationship types (e.g., "AUTHORED" -> 1.0, "MENTIONS" -> 0.5).
    pub rel_type_weights: HashMap<String, f32>,
    /// Default weight for unknown relationship types.
    pub default_weight: f32,
}

impl Default for PathScorer {
    fn default() -> Self {
        Self {
            distance_decay: 0.8,
            rel_type_weights: HashMap::new(),
            default_weight: 1.0,
        }
    }
}

impl PathScorer {
    /// Creates a new path scorer with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: set distance decay factor.
    #[must_use]
    pub fn with_decay(mut self, decay: f32) -> Self {
        self.distance_decay = decay.clamp(0.0, 1.0);
        self
    }

    /// Builder: add a relationship type weight.
    #[must_use]
    pub fn with_rel_weight(mut self, rel_type: impl Into<String>, weight: f32) -> Self {
        self.rel_type_weights.insert(rel_type.into(), weight);
        self
    }

    /// Builder: set default weight for unknown relationship types.
    #[must_use]
    pub fn with_default_weight(mut self, weight: f32) -> Self {
        self.default_weight = weight;
        self
    }

    /// Scores a path based on length and relationship types.
    ///
    /// # Arguments
    /// * `path` - Slice of (source_id, target_id, rel_type) tuples representing edges
    ///
    /// # Returns
    /// Score between 0.0 and 1.0 where:
    /// - 1.0 = direct match (empty path)
    /// - Lower scores for longer paths and weaker relationship types
    #[must_use]
    pub fn score_path(&self, path: &[(u64, u64, &str)]) -> f32 {
        if path.is_empty() {
            return 1.0; // Direct match, no traversal needed
        }

        let mut score = 1.0;

        for (i, (_, _, rel_type)) in path.iter().enumerate() {
            // Distance decay: exponential decay per hop
            let hop_decay = self.distance_decay.powi(i as i32 + 1);

            // Relationship type weight
            let rel_weight = self
                .rel_type_weights
                .get(*rel_type)
                .copied()
                .unwrap_or(self.default_weight);

            score *= hop_decay * rel_weight;
        }

        score.clamp(0.0, 1.0)
    }

    /// Scores a path given only relationship types (simplified API).
    #[must_use]
    pub fn score_rel_types(&self, rel_types: &[&str]) -> f32 {
        if rel_types.is_empty() {
            return 1.0;
        }

        let mut score = 1.0;

        for (i, rel_type) in rel_types.iter().enumerate() {
            let hop_decay = self.distance_decay.powi(i as i32 + 1);
            let rel_weight = self
                .rel_type_weights
                .get(*rel_type)
                .copied()
                .unwrap_or(self.default_weight);
            score *= hop_decay * rel_weight;
        }

        score.clamp(0.0, 1.0)
    }

    /// Scores based on path length only (ignores relationship types).
    #[must_use]
    pub fn score_length(&self, length: usize) -> f32 {
        if length == 0 {
            return 1.0;
        }
        self.distance_decay.powi(length as i32).clamp(0.0, 1.0)
    }
}

// Tests moved to score_fusion_tests.rs per project rules
