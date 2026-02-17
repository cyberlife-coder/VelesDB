//! Path Scorer for graph traversal scoring (EPIC-049 US-002).
//!
//! Scores paths based on distance decay and relationship type weights.

// SAFETY: Numeric casts in path scoring are intentional:
// - usize->i32 for powi() indices: values bounded by path lengths (typically < 100)
#![allow(clippy::cast_possible_wrap)]

use std::collections::HashMap;

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
    /// * `path` - Slice of (`source_id`, `target_id`, `rel_type`) tuples representing edges
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
